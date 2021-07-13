mod util;

use crate::util::event::{Event, Events};
use gumdrop::Options;
use freebsd_libgeom::{Snapshot, Statistics, Tree};
use nix::time::{ClockId, clock_gettime};
use regex::Regex;
use std::{
    collections::hash_map::HashMap,
    error::Error,
    io,
    mem,
    ops::Index,
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
#[derive(Debug, Options)]
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
    interval: Option<String>,
    /// only display physical providers (those with rank of 1).
    #[options(short = 'p')]
    physical: bool,
    /// Reverse the sort
    #[options(short = 'r')]
    reverse: bool,
    /// Sort by the named column.  The name should match the column header.
    sort: Option<String>
}

struct Column {
    header: &'static str,
    enabled: bool,
    format: fn(&Field) -> String,
    width: Constraint
}

impl Column {
    fn new(
        header: &'static str,
        enabled: bool,
        width: Constraint,
        format: fn(&Field) -> String
    ) -> Self
    {
        Column {header, enabled, format, width}
    }
}

/// The value of one metric of one geom
#[derive(Clone, Debug, PartialEq, PartialOrd)]
enum Field {
    Int(u32),
    Float(f64),
    Str(String)
}

impl Field {
    fn as_int(&self) -> u32 {
        match self {
            Field::Int(x) => *x,
            _ => panic!("not an int")
        }
    }

    fn as_float(&self) -> f64 {
        match self {
            Field::Float(x) => *x,
            _ => panic!("not an float")
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Field::Str(x) => x,
            _ => panic!("not a string")
        }
    }
}

/// The data for one element in the table, usually a Geom provider
#[derive(Clone, Debug, Default)]
struct Element{
    fields: HashMap<&'static str, Field>,
    rank: u32
}

impl Element {
    fn insert(&mut self, k: &'static str, v: Field) -> Option<Field> {
        self.fields.insert(k, v)
    }

    fn row(&self, columns: &[Column]) -> Row {
        const BUSY_HIGH_THRESH: f64 = 80.0;
        const BUSY_MEDIUM_THRESH: f64 = 50.0;

        let pct_busy = self[" %busy"].as_float();
        let color = if pct_busy > BUSY_HIGH_THRESH {
            Color::Red
        } else if pct_busy > BUSY_MEDIUM_THRESH {
            Color::Magenta
        } else {
            Color::Green
        };

        let cells = columns.iter()
            .filter(|col| col.enabled)
            .map(|col| {
                let style = Style::default();
                let style = if col.header == " %busy" {
                    style.fg(color)
                } else {
                    style
                };
                Cell::from((col.format)(&self.fields[col.header]))
                    .style(style)
            }).collect::<Vec<_>>();
        Row::new(cells)
    }
}

impl Index<&'static str> for Element {
    type Output = Field;

    fn index(&self, key: &'static str) -> &Self::Output {
        &self.fields[key]
    }
}

#[derive(Debug, Default)]
struct DataSource {
    items: Vec<Element>
}

pub struct StatefulTable {
    prev: Option<Snapshot>,
    cur: Snapshot,
    tree: Tree,
    state: TableState,
    data: DataSource
}

impl StatefulTable {
    fn new(sort_key: Option<&'static str>, reverse: bool)
        -> io::Result<StatefulTable>
    {
        let tree = Tree::new()?;
        let prev = None;
        // XXX difference from gstat: the first display will show stats since
        // boot, like iostat.
        let cur = Snapshot::new()?;
        let mut table = StatefulTable {
            prev,
            cur,
            tree,
            state: TableState::default(),
            data: DataSource::default(),
        };
        table.regen(sort_key, reverse)?;
        Ok(table)
    }
    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.data.items.len() - 1 {
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
                    self.data.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn refresh(&mut self, sort_key: Option<&'static str>, reverse: bool)
        -> io::Result<()>
    {
        self.prev = Some(mem::replace(&mut self.cur, Snapshot::new()?));
        self.regen(sort_key, reverse)?;
        Ok(())
    }

    /// Regenerate the DataSource
    fn regen(&mut self, sort_key: Option<&'static str>, reverse: bool)
        -> io::Result<()>
    {
        let etime = if let Some(prev) = self.prev.as_mut() {
            f64::from(self.cur.timestamp() - prev.timestamp())
        } else {
            let boottime = clock_gettime(ClockId::CLOCK_UPTIME)?;
            boottime.tv_sec() as f64 + boottime.tv_nsec() as f64 * 1e-9
        };
        self.data.items.clear();
        for (curstat, prevstat) in self.cur.iter_pair(self.prev.as_mut()) {
            if let Some(gident) = self.tree.lookup(curstat.id()) {
                if let Some(rank) = gident.rank() {
                    let stats = Statistics::compute(curstat, prevstat, etime);
                    let mut elem = Element::default();
                    elem.insert("Name", Field::Str(
                        gident.name().to_string_lossy().to_string()));
                    elem.insert("L(q)", Field::Int(stats.queue_length()));
                    elem.rank = rank;
                    elem.insert(" ops/s",
                                Field::Float(stats.transfers_per_second()));
                    elem.insert("   r/s", Field::Float(
                        stats.transfers_per_second_read()));
                    elem.insert("kB/r", Field::Float(
                        stats.kb_per_transfer_read()));
                    elem.insert("kB/s r", Field::Float(
                        stats.mb_per_second_read() * 1024.0));
                    elem.insert("  ms/r", Field::Float(
                        stats.ms_per_transaction_read()));
                    elem.insert("   w/s", Field::Float(
                        stats.transfers_per_second_write()));
                    elem.insert("kB/w", Field::Float(
                        stats.kb_per_transfer_write()));
                    elem.insert("kB/s w", Field::Float(
                        stats.mb_per_second_write() * 1024.0));
                    elem.insert("  ms/w", Field::Float(
                        stats.ms_per_transaction_write()));
                    elem.insert("   d/s", Field::Float(
                        stats.transfers_per_second_free()));
                    elem.insert("kB/d", Field::Float(
                        stats.kb_per_transfer_free()));
                    elem.insert("kB/s d", Field::Float(
                        stats.mb_per_second_free() * 1024.0));
                    elem.insert("  ms/d", Field::Float(
                        stats.ms_per_transaction_free()));
                    elem.insert("   o/s", Field::Float(
                        stats.transfers_per_second_other()));
                    elem.insert("  ms/o", Field::Float(
                        stats.ms_per_transaction_other()));
                    elem.insert(" %busy", Field::Float(stats.busy_pct()));
                    self.data.items.push(elem);
                }
            }
        }
        if let Some(k) = sort_key {
            self.data.items.sort_by(|l, r| {
                if reverse {
                    r.fields[k].partial_cmp(&l.fields[k])
                } else {
                    l.fields[k].partial_cmp(&r.fields[k])
                }.unwrap()
            });
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    const _COL_QD: usize = 0;
    const _COL_OPS_S: usize = 1;
    const _COL_R_S: usize = 2;
    const COL_KB_R: usize = 3;
    const _COL_KBS_R: usize = 4;
    const _COL_MS_R: usize = 5;
    const _COL_W_S: usize = 6;
    const COL_KB_W: usize = 7;
    const _COL_KBS_W: usize = 8;
    const _COL_MS_W: usize = 9;
    const COL_D_S: usize = 10;
    const COL_KB_D: usize = 11;
    const COL_KBS_D: usize = 12;
    const COL_MS_D: usize = 13;
    const _COL_O_S: usize = 14;
    const _COL_MS_O: usize = 15;
    const _COL_PCT_BUSY: usize = 16;
    const _COL_NAME: usize = 17;
    const _COL_MAX: usize = 18;

    let mut cli: Cli = Cli::parse_args_default_or_exit();
    let mut filter = cli.filter.as_ref().map(|s| Regex::new(s).unwrap());
    let mut tick_rate: Duration = match cli.interval.as_mut() {
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

    let mut columns = [
        Column::new("L(q)", true, Constraint::Length(5),
            |f| format!("{:>4}", f.as_int())),
        Column::new(" ops/s", true, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("   r/s", true, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("kB/r", cli.size, Constraint::Length(5),
            |f| format!("{:>4.0}", f.as_float())),
        Column::new("kB/s r", true, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("  ms/r", true, Constraint::Length(7),
            |f| format!("{:>6.1}", f.as_float())),
        Column::new("   w/s", true, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("kB/w", cli.size, Constraint::Length(5),
            |f| format!("{:>4.0}", f.as_float())),
        Column::new("kB/s w", true, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("  ms/w", true, Constraint::Length(7),
            |f| format!("{:>6.1}", f.as_float())),
        Column::new("   d/s", cli.delete, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("kB/d", cli.size && cli.delete, Constraint::Length(5),
            |f| format!("{:>4.0}", f.as_float())),
        Column::new("kB/s d", cli.delete, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("  ms/d", cli.delete, Constraint::Length(7),
            |f| format!("{:>6.1}", f.as_float())),
        Column::new("   o/s", cli.other, Constraint::Length(7),
            |f| format!("{:>6.0}", f.as_float())),
        Column::new("  ms/o", cli.other, Constraint::Length(7),
            |f| format!("{:>6.1}", f.as_float())),
        Column::new(" %busy", true, Constraint::Length(7),
            |f| format!("{:>6.1}", f.as_float())),
        Column::new("Name", true, Constraint::Min(10),
            |f| f.as_str().to_string()),
    ];

    let mut sort_idx: Option<usize> = cli.sort.as_ref()
        .map(|name| columns.iter()
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

    let sort_key = sort_idx.map(|idx| columns[idx].header);
    let mut table = StatefulTable::new(sort_key, cli.reverse)?;

    let normal_style = Style::default().bg(Color::Blue);
    let selected_style = Style::default().add_modifier(Modifier::REVERSED);

    loop {
        terminal.draw(|f| {
            let rects = Layout::default()
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(f.size());

            let header_cells = columns.iter()
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
            let rows = table.data.items.iter()
                .filter(|item| !cli.auto || item[" %busy"].as_float() > 0.1)
                .filter(|item| !cli.physical || item.rank == 1)
                .filter(|item|
                        filter.as_ref()
                        .map(|f| f.is_match(item["Name"].as_str()))
                        .unwrap_or(true)
                ).map(|item| {
                    item.row(&columns)
                });
            let widths = columns.iter()
                .filter(|col| col.enabled)
                .map(|col| col.width)
                .collect::<Vec<_>>();
            let t = Table::new(rows)
                .header(header)
                .block(Block::default())
                .highlight_style(selected_style)
                .widths(&widths[..]);
            f.render_stateful_widget(t, rects[0], &mut table.state);

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
                let sort_key = sort_idx.map(|idx| columns[idx].header);
                table.refresh(sort_key, cli.reverse)?;
            }
            Some(Event::Key(key)) => {
                if editting_regex {
                    match key {
                        Key::Char('\n') => {
                            editting_regex = false;
                            filter = Some(Regex::new(&new_regex)?);
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
                        Key::Char('<') => {
                            tick_rate /= 2;
                        }
                        Key::Char('>') => {
                            tick_rate *= 2;
                        }
                        Key::Char('a') => {
                            cli.auto ^= true;
                        }
                        Key::Char('d') => {
                            cli.delete ^= true;
                            columns[COL_D_S].enabled = cli.delete;
                            columns[COL_KB_D].enabled = cli.delete && cli.size;
                            columns[COL_KBS_D].enabled = cli.delete;
                            columns[COL_MS_D].enabled = cli.delete;
                        }
                        Key::Char('o') => {
                            for col in columns.iter_mut() {
                                let flushcols = ["   o/s", "  ms/o"];
                                if flushcols.contains(&col.header)  {
                                    col.enabled ^= true;
                                }
                            }
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
                                        sort_idx = Some(columns.len() - 1);
                                    }
                                }
                                if sort_idx.is_none() {
                                    break;
                                }
                                if columns[sort_idx.unwrap()].enabled {
                                    break;
                                }
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
                                if idx >= columns.len() {
                                    sort_idx = None;
                                    break;
                                }
                                if columns[idx].enabled {
                                    sort_idx = Some(idx);
                                    break;
                                }
                            }
                        }
                        Key::Char('p') => {
                            cli.physical ^= true;
                        }
                        Key::Char('r') => {
                            cli.reverse ^= true;
                        }
                        Key::Char('s') => {
                            cli.size ^= true;
                            columns[COL_KB_R].enabled = cli.size;
                            columns[COL_KB_W].enabled = cli.size;
                            columns[COL_KB_D].enabled = cli.delete && cli.size;
                        }
                        Key::Char('F') => {
                            filter = None;
                        }
                        Key::Char('f') => {
                            editting_regex = true;
                            new_regex = String::new();
                        }
                        Key::Char('q') => {
                            break;
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
