//! Saves reading positions when the terminal goes away.
//!
//! A closed terminal sends SIGHUP while a child such as fzf may hold
//! the tty, so the event loop cannot always save. A thread waits for
//! SIGHUP or SIGTERM, gives the loop a second to exit and save on its
//! own, then writes the last recorded state, restores the terminal, and
//! exits.
use crate::Result;
use crate::prompt::History;
use crate::state::State;
use signal_hook::consts::{SIGHUP, SIGTERM};
use signal_hook::iterator::Signals;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// State as last recorded by the event loop, for the hangup thread.
pub type Latest = Arc<Mutex<Option<(State, History)>>>;

/// Sets `flag` on SIGHUP or SIGTERM, then saves `latest` to `store` and
/// exits unless the process ended first.
pub fn watch(
  store: Option<PathBuf>,
  flag: Arc<AtomicBool>,
  latest: Latest,
) -> Result<()> {
  let mut signals = Signals::new([SIGHUP, SIGTERM])?;
  thread::spawn(move || {
    let Some(signal) = signals.forever().next() else { return };
    flag.store(true, Ordering::Relaxed);
    crate::input::wake();
    thread::sleep(Duration::from_secs(1));
    if let Some(store) = store
      && let Ok(mut latest) = latest.lock()
      && let Some((state, history)) = latest.as_mut()
    {
      drop(state.save(&store, &history.commands, &history.searches));
    }
    // The loop may block in a prompt that never sees the flag.
    crate::terminal::restore();
    std::process::exit(128 + signal);
  });
  Ok(())
}
