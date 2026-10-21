use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

pub struct Watchdog {
  stop: mpsc::Sender<()>,
  thread: Option<JoinHandle<()>>,
}

impl Watchdog {
  pub fn new(deadline: Instant) -> Self {
    let (stop, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
      if matches!(
        receiver
          .recv_timeout(deadline.saturating_duration_since(Instant::now())),
        Err(mpsc::RecvTimeoutError::Timeout)
      ) {
        eprintln!("Headless scenario timed out");
        std::process::exit(124);
      }
    });
    Self { stop, thread: Some(thread) }
  }
}

impl Drop for Watchdog {
  fn drop(&mut self) {
    let _ = self.stop.send(());
    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}
