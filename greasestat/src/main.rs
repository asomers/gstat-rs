use argh::FromArgs;
use freebsd_libgeom::{Snapshot, Statistics, Tree};
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
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Cell, Row, Table, TableState},
    Terminal,
};

/// Drop-in compatible gstat(8) replacement
#[derive(Debug, FromArgs)]
struct Cli {
    /// only display providers that are at least 0.1% busy (unimplemented)
    #[argh(switch, short = 'a')]
    auto: bool,
    /// batch mode.  Collect numbers, print and exit. (unimplemented)
    #[argh(switch, short = 'b')]
    batch: bool,
    /// endless batch mode.  Same as batch mode, but does not exit after
    /// collecting the first set of data. (unimplemented)
    #[argh(switch, short = 'B')]
    endless_batch: bool,
    /// enable display of geom(4) consumers too. (unimplemented)
    #[argh(switch, short = 'c')]
    consumers: bool,
    /// output in CSV.  Implies endless batch mode. (unimplemented)
    #[argh(switch, short = 'C')]
    csv: bool,
    /// display statistics for delete (BIO_DELETE) operations. (unimplemented)
    #[argh(switch, short = 'd')]
    delete: bool,
    /// only display devices with names matching filter, as a regex.
    /// (unimplemented)
    #[argh(option, short = 'f')]
    filter: Option<String>,
    /// display statistics for other (BIO_FLUSH) operations. (unimplemented)
    #[argh(switch, short = 'o')]
    other: bool,
    /// display block size statistics (unimplemented)
    #[argh(switch, short = 's')]
    size: bool,
    /// display update interval, in microseconds or with the specified unit
    /// (unimplemented)
    #[argh(option, short = 'I', default = "String::from(\"1s\")")]
    interval: String,
    /// only display physical providers (those with rank of 1). (unimplemented)
    #[argh(switch, short = 'p')]
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
}

impl Element {
    fn row(&self) -> Row {
        Row::new([
            Cell::from(format!("{:>4}", self.qd)),
            Cell::from(format!("{:>6.0}", self.ops_s)),
            Cell::from(format!("{:>6.0}", self.r_s)),
            Cell::from(format!("{:>6.0}", self.kbps_r)),
            Cell::from(format!("{:>6.1}", self.ms_r)),
            Cell::from(format!("{:>6.0}", self.w_s)),
            Cell::from(format!("{:>6.0}", self.kbps_w)),
            Cell::from(format!("{:>6.1}", self.ms_w)),
            Cell::from(format!("{:>6.1}", self.pct_busy)),
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
        table.regen();
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
        self.regen();
        Ok(())
    }

    /// Regenerate the DataSource
    fn regen(&mut self) {
        let etime = if let Some(prev) = self.prev.as_mut() {
            f64::from(self.cur.timestamp() - prev.timestamp())
        } else {
            // TODO: get it with Nix
            //let boottime = clock_gettime(ClockId::CLOCK_UPTIME)?;
            //boottime.tv_sec() as f64 + boottime.tv_nsec() as f64 * 1e-9
            1.0
        };
        self.data.items.clear();
        for (curstat, prevstat) in self.cur.iter_pair(self.prev.as_mut()) {
            if let Some(gident) = self.tree.lookup(curstat.id()) {
                if gident.rank().is_some() {
                    let stats = Statistics::compute(curstat, prevstat, etime);
                    self.data.items.push(Element{
                        name: gident.name().to_string_lossy().to_string(),
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
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli: Cli = argh::from_env();

    // Terminal initialization
    let backend = RustboxBackend::new()?;
    let mut terminal = Terminal::new(backend)?;

    let mut table = StatefulTable::new()?;

    let normal_style = Style::default().bg(Color::Blue);
    let selected_style = Style::default().add_modifier(Modifier::REVERSED);

    let tick_rate = Duration::from_millis(500);

    // Input
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
            let rows = table.data.items.iter().map(|item| {
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
        }).unwrap();

        match terminal.backend().rustbox().peek_event(tick_rate, false) {
            Ok(Event::KeyEvent(key)) => {
                match key {
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
