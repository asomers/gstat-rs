#[allow(dead_code)]
mod util;

use crate::util::event::{Event, Events};
use std::{error::Error, io};
use termion::{event::Key, input::MouseTerminal, raw::IntoRawMode, screen::AlternateScreen};
use tui::{
    backend::TermionBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Row, Table, TableState},
    Terminal,
};

/// The data for one element in the table, usually a Geom provider
struct Element {
    name: String,
    qd: u32,
    ops_s: f64
}

impl Element {
    fn new(name: &str, qd: u32, ops_s: f64) -> Self {
        Element {name: name.to_owned(), qd, ops_s}
    }

    fn row(&self) -> Row {
        Row::new([
            Cell::from(format!("{:>4}", self.qd)),
            Cell::from(format!("{:>6.0}", self.ops_s)),
            Cell::from(self.name.as_str()),
        ])
    }
}

struct DataSource {
    items: Vec<Element>
}

impl DataSource {
    fn new() -> DataSource {
        Self {
            items: vec![
                Element::new("nvd0", 0, 0.0),
                Element::new("nvd0p1", 0, 0.0),
                Element::new("nvd0p2", 0, 0.0),
                Element::new("nvd0p3", 0, 0.0),
                Element::new("nvd0p4", 0, 0.0),
                Element::new("gpt/gptboot0", 0, 0.0),
                Element::new("gpt/bfffs0", 0, 0.0),
                Element::new("ada0", 0, 0.0),
                Element::new("cd0", 0, 0.0),
                Element::new("ada1", 0, 0.0),
                Element::new("ufsid/0123456789abcdef", 0, 0.0),
            ]
        }
    }
}

pub struct StatefulTable {
    state: TableState,
    data: DataSource
}

impl StatefulTable {
    fn new() -> StatefulTable {
        StatefulTable {
            state: TableState::default(),
            data: DataSource::new(),
        }
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
}

fn main() -> Result<(), Box<dyn Error>> {
    // Terminal initialization
    let stdout = io::stdout().into_raw_mode()?;
    let stdout = MouseTerminal::from(stdout);
    let stdout = AlternateScreen::from(stdout);
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let events = Events::new();

    let mut table = StatefulTable::new();

    // Input
    loop {
        terminal.draw(|f| {
            let rects = Layout::default()
                .constraints([Constraint::Percentage(100)].as_ref())
                .split(f.size());

            let selected_style = Style::default().add_modifier(Modifier::REVERSED);
            let normal_style = Style::default().bg(Color::Blue);
            let header_cells = ["L(q)", " ops/s", "Name"]
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
        };
    }

    Ok(())
}
