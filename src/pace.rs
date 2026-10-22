//! Frame cadence for the event loop.
//!
//! macOS lets a timed wait end up to a quarter of its length late: an
//! 8 ms poll returned after about 10 ms. The pacer waits three quarters
//! of the time left, repeatedly, and schedules each frame from the
//! previous deadline rather than from when drawing finished.
use std::time::{Duration, Instant};

pub const FPS: u32 = 180;
pub const FRAME: Duration = Duration::from_nanos(1_000_000_000 / FPS as u64);
/// A frame this close to its deadline draws now.
const EARLY: Duration = Duration::from_micros(250);

pub struct Pacer {
  due: Instant,
}

impl Pacer {
  pub fn new(now: Instant) -> Self {
    Self { due: now }
  }

  /// Whether a frame drawn now meets its deadline.
  pub fn ready(&self, now: Instant) -> bool {
    now + EARLY >= self.due
  }

  /// Schedules the next frame one interval after this one's deadline,
  /// or after now when the loop fell behind or was idle.
  pub fn drawn(&mut self, now: Instant) {
    self.due = next(self.due, now);
  }

  /// How long to wait for input before checking the deadline again.
  pub fn wait(&self, now: Instant) -> Duration {
    self.due.saturating_duration_since(now) * 3 / 4
  }
}

pub fn next(due: Instant, now: Instant) -> Instant {
  if now < due + FRAME { due + FRAME } else { now + FRAME }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keeps_cadence_and_wakes_early() {
    let start = Instant::now();
    let mut pacer = Pacer::new(start);
    assert!(pacer.ready(start));
    pacer.drawn(start + Duration::from_millis(1));
    assert_eq!(pacer.due, start + FRAME);
    // A late frame keeps the cadence; an idle gap restarts it.
    pacer.drawn(start + FRAME + Duration::from_millis(3));
    assert_eq!(pacer.due, start + FRAME * 2);
    let idle = start + Duration::from_secs(1);
    pacer.drawn(idle);
    assert_eq!(pacer.due, idle + FRAME);
    assert!(!pacer.ready(idle));
    assert!(pacer.wait(idle) * 4 / 3 <= FRAME);
    assert!(pacer.ready(idle + FRAME.saturating_sub(EARLY)));
  }
}
