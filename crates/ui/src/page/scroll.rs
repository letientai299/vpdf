use crate::keys::Action;
use std::time::Instant;
use vpdf_reader::reader::Reader;

#[derive(Clone)]
struct Flight {
  target: f32,
  updated: Instant,
}

#[derive(Clone, Default)]
pub struct Scroll {
  flight: Option<Flight>,
}

impl Scroll {
  pub fn start(&mut self, target: f32, now: Instant) {
    self.flight = Some(Flight { target, updated: now });
  }

  pub fn cancel(&mut self) {
    self.flight = None;
  }

  pub fn active(&self) -> bool {
    self.flight.is_some()
  }

  pub fn target(&self) -> Option<f32> {
    self.flight.as_ref().map(|flight| flight.target)
  }

  pub fn finish(&mut self) -> Option<f32> {
    self.flight.take().map(|flight| flight.target)
  }

  pub fn finish_before(&mut self, action: Action, reader: &mut Reader) {
    if reader.uses_cursor()
      && !matches!(
        action,
        Action::Scroll(_) | Action::Screen(_) | Action::Rows(_)
      )
      && let Some(target) = self.finish()
    {
      reader.scroll = target;
    }
  }

  pub fn step(&mut self, scroll: f32, now: Instant) -> Option<f32> {
    let flight = self.flight.as_mut()?;
    let elapsed = now.duration_since(flight.updated).as_secs_f32();
    flight.updated = now;
    let next = ease(scroll, flight.target, elapsed);
    let scroll = next.unwrap_or(flight.target);
    if next.is_none() {
      self.cancel();
    }
    Some(scroll)
  }
}

#[expect(clippy::float_cmp, reason = "detects a step that rounds back")]
fn ease(scroll: f32, target: f32, elapsed: f32) -> Option<f32> {
  let distance = target - scroll;
  let next = scroll + distance * (1.0 - (-elapsed / 0.018).exp());
  // Large-coordinate rounding must finish the animation.
  (distance.abs() >= 0.5 && next != scroll).then_some(next)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn easing_arrives_far_down_long_documents() {
    assert!(ease(0.0, 100.0, 0.008).is_some_and(|next| next > 30.0));
    assert_eq!(ease(99.8, 100.0, 0.008), None);
    assert_eq!(ease(4_384_724.5, 4_384_725.0, 0.008), None);
  }

  #[test]
  fn transitions_keep_time_and_target_together() {
    use std::time::Duration;
    let now = Instant::now();
    let frame = Duration::from_millis(8);
    let mut scroll = Scroll::default();
    assert_eq!(scroll.step(0.0, now), None);
    scroll.start(100.0, now);
    let first = scroll.step(0.0, now + frame).unwrap();
    assert!(first > 0.0 && first < 100.0);
    assert_eq!(scroll.target(), Some(100.0));
    scroll.start(200.0, now + frame);
    let next = scroll.step(first, now + frame * 2).unwrap();
    assert!(next > first && next < 200.0);
    assert_eq!(scroll.finish(), Some(200.0));
    assert!(!scroll.active());
    scroll.start(300.0, now + frame * 2);
    scroll.cancel();
    assert_eq!(scroll.finish(), None);
    assert_eq!(scroll.step(next, now + frame * 3), None);
    scroll.start(4_384_725.0, now);
    assert_eq!(scroll.step(4_384_724.5, now + frame), Some(4_384_725.0));
    assert!(!scroll.active());
  }

  #[test]
  fn cursor_actions_finish_pending_scroll() {
    let mut scroll = Scroll::default();
    let mut reader = Reader::default();
    scroll.start(100.0, Instant::now());
    scroll.finish_before(Action::Cancel, &mut reader);
    assert!(scroll.active());
    reader.enter_cursor();
    for action in [Action::Scroll(0.5), Action::Screen(1), Action::Rows(1)] {
      scroll.finish_before(action, &mut reader);
      assert!(scroll.active());
    }
    scroll.finish_before(Action::Cancel, &mut reader);
    assert!(!scroll.active());
    assert_eq!(reader.scroll, 100.0);
  }
}
