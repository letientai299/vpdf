//! Waits for terminal input, SIGWINCH, and worker results with one
//! `poll`, so an idle reader sleeps instead of waking every few ms.
use crate::Result;
use crate::decode::{Event, Parser};
use anyhow::Context;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

struct Reader {
  tty: File,
  parser: Parser,
  events: VecDeque<Event>,
  wake: UnixStream,
  winch: UnixStream,
  /// When a lone ESC started waiting for the rest of its sequence.
  esc: Option<Instant>,
}

/// Quiet time after which a lone ESC is the Esc key. Under Kitty's
/// keyboard protocol Esc arrives as `CSI 27 u`, so a lone ESC byte is a
/// sequence cut by the pty (1022-byte reads on macOS) or the network.
const ESC_WAIT: Duration = Duration::from_millis(25);

static READER: Mutex<Option<Reader>> = Mutex::new(None);

/// Write end of the wake socket.
static WAKER: OnceLock<UnixStream> = OnceLock::new();

/// Set while a wake byte waits, so a burst of results writes one.
static WOKEN: AtomicBool = AtomicBool::new(false);

/// Starts reading the terminal; later calls do nothing.
pub fn start() -> Result<()> {
  let mut reader = READER.lock().unwrap_or_else(PoisonError::into_inner);
  if reader.is_some() {
    return Ok(());
  }
  let (wake, waker) = UnixStream::pair()?;
  let (winch, signal) = UnixStream::pair()?;
  for socket in [&wake, &waker, &winch, &signal] {
    socket.set_nonblocking(true)?;
  }
  signal_hook::low_level::pipe::register(
    signal_hook::consts::SIGWINCH,
    signal,
  )?;
  let _ = WAKER.set(waker);
  let stdin = io::stdin();
  // Raw mode applies to the same terminal, as in crossterm.
  let tty = if stdin.is_terminal() {
    File::from(stdin.as_fd().try_clone_to_owned()?)
  } else {
    File::open("/dev/tty")?
  };
  *reader = Some(Reader {
    tty,
    parser: Parser::default(),
    events: VecDeque::new(),
    wake,
    winch,
    esc: None,
  });
  Ok(())
}

/// Makes a waiting `next` return; workers call it after sending results.
pub fn wake() {
  let Some(mut waker) = WAKER.get() else { return };
  if !WOKEN.swap(true, Ordering::AcqRel) {
    let _ = waker.write(&[0]);
  }
}

/// Passes on a send to the event loop's channel and wakes the loop.
pub fn notify<T>(sent: T) -> T {
  wake();
  sent
}

/// The next input; None once `timeout` passes or a worker wakes the
/// loop. Without a timeout it waits for either.
pub fn next(timeout: Option<Duration>) -> Result<Option<Event>> {
  let mut reader = READER.lock().unwrap_or_else(PoisonError::into_inner);
  reader.as_mut().context("Input not started")?.next(timeout)
}

impl Reader {
  fn next(&mut self, timeout: Option<Duration>) -> Result<Option<Event>> {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
      if let Some(event) = self.events.pop_front() {
        return Ok(Some(event));
      }
      let esc = self.esc.map(|since| since + ESC_WAIT);
      let until = match (deadline, esc) {
        (Some(deadline), Some(esc)) => Some(deadline.min(esc)),
        (deadline, esc) => deadline.or(esc),
      };
      let left = until
        .map(|until| until.saturating_duration_since(Instant::now()))
        .map(Timespec::try_from)
        .transpose()?;
      let mut fds = [
        PollFd::new(&self.tty, PollFlags::IN),
        PollFd::new(&self.wake, PollFlags::IN),
        PollFd::new(&self.winch, PollFlags::IN),
      ];
      match poll(&mut fds, left.as_ref()) {
        Ok(0) => {
          let now = Instant::now();
          if esc.is_some_and(|esc| esc <= now) {
            self.esc = None;
            self.parser.end(&mut self.events);
          } else if deadline.is_some_and(|deadline| deadline <= now) {
            return Ok(None);
          }
          continue;
        }
        Ok(_) => {}
        Err(Errno::INTR) => continue,
        Err(error) => return Err(error.into()),
      }
      let [tty, woke, resized] = fds.map(|fd| !fd.revents().is_empty());
      if resized {
        drain(&self.winch);
        self.events.push_back(Event::Resize(None));
      }
      if tty {
        self.read()?;
      }
      if woke {
        // Drain before clearing: a wake in between finds the flag set,
        // and its result is already in the channel the caller checks.
        drain(&self.wake);
        WOKEN.store(false, Ordering::Release);
        if self.events.is_empty() {
          return Ok(None);
        }
      }
    }
  }

  fn read(&mut self) -> Result<()> {
    let mut chunk = [0; 16 * 1024];
    let read = match rustix::io::read(&self.tty, &mut chunk) {
      Ok(read) => read,
      Err(Errno::INTR | Errno::AGAIN) => return Ok(()),
      Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(read > 0, "Terminal closed");
    self.parser.feed(&chunk[..read], &mut self.events);
    // A read that leaves a lone ESC ended with it, so its wait starts now.
    self.esc = self.parser.waiting().then(Instant::now);
    Ok(())
  }
}

fn drain(mut socket: &UnixStream) {
  let mut sink = [0; 64];
  while socket.read(&mut sink).is_ok_and(|read| read > 0) {}
}

#[cfg(test)]
mod tests {
  use super::*;
  use crossterm::event::{KeyCode, KeyEvent};
  use std::os::fd::OwnedFd;

  /// A reader on sockets; the test writes the first one returned and
  /// keeps the others open.
  fn reader() -> Result<(Reader, [UnixStream; 3])> {
    let (tty, terminal) = UnixStream::pair()?;
    let (wake, waker) = UnixStream::pair()?;
    let (winch, signal) = UnixStream::pair()?;
    let tty = File::from(OwnedFd::from(tty));
    let events = VecDeque::new();
    let parser = Parser::default();
    let reader = Reader { tty, parser, events, wake, winch, esc: None };
    Ok((reader, [terminal, waker, signal]))
  }

  fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::from(code))
  }

  #[test]
  fn lone_esc_waits_for_the_rest() -> Result<()> {
    let (mut reader, [mut terminal, _waker, _signal]) = reader()?;
    let short = Some(Duration::from_millis(1));
    terminal.write_all(b"\x1b")?;
    assert_eq!(reader.next(short)?, None, "ESC waits");
    terminal.write_all(b"[A")?;
    assert_eq!(reader.next(short)?, Some(key(KeyCode::Up)));
    terminal.write_all(b"\x1b")?;
    let started = Instant::now();
    let esc = reader.next(Some(Duration::from_secs(1)))?;
    assert_eq!(esc, Some(key(KeyCode::Esc)));
    assert!(started.elapsed() >= ESC_WAIT);
    drop(terminal);
    assert!(reader.next(short).is_err(), "closed");
    Ok(())
  }

  /// A second lone ESC waits its own quiet time, and a burst cut after
  /// an ESC into two writes stays mouse reports.
  #[test]
  fn each_esc_waits_its_own_quiet_time() -> Result<()> {
    let (mut reader, [mut terminal, _waker, _signal]) = reader()?;
    let short = Some(Duration::from_millis(1));
    terminal.write_all(b"\x1b")?;
    assert_eq!(reader.next(short)?, None);
    std::thread::sleep(ESC_WAIT * 2);
    terminal.write_all(b"[A\x1b")?;
    assert_eq!(reader.next(short)?, Some(key(KeyCode::Up)));
    assert_eq!(reader.next(Some(Duration::from_millis(5)))?, None);
    terminal.write_all(b"[B")?;
    assert_eq!(reader.next(short)?, Some(key(KeyCode::Down)));
    let burst = b"\x1b[<35;1;1M".repeat(100);
    let cut = burst.len() - 9;
    assert_eq!(burst[cut - 1], 0x1b);
    terminal.write_all(&burst[..cut])?;
    let mut events = 0;
    while let Some(event) = reader.next(short)? {
      assert!(matches!(event, Event::Mouse(_)), "{event:?}");
      events += 1;
    }
    terminal.write_all(&burst[cut..])?;
    while let Some(event) = reader.next(Some(ESC_WAIT * 2))? {
      assert!(matches!(event, Event::Mouse(_)), "{event:?}");
      events += 1;
    }
    assert_eq!(events, 100);
    Ok(())
  }
}
