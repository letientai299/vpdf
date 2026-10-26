use super::{InputOwner, transfers_overlay};
use crate::context::{ContextMenu, Transition as Menu};
use crate::keys::{Action, Grammar, Input, Keys, parse};
use crate::layout::Layout;
use crate::overview::{Grid, Navigation, Overview};
use crate::preview::{Event, Preview};
use crate::region::{CaptureId, Region, Snapshot, Transition as Capture};
use crate::text::{Bounds, PageId};
use crate::window::Window;
use vpdf_reader::motion::Motion;

fn layout() -> Layout {
  Layout::new(
    [Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 }],
    crate::testing::screen::cells(80, 24),
  )
}

fn region() -> Region {
  Region::open(Snapshot {
    id: CaptureId::new(1),
    page: PageId(0),
    bounds: Bounds { x0: 2.0, y0: 3.0, x1: 20.0, y1: 30.0 },
  })
}

#[test]
fn transfer_preserves_input() {
  let mut menu = ContextMenu::<()>::new(Vec::new(), (0, 0), (0, 80), 23, false);
  let mut region = region();
  let layout = layout();
  for action in [
    Action::Window(Window::New),
    Action::Tab(true),
    Action::LocalFocus,
    Action::History(false),
    Action::Help,
    Action::ClosePanel,
  ] {
    assert!(transfers_overlay(action));
    assert!(
      menu.allows(action) && region.allows(action) && Preview::allows(action)
    );
    assert_eq!(menu.input(action), Menu::Transfer);
    assert_eq!(region.input(action), Capture::Transfer);
    let input = Input { action, count: 3, explicit: true, register: 'a' };
    let Event::Transfer(transferred) = Preview::input(input, &layout) else {
      panic!("Missing transfer")
    };
    assert_eq!(transferred.action, action);
    assert_eq!(transferred.count, 3);
    assert!(transferred.explicit);
    assert_eq!(transferred.register, 'a');
  }
}

#[test]
fn command_and_ignored_actions() {
  let mut menu = ContextMenu::<()>::new(Vec::new(), (0, 0), (0, 80), 23, false);
  let mut region = region();
  let layout = layout();
  assert!(!transfers_overlay(Action::Command));
  assert_eq!(menu.input(Action::Command), Menu::Command);
  assert_eq!(region.input(Action::Command), Capture::Command);
  assert!(matches!(
    Preview::input(Input::plain(Action::Command), &layout),
    Event::Action(_)
  ));
  for action in [Action::Zoom(1), Action::Definition, Action::WordSearch] {
    assert!(
      !menu.allows(action)
        && !region.allows(action)
        && !Preview::allows(action)
    );
    assert_eq!(menu.input(action), Menu::Unhandled);
    assert_eq!(region.input(action), Capture::Stay);
    assert!(matches!(
      Preview::input(Input::plain(action), &layout),
      Event::Ignored
    ));
  }
}

#[test]
fn clues_follow_overlay() {
  let menu = ContextMenu::<()>::new(Vec::new(), (0, 0), (0, 80), 23, false);
  let region = region();
  let allows = |owner, action| match owner {
    InputOwner::Context => menu.allows(action),
    InputOwner::Peek => Preview::allows(action),
    InputOwner::Region => region.allows(action),
    _ => Overview::allows(action),
  };
  for owner in [
    InputOwner::Context,
    InputOwner::Peek,
    InputOwner::Region,
    InputOwner::Overview,
  ] {
    for prefix in ["g", "z"] {
      let mut keys = Keys::default();
      for key in parse(&format!("\"a2{prefix}")).unwrap() {
        keys.feed(key, Grammar::Other);
      }
      let before = keys.pending();
      let labels: Vec<_> = keys
        .clues(Grammar::Other, |action| allows(owner, action))
        .into_iter()
        .map(|(key, _)| key)
        .collect();
      assert_eq!(keys.pending(), before);
      if prefix == "z" {
        assert_eq!(labels, [] as [String; 0]);
        continue;
      }
      for key in ["?", "T", "a", "o", "q", "r", "s", "t"] {
        assert!(labels.contains(&key.to_string()), "{owner:?} {key}");
      }
      for key in ["E", "d", "l", "v", "x"] {
        assert!(!labels.contains(&key.to_string()), "{owner:?} {key}");
      }
      assert_eq!(labels.contains(&"F".to_owned()), owner == InputOwner::Region);
      assert_eq!(
        labels.contains(&"g".to_owned()),
        owner == InputOwner::Overview
      );
      assert_eq!(
        labels.contains(&"p".to_owned()),
        owner == InputOwner::Overview
      );
    }
  }
}

#[test]
fn overview_ignores_unhandled_actions() {
  let layout = layout();
  let grid = Grid::new(
    layout.screen(),
    Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 },
  );
  let mut overview = Overview::new(PageId(0));
  let right = Input {
    action: Action::Motion(Motion::Right),
    count: 3,
    explicit: true,
    register: 'a',
  };
  assert!(matches!(overview.navigate(right, &grid, 10), Navigation::Handled));
  assert_eq!(overview.selected(), 3);
  for action in [
    Action::Command,
    Action::Help,
    Action::ClosePanel,
    Action::LocalFocus,
    Action::YankPath,
  ] {
    assert!(Overview::allows(action));
    assert!(matches!(
      overview.navigate(Input::plain(action), &grid, 10),
      Navigation::Pass
    ));
    assert_eq!(overview.selected(), 3);
  }
  for action in
    [Action::Zoom(1), Action::Definition, Action::Motion(Motion::Word)]
  {
    assert!(!Overview::allows(action));
    assert!(matches!(
      overview.navigate(Input::plain(action), &grid, 10),
      Navigation::Handled
    ));
    assert_eq!(overview.selected(), 3);
  }
}
