use super::InputOwner;
use crate::pointer::Hit;
use crate::text::{PageId, Position};
use std::time::{Duration, Instant};
use vpdf_model::document::Target;
use vpdf_model::input::MouseEventKind;

pub const REST: Duration = Duration::from_millis(400);

#[derive(Default)]
pub struct Hover {
  hit: Hit,
  rest: Option<Rest>,
}

struct Rest {
  link: (PageId, usize),
  since: Option<Instant>,
}

impl Hover {
  pub fn hit(&self) -> Hit {
    self.hit
  }

  pub fn resting(&self) -> Option<(PageId, usize)> {
    self.rest.as_ref().map(|rest| rest.link)
  }

  pub fn point(
    &mut self,
    hit: Hit,
    beneath: Option<(PageId, usize)>,
    now: impl FnOnce() -> Instant,
  ) {
    self.hit = hit;
    if self.resting() != beneath {
      self.rest = beneath.map(|link| Rest { link, since: Some(now()) });
    }
  }

  pub fn due(
    &self,
    owner: InputOwner,
    preview: bool,
  ) -> Option<(PageId, usize, Instant)> {
    let rest = self.rest.as_ref()?;
    let since = rest.since?;
    if owner != InputOwner::Content || preview {
      return None;
    }
    Some((rest.link.0, rest.link.1, since + REST))
  }

  pub fn restart(&mut self, now: impl FnOnce() -> Instant) {
    if let Some(since) = self.rest.as_mut().and_then(|r| r.since.as_mut()) {
      *since = now();
    }
  }

  pub fn calm(&mut self) {
    if let Some(rest) = &mut self.rest {
      rest.since = None;
    }
  }
}

#[derive(Default)]
pub struct Press {
  state: State,
}

#[derive(Default)]
enum State {
  #[default]
  Idle,
  Held(Target),
  Note(PageId, usize, Option<Position>),
  Followed,
}

pub enum Step {
  Stop,
  Follow(Target),
  Note(PageId, usize),
  Drag(Option<Position>),
  Begin { inspect: bool },
}

impl Press {
  pub fn step(&mut self, kind: MouseEventKind, clicks: u8) -> Step {
    if matches!(kind, MouseEventKind::Up(_)) {
      return match std::mem::take(&mut self.state) {
        State::Held(target) => {
          self.state = State::Followed;
          Step::Follow(target)
        }
        State::Note(page, index, _) => {
          self.state = State::Followed;
          Step::Note(page, index)
        }
        state => {
          self.state = state;
          Step::Stop
        }
      };
    }
    if clicks > 1 && matches!(self.state, State::Followed) {
      return Step::Stop;
    }
    if matches!(kind, MouseEventKind::Drag(_)) {
      let from = match self.state {
        State::Note(_, _, from) => from,
        _ => None,
      };
      return Step::Drag(from);
    }
    Step::Begin { inspect: clicks == 1 }
  }

  pub fn link(&mut self, target: Option<Target>) {
    self.state = target.map_or(State::Idle, State::Held);
  }

  pub fn note(&mut self, page: PageId, index: usize, from: Option<Position>) {
    self.state = State::Note(page, index, from);
  }

  pub fn cancel(&mut self) {
    self.state = State::Idle;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::input::MouseButton;

  #[test]
  fn rest_survives_only_the_same_link() {
    let now = Instant::now();
    let link = (PageId(1), 3);
    let mut hover = Hover::default();
    hover.point(Hit::Link(link.0, link.1), Some(link), || now);
    let due = Some((link.0, link.1, now + REST));
    assert_eq!(hover.due(InputOwner::Content, false), due);
    let later = now + Duration::from_millis(100);
    hover.point(Hit::Preview { follows: true }, Some(link), || later);
    assert_eq!(hover.due(InputOwner::Content, false), due);
    assert_eq!(hover.hit(), Hit::Preview { follows: true });
    assert!(hover.due(InputOwner::Prompt, false).is_none());
    assert!(hover.due(InputOwner::Content, true).is_none());
    hover.restart(|| later);
    assert_eq!(
      hover.due(InputOwner::Content, false),
      Some((link.0, link.1, later + REST))
    );
    hover.calm();
    hover.restart(|| panic!("calmed hover reads clock"));
    hover.point(Hit::Link(link.0, link.1), Some(link), || {
      panic!("unchanged hover reads clock")
    });
    assert!(hover.due(InputOwner::Content, false).is_none());
    hover.point(Hit::Nothing, None, || panic!("absent link reads clock"));
    assert!(hover.resting().is_none());
    hover.point(Hit::Link(link.0, link.1), Some(link), || later);
    assert_eq!(
      hover.due(InputOwner::Content, false),
      Some((link.0, link.1, later + REST))
    );
  }

  #[test]
  fn releases_consume_a_click_run() {
    let mut press = Press::default();
    let down = MouseEventKind::Down(MouseButton::Left);
    let up = MouseEventKind::Up(MouseButton::Left);
    assert!(matches!(press.step(up, 1), Step::Stop));
    assert!(matches!(press.step(down, 1), Step::Begin { inspect: true }));
    press.link(Some(Target::Page(PageId(2))));
    assert!(matches!(press.step(up, 1), Step::Follow(Target::Page(PageId(2)))));
    assert!(matches!(press.step(down, 2), Step::Stop));
    assert!(matches!(press.step(up, 2), Step::Stop));
    assert!(matches!(press.step(down, 1), Step::Begin { inspect: true }));
    press.link(None);
    assert!(matches!(press.step(down, 2), Step::Begin { inspect: false }));
    assert!(matches!(press.step(up, 2), Step::Stop));
  }

  #[test]
  fn note_drag_waits_for_selection() {
    let mut press = Press::default();
    let position = Position { page: PageId(1), line: 2, glyph: 3 };
    press.note(position.page, 4, Some(position));
    let drag = MouseEventKind::Drag(MouseButton::Left);
    for _ in 0..2 {
      let step = press.step(drag, 1);
      assert!(matches!(step, Step::Drag(Some(from)) if from == position));
    }
    press.cancel();
    let up = MouseEventKind::Up(MouseButton::Left);
    assert!(matches!(press.step(up, 1), Step::Stop));
    press.note(position.page, 4, None);
    assert!(matches!(press.step(up, 1), Step::Note(PageId(1), 4)));
    assert!(matches!(press.step(up, 1), Step::Stop));
  }
}
