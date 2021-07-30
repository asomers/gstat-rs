mod util;

use bitfield::bitfield;
use crate::util::{
    event::{Event, Events},
    iter::IteratorExt
};
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
    str::FromStr,
    time::Duration
};
use structopt::StructOpt;
use termion::{
    event::Key,
    input::MouseTerminal,
    raw::IntoRawMode,
};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Direction, Layout, Rect,},
    style::{Color, Modifier, Style},
    text::Text,
    widgets::{
        Block, Borders, Cell, Clear, List, ListItem, ListState,
        Paragraph, Row, Table, TableState
    },
    Terminal,
};

/// helper function to create a one-line popup box
fn popup_layout(x: u16, y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Max(r.height.saturating_sub(y)/2),
                Constraint::Length(y),
                Constraint::Max(r.height.saturating_sub(y)/2),
            ]
            .as_ref(),
        )
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            [
                Constraint::Max(r.width.saturating_sub(x) / 2),
                Constraint::Length(x),
                Constraint::Max(r.width.saturating_sub(x) / 2),
            ]
            .as_ref(),
        )
        .split(popup_layout[1])[1]
}

/// Drop-in replacement for gstat(8)
// TODO: shorten the help options so they fit on 80 columns.
#[derive(Debug, Default, Deserialize, Serialize, StructOpt)]
struct Cli {
    /// Only display providers that are at least 0.1% busy
    #[structopt(short = "a", long = "auto")]
    auto: bool,
    /// Display statistics for delete (BIO_DELETE) operations.
    #[structopt(short = "d", long = "delete")]
    delete: bool,
    /// Only display devices with names matching filter, as a regex.
    #[structopt(short = "f", long = "filter")]
    filter: Option<String>,
    /// Display statistics for other (BIO_FLUSH) operations.
    #[structopt(short = "o", long = "other")]
    other: bool,
    /// Display block size statistics
    #[structopt(short = "s", long = "size")]
    size: bool,
    /// Only display physical providers (those with rank of 1).
    #[structopt(short = "p", long = "physical")]
    physical: bool,
    /// Reset the config file to defaults
    #[serde(skip)]
    #[structopt(long = "reset-config")]
    reset_config: bool,
    /// Reverse the sort
    #[structopt(short = "r", long = "reverse")]
    reverse: bool,
    /// Sort by the named column.  The name should match the column header.
    #[structopt(short = "S", long = "sort")]
    sort: Option<String>,
    /// Bitfield of columns to enable
    #[serde(default = "default_columns_enabled")]
    columns: Option<ColumnsEnabled>,
    /// Display update interval, in microseconds or with the specified unit
    #[structopt(
        short = "I",
        long = "interval",
        parse(try_from_str = Cli::duration_from_str)
    )]
    interval: Option<Duration>,
}

impl Cli {
    fn duration_from_str(s: &str) -> Result<Duration, humanize_rs::ParseError> {
        if let Ok(us) = s.parse::<u64>() {
            // With no units, default to microseconds
            Ok(Duration::from_micros(us))
        } else {
            humanize_rs::duration::parse(s)
        }
    }
}

impl BitOrAssign for Cli {
    #[allow(clippy::or_fun_call)]
    fn bitor_assign(&mut self, rhs: Self) {
        self.auto |= rhs.auto;
        self.delete |= rhs.delete;
        self.filter = rhs.filter.or(self.filter.take());
        self.other |= rhs.other;
        self.size |= rhs.size;
        self.interval = rhs.interval.or(self.interval.take());
        self.physical |= rhs.physical;
        self.reverse |= rhs.reverse;
        self.sort = rhs.sort.or(self.sort.take());
        self.columns = rhs.columns.or(self.columns.take());
    }
}

struct Column {
    name: &'static str,
    header: &'static str,
    enabled: bool,
    width: Constraint
}

impl Column {
    fn new(
        name: &'static str,
        header: &'static str,
        enabled: bool,
        width: Constraint,
    ) -> Self
    {
        Column {name, header, enabled, width}
    }

    fn min_width(&self) -> u16 {
        match self.width {
            Constraint::Min(x) => x,
            Constraint::Length(x) => x,
            _ => unreachable!("gstat-rs doesn't create columns like this")
        }
    }
}

bitfield!{
    #[derive(Clone, Copy, Deserialize, Serialize)]
    pub struct ColumnsEnabled(u32);
    impl Debug;
    u32; qd, set_qd: 0;
    u32; ops_s, set_ops_s: 1;
    u32; r_s, set_r_s: 2;
    u32; kb_r, set_kb_r: 3;
    u32; kbs_r, set_kbs_r: 4;
    u32; ms_r, set_ms_r: 5;
    u32; w_s, set_w_s: 6;
    u32; kb_w, set_kb_w: 7;
    u32; kbs_w, set_kbs_w: 8;
    u32; ms_w, set_ms_w: 9;
    u32; d_s, set_d_s: 10;
    u32; kb_d, set_kb_d: 11;
    u32; kbs_d, set_kbs_d: 12;
    u32; ms_d, set_ms_d: 13;
    u32; o_s, set_o_s: 14;
    u32; ms_o, set_ms_o: 15;
    u32; pct_busy, set_pct_busy: 16;
    u32; name, set_name: 17;
}

impl Default for ColumnsEnabled {
    fn default() -> Self {
        ColumnsEnabled(Columns::DEFAULT_ENABLED)
    }
}

fn default_columns_enabled() -> Option<ColumnsEnabled> {
    Some(Default::default())
}

// TODO: remove this impl.  It only exists because gumdrop can't skip a field.
impl FromStr for ColumnsEnabled {
    type Err = io::Error;
    fn from_str(_s: &str) -> Result<Self, Self::Err> {
        Ok(Self::default())
    }
}

struct Columns {
    cols: [Column; Columns::LEN],
    state: ListState
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
    const LEN: usize = 18;
    const DEFAULT_ENABLED: u32 = 0x30377;

    fn new(cfg: &mut Cli) -> Self {
        let mut cb = match cfg.columns {
            Some(cb) => cb,
            None => {
                // Can only happen when using --reset-config
                ColumnsEnabled(Self::DEFAULT_ENABLED)
            }
        };
        // Apply the -ods switches, for legacy compatibility
        if cfg.delete {
            cb.set_d_s(true);
            cb.set_kbs_d(true);
            cb.set_ms_d(true);
        }
        if cfg.other {
            cb.set_o_s(true);
            cb.set_ms_o(true);
        }
        if cfg.delete && cfg.size {
            cb.set_kb_d(true);
        }
        if cfg.size {
            cb.set_kb_r(true);
            cb.set_kb_w(true);
        }
        // Write back any changes we made.
        cfg.columns = Some(cb);
        let cols = [
            Column::new("Queue depth", "L(q)", cb.qd(),
                        Constraint::Length(5)),
            Column::new("IOPs", " ops/s", cb.ops_s(),
                        Constraint::Length(7)),
            Column::new("Read IOPs", "   r/s", cb.r_s(),
                        Constraint::Length(7)),
            Column::new("Read size", "kB/r", cb.kb_r(),
                        Constraint::Length(5)),
            Column::new("Read throughput", "kB/s r", cb.kbs_r(),
                        Constraint::Length(7)),
            Column::new("Read latency", "  ms/r", cb.ms_r(),
                        Constraint::Length(7)),
            Column::new("Write IOPs", "   w/s", cb.w_s(),
                        Constraint::Length(7)),
            Column::new("Write size", "kB/w", cb.kb_w(),
                        Constraint::Length(5)),
            Column::new("Write throughput", "kB/s w", cb.kbs_w(),
                        Constraint::Length(7)),
            Column::new("Write latency", "  ms/w", cb.ms_w(),
                        Constraint::Length(7)),
            Column::new("Delete IOPs", "   d/s", cb.d_s(),
                        Constraint::Length(7)),
            Column::new("Delete size", "kB/d", cb.kb_d(),
                        Constraint::Length(5)),
            Column::new("Delete throughput", "kB/s d", cb.kbs_d(),
                        Constraint::Length(7)),
            Column::new("Delete latency", "  ms/d", cb.ms_d(),
                        Constraint::Length(7)),
            Column::new("Other IOPs", "   o/s", cb.o_s(),
                        Constraint::Length(7)),
            Column::new("Other latency", "  ms/o", cb.ms_o(),
                        Constraint::Length(7)),
            Column::new("Percent busy", " %busy", cb.pct_busy(),
                        Constraint::Length(7)),
            Column::new("Name", "Name", cb.name(),
                        Constraint::Min(10)),
        ];
        let mut state = ListState::default();
        state.select(Some(0));
        Columns {cols, state}
    }

    // This value is "defined" by the unit test of the same name.
    pub const fn max_name_width(&self) -> u16 {
        17
    }

    pub fn next(&mut self) {
        let s = Some((self.state.selected().unwrap() + 1) % self.cols.len());
        self.state.select(s);
    }

    pub fn previous(&mut self) {
        let old = self.state.selected().unwrap() as isize;
        let s = Some((old - 1).rem_euclid(self.cols.len() as isize) as usize);
        self.state.select(s);
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
        let mut cells = Vec::with_capacity(Columns::LEN);
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
        let mut ds = DataSource {
            prev,
            cur,
            tree,
            items
        };
        ds.regen()?;
        Ok(ds)
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
                    let name = gident.name().unwrap().to_string_lossy();
                    let elem = Element::new(&name, rank, &stats);
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
        let s = match self.state.selected() {
            Some(i) => {
                if i >= self.len.saturating_sub(1) {
                    None
                } else {
                    Some(i + 1)
                }
            }
            None => if self.len > 0 {
                Some(0)
            } else {
                None
            }
        };
        self.state.select(s);
    }

    pub fn previous(&mut self) {
        let s = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    None
                } else {
                    Some(i - 1)
                }
            }
            None => self.len.checked_sub(1),
        };
        self.state.select(s);
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

// https://github.com/rust-lang/rust-clippy/issues/7483
#[allow(clippy::or_fun_call)]
fn main() -> Result<(), Box<dyn Error>> {
    let cli: Cli = Cli::from_args();
    let mut cfg = if cli.reset_config {
        cli
    } else {
        let mut cfg: Cli = confy::load("gstat-rs")?;
        cfg |= cli;
        cfg
    };
    let mut filter = cfg.filter.as_ref().map(|s| Regex::new(s).unwrap());
    let mut tick_rate = cfg.interval.unwrap_or(Duration::from_secs(1));
    let mut editting_regex = false;
    let mut new_regex = String::new();
    let mut paused = false;
    let mut selecting_columns = false;

    let mut columns = Columns::new(&mut cfg);

    let mut sort_idx: Option<usize> = cfg.sort.as_ref()
        .map(|name| columns.cols.iter()
             .enumerate()
             .find(|(_i, col)| col.header.trim() == name.trim())
             .map(|(i, _col)| i)
        ).flatten();

    // Terminal initialization
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let stdin = io::stdin();
    let mut events = Events::new(stdin);

    let mut data = DataSource::new()?;
    let mut table = StatefulTable::default();
    data.sort(sort_idx, cfg.reverse);

    let normal_style = Style::default().bg(Color::Blue);

    terminal.clear()?;
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
            let max_name_width = data.items.iter()
                .filter(|elem| !cfg.auto || elem.pct_busy > 0.1)
                .filter(|elem| !cfg.physical || elem.rank == 1)
                .filter(|elem|
                        filter.as_ref()
                        .map(|f| f.is_match(&elem.name))
                        .unwrap_or(true)
                ).map(|elem| elem.name.len() as u16)
                .max()
                .unwrap_or(0);
            let twidth: u16 = columns.cols.iter()
                .filter(|col| col.enabled)
                .map(|col| {
                    if col.name == "Name" {
                        max_name_width
                    } else {
                        col.min_width()
                    }
                }).sum();
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
                ).map(|elem| elem.row(&columns))
                .deinterleave::<Vec<_>>(ntables.into());
            for (i, rows) in multirows.into_iter().enumerate() {
                let t = table.table(header.clone(), rows, &widths);
                f.render_stateful_widget(t, rects[i], &mut table.state);
            }

            if editting_regex {
                let area = popup_layout(40, 3, f.size());
                let popup_box = Paragraph::new(new_regex.as_ref())
                    .block(
                        Block::default()
                        .borders(Borders::ALL)
                        .title("Filter regex")
                    );
                f.render_widget(Clear, area);
                f.render_widget(popup_box, area);
            } else if selecting_columns {
                let boxwidth = columns.max_name_width() + 6;
                let area = popup_layout(boxwidth, 20, f.size());
                f.render_widget(Clear, area);
                let items = columns.cols.iter()
                    .map(|c| {
                        let text = if c.enabled {
                            format!("[x] {}", c.name)
                        } else {
                            format!("[ ] {}", c.name)
                        };
                        ListItem::new(Text::from(text))
                    })
                    .collect::<Vec<_>>();

                let list = List::new(items)
                    .block(
                        Block::default()
                        .borders(Borders::ALL)
                        .title("Select columns")
                    ).highlight_style(
                        Style::default()
                        .add_modifier(Modifier::REVERSED)
                    );
                f.render_stateful_widget(list, area, &mut columns.state);
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
                } else if selecting_columns {
                    match key {
                        Key::Char(' ') => {
                            if let Some(i) = columns.state.selected() {
                                // unwrapping is safe; the default value should
                                // always be set by this point.
                                cfg.columns.as_mut().unwrap().0 ^= 1 << i;
                                columns.cols[i].enabled ^= true;
                            }
                        }
                        Key::Char('q') => {
                            break;
                        }
                        Key::Down => {
                            columns.next();
                        }
                        Key::Up => {
                            columns.previous();
                        }
                        Key::Esc => {
                            selecting_columns = false;
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
                            cfg.interval = Some(tick_rate);
                        }
                        Key::Char('>') => {
                            tick_rate *= 2;
                            cfg.interval = Some(tick_rate);
                        }
                        Key::Char('F') => {
                            cfg.filter = None;
                            filter = None;
                        }
                        Key::Char('a') => {
                            cfg.auto ^= true;
                        }
                        Key::Char('f') => {
                            editting_regex = true;
                            new_regex = String::new();
                        }
                        Key::Char('p') => {
                            cfg.physical ^= true;
                        }
                        Key::Char('q') => {
                            break;
                        }
                        Key::Char('r') => {
                            cfg.reverse ^= true;
                            data.sort(sort_idx, cfg.reverse);
                        }
                        Key::Down => {
                            table.next();
                        }
                        Key::Up => {
                            table.previous();
                        }
                        Key::Delete => {
                            if let Some(i) = sort_idx {
                                cfg.columns.as_mut().unwrap().0 ^= 1 << i;
                                columns.cols[i].enabled ^= true;
                            }
                        }
                        Key::Insert => {
                            selecting_columns = true;
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
    if let Err(e) = confy::store("gstat-rs", &cfg) {
        eprintln!("Warning: failed to save config file: {}", e);
    }
    terminal.set_cursor(0, terminal.size()?.height - 1)?;

    Ok(())
}

#[cfg(test)]
mod t {
    use super::*;

    mod columns {
        use super::*;

        #[test]
        fn max_name_width() {
            let mut cfg = Cli::default();
            let columns = Columns::new(&mut cfg);
            let expected = columns.cols.iter()
                .map(|col| col.name.len())
                .max()
                .unwrap();
            assert_eq!(expected, usize::from(columns.max_name_width()));
        }

        /// Unlike TableState, it makes no sense for the ColumnSelector to have
        /// no row selected.  So wrap from end to beginning, skipping None.
        #[test]
        fn next() {
            let mut cfg = Cli::default();
            let mut columns = Columns::new(&mut cfg);
            assert_eq!(columns.state.selected(), Some(0));
            for _ in 0..(columns.cols.len() - 1) { columns.next(); }
            assert_eq!(columns.state.selected(), Some(columns.cols.len() - 1));
            columns.next();
            assert_eq!(columns.state.selected(), Some(0));
        }

        #[test]
        fn previous() {
            let mut cfg = Cli::default();
            let mut columns = Columns::new(&mut cfg);
            assert_eq!(columns.state.selected(), Some(0));
            columns.previous();
            assert_eq!(columns.state.selected(), Some(columns.cols.len() - 1));
            for _ in 0..(columns.cols.len() - 1) { columns.previous(); }
            assert_eq!(columns.state.selected(), Some(0));
        }
    }

    mod stateful_table {
        use super::*;

        #[test]
        fn next_empty() {
            let mut t = StatefulTable::default();
            assert_eq!(t.len, 0);
            assert!(t.state.selected().is_none());
            t.next();
            assert!(t.state.selected().is_none());
        }

        #[test]
        #[allow(clippy::field_reassign_with_default)]
        fn next_nonempty() {
            let mut t = StatefulTable::default();
            t.len = 2;
            assert!(t.state.selected().is_none());
            t.next();
            assert_eq!(t.state.selected(), Some(0));
            t.next();
            assert_eq!(t.state.selected(), Some(1));
            t.next();
            assert_eq!(t.state.selected(), None);
        }

        #[test]
        fn previous_empty() {
            let mut t = StatefulTable::default();
            assert_eq!(t.len, 0);
            assert!(t.state.selected().is_none());
            t.previous();
            assert!(t.state.selected().is_none());
        }

        #[test]
        #[allow(clippy::field_reassign_with_default)]
        fn previous_nonempty() {
            let mut t = StatefulTable::default();
            t.len = 2;
            assert!(t.state.selected().is_none());
            t.previous();
            assert_eq!(t.state.selected(), Some(1));
            t.previous();
            assert_eq!(t.state.selected(), Some(0));
            t.previous();
            assert_eq!(t.state.selected(), None);
        }

    }
}
