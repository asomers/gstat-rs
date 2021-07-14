mod util;

use crate::util::{
    event::{Event, Events},
    iter::IteratorExt
};
use gumdrop::Options;
use freebsd_libgeom::{Snapshot, Statistics, Tree};
use nix::time::{ClockId, clock_gettime};
use regex::Regex;
use serde_derive::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    error::Error,
    io,
    mem,
    num::NonZeroU16,
    ops::BitOrAssign,
    time::Duration
};
use termion::{
    event::Key,
    input::MouseTerminal,
    raw::IntoRawMode,
    screen::AlternateScreen
};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Direction, Layout, Rect,},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState},
    Terminal,
};

/// helper function to create a one-line popup box as a fraction of r's width
fn popup_layout(percent_x: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Max((r.height - 3)/2),
                Constraint::Length(3),
                Constraint::Max((r.height - 3)/2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Percentage((100 - percent_x) / 2),
                Constraint::Percentage(percent_x),
                Constraint::Percentage((100 - percent_x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}


/// Drop-in compatible gstat(8) replacement
// TODO: shorten the help options so they fit on 80 columns.
#[derive(Debug, Default, Deserialize, Options, Serialize)]
struct Cli {
    #[options(help = "print help message")]
    help: bool,
    /// only display providers that are at least 0.1% busy
    #[options(short = 'a')]
    auto: bool,
    /// display statistics for delete (BIO_DELETE) operations.
    #[options(short = 'd')]
    delete: bool,
    /// only display devices with names matching filter, as a regex.
    #[options(short = 'f')]
    filter: Option<String>,
    /// display statistics for other (BIO_FLUSH) operations.
    #[options(short = 'o')]
    other: bool,
    /// display block size statistics
    #[options(short = 's')]
    size: bool,
    /// display update interval, in microseconds or with the specified unit
    #[options(short = 'I')]
    // Note: argh has a "from_str_fn" property that could be used to create a
    // custom parser, to parse interval directly to an int or a Duration.  That
    // would make it easier to save the config file.  But gumpdrop doesn't have
    // that option.
    interval: Option<String>,
    /// only display physical providers (those with rank of 1).
    #[options(short = 'p')]
    physical: bool,
    /// Reset the config file
    #[serde(skip)]
    reset_config: bool,
    /// Reverse the sort
    #[options(short = 'r')]
    reverse: bool,
    /// Sort by the named column.  The name should match the column header.
    sort: Option<String>
}

impl BitOrAssign for Cli {
    #[allow(clippy::or_fun_call)]
    fn bitor_assign(&mut self, rhs: Self) {
        self.help |= rhs.help;
        self.auto |= rhs.auto;
        self.delete |= rhs.delete;
        self.filter = rhs.filter.or(self.filter.take());
        self.other |= rhs.other;
        self.size |= rhs.size;
        self.interval = rhs.interval.or(self.interval.take());
        self.physical |= rhs.physical;
        self.reverse |= rhs.reverse;
        self.sort = rhs.sort.or(self.sort.take());
    }
}

struct Column {
    header: &'static str,
    enabled: bool,
    width: Constraint
}

impl Column {
    fn new(
        header: &'static str,
        enabled: bool,
        width: Constraint,
    ) -> Self
    {
        Column {header, enabled, width}
    }

    fn min_width(&self) -> u16 {
        match self.width {
            Constraint::Min(x) => x,
            Constraint::Length(x) => x,
            _ => unreachable!("gstat-rs doesn't create columns like this")
        }
    }
}

struct Columns {
    cols: [Column; Columns::MAX]
}

impl Columns {
    const QD: usize = 0;
    const OPS_S: usize = 1;
    const R_S: usize = 2;
    const KB_R: usize = 3;
    const KBS_R: usize = 4;
    const MS_R: usize = 5;
    const W_S: usize = 6;
    const KB_W: usize = 7;
    const KBS_W: usize = 8;
    const MS_W: usize = 9;
    const D_S: usize = 10;
    const KB_D: usize = 11;
    const KBS_D: usize = 12;
    const MS_D: usize = 13;
    const O_S: usize = 14;
    const MS_O: usize = 15;
    const PCT_BUSY: usize = 16;
    const NAME: usize = 17;
    const MAX: usize = 18;

    fn new(cfg: &Cli) -> Self {
        let cols = [
            Column::new("L(q)", true, Constraint::Length(5)),
            Column::new(" ops/s", true, Constraint::Length(7)),
            Column::new("   r/s", true, Constraint::Length(7)),
            Column::new("kB/r", cfg.size, Constraint::Length(5)),
            Column::new("kB/s r", true, Constraint::Length(7)),
            Column::new("  ms/r", true, Constraint::Length(7)),
            Column::new("   w/s", true, Constraint::Length(7)),
            Column::new("kB/w", cfg.size, Constraint::Length(5)),
            Column::new("kB/s w", true, Constraint::Length(7)),
            Column::new("  ms/w", true, Constraint::Length(7)),
            Column::new("   d/s", cfg.delete, Constraint::Length(7)),
            Column::new("kB/d", cfg.size && cfg.delete, Constraint::Length(5)),
            Column::new("kB/s d", cfg.delete, Constraint::Length(7)),
            Column::new("  ms/d", cfg.delete, Constraint::Length(7)),
            Column::new("   o/s", cfg.other, Constraint::Length(7)),
            Column::new("  ms/o", cfg.other, Constraint::Length(7)),
            Column::new(" %busy", true, Constraint::Length(7)),
            Column::new("Name", true, Constraint::Min(10)),
        ];
        Columns {cols}
    }
}

/// The data for one element in the table, usually a Geom provider
#[derive(Clone, Debug)]
struct Element{
    qd: u32,
    ops_s: f64,
    r_s: f64,
    kb_r: f64,
    kbs_r: f64,
    ms_r: f64,
    w_s: f64,
    kb_w: f64,
    kbs_w: f64,
    ms_w: f64,
    d_s: f64,
    kb_d: f64,
    kbs_d: f64,
    ms_d: f64,
    o_s: f64,
    ms_o: f64,
    pct_busy: f64,
    name: String,
    rank: u32
}

impl Element {
    fn new(name: &str, rank: u32, stats: &Statistics) -> Self {
        Element {
            qd: stats.queue_length(),
            ops_s: stats.transfers_per_second(),
            r_s: stats.transfers_per_second_read(),
            kb_r: stats.kb_per_transfer_read(),
            kbs_r: stats.mb_per_second_read() * 1024.0,
            ms_r: stats.ms_per_transaction_read(),
            w_s: stats.transfers_per_second_write(),
            kb_w: stats.kb_per_transfer_write(),
            kbs_w: stats.mb_per_second_write() * 1024.0,
            ms_w: stats.ms_per_transaction_write(),
            d_s: stats.transfers_per_second_free(),
            kb_d: stats.kb_per_transfer_free(),
            kbs_d: stats.mb_per_second_free() * 1024.0,
            ms_d: stats.ms_per_transaction_free(),
            o_s: stats.transfers_per_second_other(),
            ms_o: stats.ms_per_transaction_other(),
            pct_busy: stats.busy_pct(),
            name: name.to_owned(),
            //fields: f,
            rank
        }
    }

    /// Like [`std::cmp::PartialOrd::partial_cmp`], but based on the selected
    /// field.
    fn partial_cmp_by(&self, k: usize, other: &Self) -> Option<Ordering> {
        match k {
            Columns::QD => self.qd.partial_cmp(&other.qd),
            Columns::OPS_S => self.ops_s.partial_cmp(&other.ops_s),
            Columns::R_S => self.r_s.partial_cmp(&other.r_s),
            Columns::KB_R => self.kb_r.partial_cmp(&other.kb_r),
            Columns::KBS_R => self.kbs_r.partial_cmp(&other.kbs_r),
            Columns::MS_R => self.ms_r.partial_cmp(&other.ms_r),
            Columns::W_S => self.w_s.partial_cmp(&other.w_s),
            Columns::KB_W => self.kb_w.partial_cmp(&other.kb_w),
            Columns::KBS_W => self.kbs_w.partial_cmp(&other.kbs_w),
            Columns::MS_W => self.ms_w.partial_cmp(&other.ms_w),
            Columns::D_S => self.d_s.partial_cmp(&other.d_s),
            Columns::KB_D => self.kb_d.partial_cmp(&other.kb_d),
            Columns::KBS_D => self.kbs_d.partial_cmp(&other.kbs_d),
            Columns::MS_D => self.ms_d.partial_cmp(&other.ms_d),
            Columns::O_S => self.o_s.partial_cmp(&other.o_s),
            Columns::MS_O => self.ms_o.partial_cmp(&other.ms_o),
            Columns::PCT_BUSY => self.pct_busy.partial_cmp(&other.pct_busy),
            Columns::NAME => self.name.partial_cmp(&other.name),
            _ => None
        }
    }

    fn row(&self, columns: &Columns) -> Row {
        let mut cells = Vec::with_capacity(Columns::MAX);
        if columns.cols[Columns::QD].enabled {
            cells.push(Cell::from(format!("{:>4}", self.qd)));
        }
        if columns.cols[Columns::OPS_S].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.ops_s)));
        }
        if columns.cols[Columns::R_S].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.r_s)));
        }
        if columns.cols[Columns::KB_R].enabled {
            cells.push(Cell::from(format!("{:>4.0}", self.kb_r)));
        }
        if columns.cols[Columns::KBS_R].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.kbs_r)));
        }
        if columns.cols[Columns::MS_R].enabled {
            cells.push(Cell::from(format!("{:>6.1}", self.ms_r)));
        }
        if columns.cols[Columns::W_S].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.w_s)));
        }
        if columns.cols[Columns::KB_W].enabled {
            cells.push(Cell::from(format!("{:>4.0}", self.kb_w)));
        }
        if columns.cols[Columns::KBS_W].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.kbs_w)));
        }
        if columns.cols[Columns::MS_W].enabled {
            cells.push(Cell::from(format!("{:>6.1}", self.ms_w)));
        }
        if columns.cols[Columns::D_S].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.d_s)));
        }
        if columns.cols[Columns::KB_D].enabled {
            cells.push(Cell::from(format!("{:>4.0}", self.kb_d)));
        }
        if columns.cols[Columns::KBS_D].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.kbs_d)));
        }
        if columns.cols[Columns::MS_D].enabled {
            cells.push(Cell::from(format!("{:>6.1}", self.ms_d)));
        }
        if columns.cols[Columns::O_S].enabled {
            cells.push(Cell::from(format!("{:>6.0}", self.o_s)));
        }
        if columns.cols[Columns::MS_O].enabled {
            cells.push(Cell::from(format!("{:>6.1}", self.ms_o)));
        }
        if columns.cols[Columns::PCT_BUSY].enabled {
            const BUSY_HIGH_THRESH: f64 = 80.0;
            const BUSY_MEDIUM_THRESH: f64 = 50.0;

            let color = if self.pct_busy > BUSY_HIGH_THRESH {
                Color::Red
            } else if self.pct_busy > BUSY_MEDIUM_THRESH {
                Color::Magenta
            } else {
                Color::Green
            };
            let style = Style::default().fg(color);
            let s = format!("{:>6.1}", self.pct_busy);
            let cell = Cell::from(s).style(style);
            cells.push(cell);
        }
        if columns.cols[Columns::NAME].enabled {
            cells.push(Cell::from(self.name.clone()));
        }
        Row::new(cells)
    }
}

struct DataSource {
    prev: Option<Snapshot>,
    cur: Snapshot,
    tree: Tree,
    items: Vec<Element>
}

impl DataSource {
    fn new() -> io::Result<DataSource> {
        let tree = Tree::new()?;
        let prev = None;
        // XXX difference from gstat: the first display will show stats since
        // boot, like iostat.
        let cur = Snapshot::new()?;
        let items = Default::default();
        Ok(
            DataSource {
                prev,
                cur,
                tree,
                items
            }
        )
    }

    pub fn refresh(&mut self) -> io::Result<()>
    {
        self.prev = Some(mem::replace(&mut self.cur, Snapshot::new()?));
        self.regen()?;
        Ok(())
    }

    /// Regenerate the data from geom
    fn regen(&mut self) -> io::Result<()>
    {
        let etime = if let Some(prev) = self.prev.as_mut() {
            f64::from(self.cur.timestamp() - prev.timestamp())
        } else {
            let boottime = clock_gettime(ClockId::CLOCK_UPTIME)?;
            boottime.tv_sec() as f64 + boottime.tv_nsec() as f64 * 1e-9
        };
        self.items.clear();
        for (curstat, prevstat) in self.cur.iter_pair(self.prev.as_mut()) {
            if let Some(gident) = self.tree.lookup(curstat.id()) {
                if let Some(rank) = gident.rank() {
                    let stats = Statistics::compute(curstat, prevstat, etime);
                    let elem = Element::new(&gident.name().to_string_lossy(),
                        rank, &stats);
                    self.items.push(elem);
                }
            }
        }
        Ok(())
    }

    fn sort(&mut self, sort_idx: Option<usize>, reverse: bool) {
        if let Some(k) = sort_idx {
            self.items.sort_by(|l, r| {
                if reverse {
                    r.partial_cmp_by(k, l)
                } else {
                    l.partial_cmp_by(k, r)
                }.unwrap()
            });
        }
    }
}

#[derive(Default)]
pub struct StatefulTable {
    state: TableState,
    len: usize
}

impl StatefulTable {
    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.len - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.len - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn table<'a>(&mut self, header: Row<'a>, rows: Vec<Row<'a>>, widths: &'a[Constraint])
        -> Table<'a>
    {
        let selected_style = Style::default().add_modifier(Modifier::REVERSED);
        self.len = rows.len();
        Table::new(rows)
            .header(header)
            .block(Block::default())
            .highlight_style(selected_style)
            .widths(widths)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli: Cli = Cli::parse_args_default_or_exit();
    let mut cfg: Cli = confy::load("gstat-rs")?;
    if cli.reset_config {
        cfg = cli;
    } else {
        cfg |= cli;
    }
    let mut filter = cfg.filter.as_ref().map(|s| Regex::new(s).unwrap());
    let mut tick_rate: Duration = match cfg.interval.as_mut() {
        None => Duration::from_secs(1),
        Some(s) => {
            if s.parse::<i32>().is_ok() {
                // Add the default units
                s.push_str("us");
            }
            humanize_rs::duration::parse(s)?
        }
    };
    let mut editting_regex = false;
    let mut new_regex = String::new();
    let mut paused = false;

    let mut columns = Columns::new(&cfg);

    let mut sort_idx: Option<usize> = cfg.sort.as_ref()
        .map(|name| columns.cols.iter()
             .enumerate()
             .find(|(_i, col)| col.header.trim() == name.trim())
             .map(|(i, _col)| i)
        ).flatten();

    // Terminal initialization
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let stdin = io::stdin();
    let mut events = Events::new(stdin);

    let mut data = DataSource::new()?;
    let mut table = StatefulTable::default();
    data.sort(sort_idx, cfg.reverse);

    let normal_style = Style::default().bg(Color::Blue);

    loop {
        terminal.draw(|f| {
            let header_cells = columns.cols.iter()
                .enumerate()
                .filter(|(_i, col)| col.enabled)
                .map(|(i, col)| {
                    let style = Style::default().fg(Color::Red);
                    let style = if sort_idx == Some(i) {
                        style.add_modifier(Modifier::REVERSED)
                    } else {
                        style
                    };
                    Cell::from(col.header)
                    .style(style)
                });
            let header = Row::new(header_cells)
                .style(normal_style);
            let widths = columns.cols.iter()
                .filter(|col| col.enabled)
                .map(|col| col.width)
                .collect::<Vec<_>>();
            let twidth: u16 = columns.cols.iter()
                .filter(|col| col.enabled)
                .map(|col| col.min_width())
                .sum();
            let ntables = NonZeroU16::new(f.size().width / twidth)
                .unwrap_or_else(|| NonZeroU16::new(1).unwrap());
            let rects = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(
                    (0..ntables.into())
                    .map(|_| Constraint::Percentage(100 / u16::from(ntables)))
                    .collect::<Vec<_>>()
                ).split(f.size());
            let multirows = data.items.iter()
                .filter(|elem| !cfg.auto || elem.pct_busy > 0.1)
                .filter(|elem| !cfg.physical || elem.rank == 1)
                .filter(|elem|
                        filter.as_ref()
                        .map(|f| f.is_match(&elem.name))
                        .unwrap_or(true)
                ).map(|elem| {
                    elem.row(&columns)
                }).deinterleave::<Vec<_>>(ntables.into());
            for (i, rows) in multirows.into_iter().enumerate() {
                let t = table.table(header.clone(), rows, &widths);
                f.render_stateful_widget(t, rects[i], &mut table.state);
            }

            if editting_regex {
                let area = popup_layout(60, f.size());
                let popup_box = Paragraph::new(new_regex.as_ref())
                    .block(
                        Block::default()
                        .borders(Borders::ALL)
                        .title("Filter regex")
                    );
                f.render_widget(Clear, area);
                f.render_widget(popup_box, area);
            }
        }).unwrap();

        match events.poll(&tick_rate) {
            Some(Event::Tick) => {
                if !paused {
                    data.refresh()?;
                    data.sort(sort_idx, cfg.reverse);
                }
            }
            Some(Event::Key(key)) => {
                if editting_regex {
                    match key {
                        Key::Char('\n') => {
                            editting_regex = false;
                            filter = Some(Regex::new(&new_regex)?);
                            cfg.filter = Some(new_regex.split_off(0));
                        }
                        Key::Char(c) => {
                            new_regex.push(c);
                        }
                        Key::Backspace => {
                            new_regex.pop();
                        }
                        Key::Esc => {
                            editting_regex = false;
                        }
                        _ => {}
                    }
                } else {
                    match key {
                        Key::Char(' ') => {
                            paused ^= true;
                            if !paused {
                                // Refresh immediately after unpause.
                                data.refresh()?;
                                data.sort(sort_idx, cfg.reverse);
                            }
                        }
                        Key::Char('+') => {
                            // Ideally this would be 'o' to match top's
                            // behavior.  But 'o' is already taken in gstat.
                            loop {
                                match sort_idx {
                                    Some(idx) => {sort_idx = Some(idx + 1);}
                                    None => {sort_idx = Some(0);}
                                }
                                let idx = sort_idx.unwrap();
                                if idx >= columns.cols.len() {
                                    sort_idx = None;
                                    break;
                                }
                                if columns.cols[idx].enabled {
                                    sort_idx = Some(idx);
                                    break;
                                }
                            }
                            let sort_key = sort_idx
                                .map(|idx| columns.cols[idx].header);
                            cfg.sort = sort_key.map(str::to_owned);
                            data.sort(sort_idx, cfg.reverse);
                        }
                        Key::Char('-') => {
                            // Ideally this would be 'O' to mimic top's
                            // behavior.  But 'o' is already taken in gstat.
                            loop {
                                match sort_idx {
                                    Some(idx) => {
                                        sort_idx = idx.checked_sub(1);
                                    }
                                    None => {
                                        sort_idx = Some(columns.cols.len() - 1);
                                    }
                                }
                                if sort_idx.is_none() {
                                    break;
                                }
                                if columns.cols[sort_idx.unwrap()].enabled {
                                    break;
                                }
                            }
                            let sort_key = sort_idx
                                .map(|idx| columns.cols[idx].header);
                            cfg.sort = sort_key.map(str::to_owned);
                            data.sort(sort_idx, cfg.reverse);
                        }
                        Key::Char('<') => {
                            tick_rate /= 2;
                            let s = tick_rate.as_micros().to_string();
                            cfg.interval = Some(s);
                        }
                        Key::Char('>') => {
                            tick_rate *= 2;
                            let s = tick_rate.as_micros().to_string();
                            cfg.interval = Some(s);
                        }
                        Key::Char('F') => {
                            cfg.filter = None;
                            filter = None;
                        }
                        Key::Char('a') => {
                            cfg.auto ^= true;
                        }
                        Key::Char('d') => {
                            cfg.delete ^= true;
                            columns.cols[Columns::D_S].enabled = cfg.delete;
                            columns.cols[Columns::KB_D].enabled = cfg.delete && cfg.size;
                            columns.cols[Columns::KBS_D].enabled = cfg.delete;
                            columns.cols[Columns::MS_D].enabled = cfg.delete;
                        }
                        Key::Char('f') => {
                            editting_regex = true;
                            new_regex = String::new();
                        }
                        Key::Char('o') => {
                            cfg.other ^= true;
                            columns.cols[Columns::O_S].enabled = cfg.other;
                            columns.cols[Columns::MS_O].enabled = cfg.other;
                        }
                        Key::Char('p') => {
                            cfg.physical ^= true;
                        }
                        Key::Char('q') => {
                            if let Err(e) = confy::store("gstat-rs", &cfg) {
                                eprintln!("Warning: failed to save config file: {}", e);
                            }
                            break;
                        }
                        Key::Char('r') => {
                            cfg.reverse ^= true;
                            data.sort(sort_idx, cfg.reverse);
                        }
                        Key::Char('s') => {
                            cfg.size ^= true;
                            columns.cols[Columns::KB_R].enabled = cfg.size;
                            columns.cols[Columns::KB_W].enabled = cfg.size;
                            columns.cols[Columns::KB_D].enabled = cfg.delete && cfg.size;
                        }
                        Key::Down => {
                            table.next();
                        }
                        Key::Up => {
                            table.previous();
                        }
                        _ => {}
                    }
                }
            },
            Some(Event::Mouse(_mev)) => {
                // ignore for now
            }
            None => {
                // stdin closed for some reason
                break;
            },
        };
    }

    Ok(())
}
