//! Waits for terminal input, SIGWINCH, and worker results with one
//! `poll`, so an idle reader sleeps instead of waking every few ms.
use crate::Result;
use crate::decode::{Event, Parser, Reply};
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
  /// DA1 answers asked for and not yet read; one that missed its
  /// round's deadline still arrives later.
  owed: usize,
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
    owed: 0,
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

/// Sends `request` and a DA1 query, then reads until DA1's answer or
/// `timeout`. Returns the replies, ending with DA1's when it came;
/// other input stays queued, in order, for `next`.
pub fn ask(
  out: &mut impl Write,
  request: &[u8],
  timeout: Duration,
) -> Result<Vec<Reply>> {
  out.write_all(request)?;
  out.write_all(b"\x1b[c")?;
  out.flush()?;
  let mut reader = READER.lock().unwrap_or_else(PoisonError::into_inner);
  reader.as_mut().context("Input not started")?.ask(timeout)
}

impl Reader {
  fn ask(&mut self, timeout: Duration) -> Result<Vec<Reply>> {
    let deadline = Instant::now() + timeout;
    let mut replies = Vec::new();
    let mut typed = VecDeque::new();
    for event in std::mem::take(&mut self.events) {
      if da1(&event) {
        self.owed = self.owed.saturating_sub(1);
      } else {
        typed.push_back(event);
      }
    }
    self.owed += 1;
    // `next` settles one debt per DA1 answer; the last one owed is this
    // request's, and replies before an older answer are that request's.
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
      match self.next(Some(left))? {
        Some(Event::Reply(reply @ Reply::Attributes(_))) if self.owed == 0 => {
          replies.push(reply);
          break;
        }
        Some(Event::Reply(reply))
          if self.owed == 1 && !matches!(reply, Reply::Attributes(_)) =>
        {
          replies.push(reply);
        }
        Some(Event::Reply(_)) | None => {}
        Some(event) => typed.push_back(event),
      }
    }
    typed.append(&mut self.events);
    self.events = typed;
    // `next` may have taken a wake meant for the caller's loop.
    wake();
    Ok(replies)
  }

  fn next(&mut self, timeout: Option<Duration>) -> Result<Option<Event>> {
    let deadline = timeout.map(|timeout| Instant::now() + timeout);
    loop {
      if let Some(event) = self.events.pop_front() {
        if da1(&event) {
          self.owed = self.owed.saturating_sub(1);
        }
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

/// Forgets DA1 answers still owed; another program that takes the
/// terminal reads them.
pub fn forget_answers() {
  let mut reader = READER.lock().unwrap_or_else(PoisonError::into_inner);
  if let Some(reader) = reader.as_mut() {
    reader.owed = 0;
  }
}

fn da1(event: &Event) -> bool {
  matches!(event, Event::Reply(Reply::Attributes(_)))
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
    let reader =
      Reader { tty, parser, events, wake, winch, esc: None, owed: 0 };
    Ok((reader, [terminal, waker, signal]))
  }

  fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::from(code))
  }

  /// Keys typed during a probe stay input, in order, around replies.
  #[test]
  fn ask_keeps_typeahead() -> Result<()> {
    let (mut reader, [mut terminal, _waker, _signal]) = reader()?;
    terminal.write_all(b"j\x1bP>|kitty(0.49.1)\x1b\\k\x1b[?0u")?;
    terminal.write_all(b"\x1b[?62;52;cl\x1b[?62c")?;
    let replies = reader.ask(Duration::from_secs(1))?;
    assert_eq!(
      replies,
      [
        Reply::Version("kitty(0.49.1)".into()),
        Reply::Keyboard(0),
        Reply::Attributes(vec![62, 52]),
      ]
    );
    let short = Some(Duration::from_millis(1));
    for want in ['j', 'k', 'l'] {
      assert_eq!(reader.next(short)?, Some(key(KeyCode::Char(want))));
    }
    let late = Event::Reply(Reply::Attributes(vec![62]));
    assert_eq!(reader.next(short)?, Some(late));
    Ok(())
  }

  /// A DA1 answer that missed its round, and the replies before it,
  /// do not end or join the next round; the loop reading it settles
  /// the debt too.
  #[test]
  fn late_attributes_stay_with_their_round() -> Result<()> {
    let (mut reader, [mut terminal, _waker, _signal]) = reader()?;
    let short = Duration::from_millis(5);
    assert_eq!(reader.ask(short)?, []);
    terminal.write_all(b"\x1bP>|kitty(0.49.1)\x1b\\\x1b[?62c")?;
    terminal.write_all(b"\x1b_Gi=7;OK\x1b\\\x1b[?62;22c")?;
    let ok =
      Reply::Graphics { id: Some(7), placement: None, message: "OK".into() };
    let answer = [ok, Reply::Attributes(vec![62, 22])];
    assert_eq!(reader.ask(Duration::from_secs(1))?, answer);
    assert_eq!(reader.owed, 0);
    assert_eq!(reader.ask(short)?, []);
    terminal.write_all(b"\x1b[?62c")?;
    let late = Event::Reply(Reply::Attributes(vec![62]));
    assert_eq!(reader.next(Some(short))?, Some(late));
    terminal.write_all(b"\x1b[?62;22c")?;
    assert_eq!(
      reader.ask(Duration::from_secs(1))?,
      [Reply::Attributes(vec![62, 22])]
    );
    Ok(())
  }

  /// A late DA1 answer parsed before the next request settles its own
  /// debt there.
  #[test]
  fn queued_attributes_settle_first() -> Result<()> {
    let (mut reader, [mut terminal, _waker, _signal]) = reader()?;
    reader.owed = 1;
    reader.events.push_back(Event::Reply(Reply::Attributes(vec![62])));
    terminal.write_all(b"\x1b[?62;22c")?;
    let replies = reader.ask(Duration::from_secs(1))?;
    assert_eq!(replies, [Reply::Attributes(vec![62, 22])]);
    assert_eq!(reader.owed, 0);
    Ok(())
  }

  /// Without DA1's answer the probe ends at the deadline with what came.
  #[test]
  fn ask_ends_without_attributes() -> Result<()> {
    let (mut reader, [mut terminal, _waker, _signal]) = reader()?;
    terminal.write_all(b"\x1bP>|kitty(0.49.1)\x1b\\x")?;
    let started = Instant::now();
    let replies = reader.ask(Duration::from_millis(30))?;
    assert!(started.elapsed() >= Duration::from_millis(30));
    assert_eq!(replies, [Reply::Version("kitty(0.49.1)".into())]);
    let short = Some(Duration::from_millis(1));
    assert_eq!(reader.next(short)?, Some(key(KeyCode::Char('x'))));
    Ok(())
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
