#[allow(dead_code)]
mod util;

use crate::util::event::{Event, Events};
use freebsd_libgeom::{Snapshot, Statistics, Tree};
use std::{
    error::Error,
    io,
    mem
};
use termion::{
    event::Key,
    input::MouseTerminal,
    raw::IntoRawMode,
    screen::AlternateScreen
};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Cell, Row, Table, TableState},
    Terminal,
};

/// The data for one element in the table, usually a Geom provider
#[derive(Debug, Default)]
struct Element {
    name: String,
    qd: u32,
    ops_s: f64,
    r_s: f64
}

impl Element {
    fn new(name: &str, qd: u32, ops_s: f64, r_s: f64) -> Self {
        Element {name: name.to_owned(), qd, ops_s, r_s}
    }

    fn row(&self) -> Row {
        Row::new([
            Cell::from(format!("{:>4}", self.qd)),
            Cell::from(format!("{:>6.0}", self.ops_s)),
            Cell::from(format!("{:>6.0}", self.r_s)),
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
                    self.data.items.push(Element::new(
                            &gident.name().to_string_lossy(),
                            stats.queue_length(),
                            stats.transfers_per_second(),
                            stats.transfers_per_second_read()
                        )
                    );
                }
            }
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // Terminal initialization
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let events = Events::new();

    let mut table = StatefulTable::new()?;

    let normal_style = Style::default().bg(Color::Blue);
    let selected_style = Style::default().add_modifier(Modifier::REVERSED);

    // Input
    loop {
        terminal.draw(|f| {
            let rects = Layout::default()
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(f.size());

            let header_cells = ["L(q)", " ops/s", "   r/s", "Name"]
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
                    Constraint::Min(10),
                ])
                ;
            f.render_stateful_widget(t, rects[0], &mut table.state);
        }).unwrap();

        if let Event::Input(key) = events.next().unwrap() {
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
        } else {
            // Timer tick.
            table.refresh()?;
        };
    }

    Ok(())
}
