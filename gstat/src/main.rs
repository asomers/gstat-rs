use gumdrop::Options;
use freebsd_libgeom::{Snapshot, Statistics, Tree};
use nix::time::{ClockId, clock_gettime};
use regex::Regex;
use rustbox::{
    Event,
    keyboard::Key
};
use std::{
    error::Error,
    io,
    mem,
    time::Duration
};
use tui::{
    backend::RustboxBackend,
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
    /// batch mode.  Collect numbers, print and exit. (unimplemented)
    #[options(short = 'b')]
    batch: bool,
    /// endless batch mode.  Same as batch mode, but does not exit after
    /// collecting the first set of data. (unimplemented)
    #[options(short = 'B')]
    endless_batch: bool,
    /// enable display of geom(4) consumers too. (unimplemented)
    #[options(short = 'c')]
    consumers: bool,
    /// output in CSV.  Implies endless batch mode. (unimplemented)
    #[options(short = 'C')]
    csv: bool,
    /// display statistics for delete (BIO_DELETE) operations. (unimplemented)
    #[options(short = 'd')]
    delete: bool,
    /// only display devices with names matching filter, as a regex.
    #[options(short = 'f')]
    filter: Option<String>,
    /// display statistics for other (BIO_FLUSH) operations. (unimplemented)
    #[options(short = 'o')]
    other: bool,
    /// display block size statistics (unimplemented)
    #[options(short = 's')]
    size: bool,
    /// display update interval, in microseconds or with the specified unit
    #[options(short = 'I')]
    interval: Option<String>,
    /// only display physical providers (those with rank of 1).
    #[options(short = 'p')]
    physical: bool
}

/// The data for one element in the table, usually a Geom provider
#[derive(Debug, Default)]
struct Element {
    qd: u32,
    ops_s: f64,
    r_s: f64,
    kbps_r: f64,
    ms_r: f64,
    w_s: f64,
    kbps_w: f64,
    ms_w: f64,
    pct_busy: f64,
    name: String,
    rank: u32,
}

impl Element {
    fn row(&self) -> Row {
        const BUSY_HIGH_THRESH: f64 = 80.0;
        const BUSY_MEDIUM_THRESH: f64 = 50.0;

        let color = if self.pct_busy > BUSY_HIGH_THRESH {
            Color::Red
        } else if self.pct_busy > BUSY_MEDIUM_THRESH {
            Color::Magenta
        } else {
            Color::Green
        };
        let busy_cell = Cell::from(format!("{:>6.1}", self.pct_busy))
            .style(Style::default().fg(color));

        Row::new([
            Cell::from(format!("{:>4}", self.qd)),
            Cell::from(format!("{:>6.0}", self.ops_s)),
            Cell::from(format!("{:>6.0}", self.r_s)),
            Cell::from(format!("{:>6.0}", self.kbps_r)),
            Cell::from(format!("{:>6.1}", self.ms_r)),
            Cell::from(format!("{:>6.0}", self.w_s)),
            Cell::from(format!("{:>6.0}", self.kbps_w)),
            Cell::from(format!("{:>6.1}", self.ms_w)),
            busy_cell,
            Cell::from(self.name.as_str()),
        ])
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
    fn new() -> io::Result<StatefulTable> {
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
        table.regen()?;
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

    pub fn refresh(&mut self) -> io::Result<()> {
        self.prev = Some(mem::replace(&mut self.cur, Snapshot::new()?));
        self.regen()?;
        Ok(())
    }

    /// Regenerate the DataSource
    fn regen(&mut self) -> io::Result<()> {
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
                    self.data.items.push(Element{
                        name: gident.name().to_string_lossy().to_string(),
                        rank,
                        qd: stats.queue_length(),
                        ops_s: stats.transfers_per_second(),
                        r_s: stats.transfers_per_second_read(),
                        kbps_r: stats.mb_per_second_read() * 1024.0,
                        ms_r: stats.ms_per_transaction_read(),
                        w_s: stats.transfers_per_second_write(),
                        kbps_w: stats.mb_per_second_write() * 1024.0,
                        ms_w: stats.ms_per_transaction_write(),
                        pct_busy: stats.busy_pct()
                    });
                }
            }
        }
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut cli: Cli = Cli::parse_args_default_or_exit();
    let mut filter = cli.filter.as_ref().map(|s| Regex::new(s).unwrap());
    let mut tick_rate: Duration = match cli.interval.as_mut() {
        None => Duration::from_secs(1),
        Some(s) => {
            if s.parse::<i32>().is_ok() {
                // Add the default units
                s.push_str("us");
            }
            humanize_rs::duration::parse(&s)?
        }
    };
    let mut editting_regex = false;
    let mut new_regex = String::new();

    // Terminal initialization
    let backend = RustboxBackend::new()?;
    let mut terminal = Terminal::new(backend)?;

    let mut table = StatefulTable::new()?;

    let normal_style = Style::default().bg(Color::Blue);
    let selected_style = Style::default().add_modifier(Modifier::REVERSED);

    loop {
        terminal.draw(|f| {
            let rects = Layout::default()
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(f.size());

            let header_cells = ["L(q)", " ops/s", "   r/s", "  kBps", "  ms/r",
                "   w/s", "  kBps", "  ms/w", " %busy", "Name"]
                .iter()
                .map(|h| Cell::from(*h).style(Style::default().fg(Color::Red)));
            let header = Row::new(header_cells)
                .style(normal_style);
            let rows = table.data.items.iter()
                .filter(|item| !cli.auto || item.pct_busy > 0.1)
                .filter(|item| !cli.physical || item.rank == 1)
                .filter(|item| filter.as_ref().map(|f| f.is_match(&item.name))
                        .unwrap_or(true)
                ).map(|item| {
                    item.row()
                });
            let t = Table::new(rows)
                .header(header)
                .block(Block::default())
                .highlight_style(selected_style)
                .widths(&[
                    Constraint::Min(5),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(7),
                    Constraint::Min(10),
                ])
                ;
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

        match terminal.backend().rustbox().peek_event(tick_rate, false) {
            Ok(Event::KeyEvent(key)) => {
                if editting_regex {
                    match key {
                        Key::Enter => {
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
                        Key::Char('p') => {
                            cli.physical ^= true;
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
            Ok(Event::NoEvent) => {
                // Timer tick.
                table.refresh()?;
            },
            Ok(Event::ResizeEvent(_, _)) => {
                // Window resize
            },
            e => {
                panic!("Unhandled event {:?}", e);
            }
        };
    }

    Ok(())
}
