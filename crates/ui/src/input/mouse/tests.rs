use super::*;
use crate::text::PageId;

fn mouse(kind: MouseEventKind, modifiers: KeyModifiers) -> MouseEvent {
  MouseEvent { kind, column: 79, row: 5, modifiers }
}

#[test]
fn gestures_keep_their_press() {
  let bar = Bar::default();
  let mut gesture = Gesture::default();
  gesture.begin_sweep(PageId(0), (1.0, 2.0));
  for dropped in [false, true] {
    if dropped {
      gesture.drop_sweep();
    }
    for kind in [
      MouseEventKind::Drag(MouseButton::Left),
      MouseEventKind::Up(MouseButton::Left),
    ] {
      assert_eq!(
        Content::Page.dispatch(
          mouse(kind, KeyModifiers::NONE),
          false,
          &bar,
          &gesture,
          || panic!("Drag queried scrollbar")
        ),
        Dispatch::Alt
      );
    }
  }
  gesture.reset();
  let down = MouseEventKind::Down(MouseButton::Left);
  for (modifiers, expected) in [
    (KeyModifiers::NONE, Dispatch::Bar),
    (KeyModifiers::ALT, Dispatch::Alt),
    (KeyModifiers::CONTROL, Dispatch::Left),
  ] {
    assert_eq!(
      Content::Page.dispatch(
        mouse(down, modifiers),
        false,
        &bar,
        &gesture,
        || true
      ),
      expected
    );
  }
}

#[test]
fn overview_hides_old_drags() {
  let bar = Bar::default();
  let gesture = Gesture::default();
  let drag = mouse(MouseEventKind::Drag(MouseButton::Left), KeyModifiers::NONE);
  for (content, shown, expected) in [
    (Content::Page, true, Dispatch::Ignore),
    (Content::Page, false, Dispatch::Left),
    (Content::Overview, true, Dispatch::Overview),
    (Content::Overview, false, Dispatch::Overview),
  ] {
    assert_eq!(
      content.dispatch(drag, shown, &bar, &gesture, || panic!(
        "Drag queried scrollbar"
      )),
      expected
    );
  }
}
