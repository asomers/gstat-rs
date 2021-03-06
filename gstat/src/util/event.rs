use nix::poll::{PollFd, PollFlags, poll};
use std::{
    convert::TryInto,
    io,
    os::unix::io::AsRawFd,
    time::Duration
};

use termion::event::{self, Key, MouseEvent};
use termion::input::TermRead;

#[derive(Debug)]
pub enum Event {
    Key(Key),
    Mouse(MouseEvent),
    Tick,
}

/// A small event handler that wrap termion input and tick events. Each event
/// type is handled in its own thread and returned to a common `Receiver`
pub struct Events<T: TermRead> {
    inner: termion::input::Events<T>,
    pollfds: [PollFd; 1]
}

impl<T: TermRead> Events<T> {
    pub fn new(stdin: T) -> Events<T>  where T: AsRawFd {
        let pollfd = PollFd::new(stdin.as_raw_fd(), PollFlags::POLLIN);
        Events {
            inner: stdin.events(),
            pollfds: [pollfd]
        }
    }

    pub fn poll(&mut self, tick_rate: &Duration) -> Option<Event>
        where T: io::Read
    {
        let poll_timeout = tick_rate.as_millis().try_into().unwrap_or(-1i32);
        if poll(&mut self.pollfds[..], poll_timeout) == Ok(0) {
            Some(Event::Tick)
        } else {
            assert_eq!(self.pollfds[0].revents(), Some(PollFlags::POLLIN));
            match self.inner.next() {
                Some(Ok(event::Event::Key(key))) => Some(Event::Key(key)),
                Some(Ok(event::Event::Mouse(mev))) => Some(Event::Mouse(mev)),
                None => None,
                Some(Ok(event::Event::Unsupported(_))) => {
                    // Ignore unknown inputs.  This can be triggered by the
                    // arrows keys after the terminal gets into a screwy state.
                    None
                }
                Some(Err(e)) => {
                    eprintln!("Error: reading from terminal returned {}", e);
                    None
                }
            }
        }
    }
}
