#[path = "../../../../tests/support/scrollbar.rs"]
mod fixture;
use super::*;
use fixture::{layout, route, view};
#[test]
fn touching_ticks_of_one_color_merge() {
  let (a, b) = (theme::Paint::Rgba([1; 4]), theme::Paint::Rgba([2; 4]));
  // (ticks) -> runs
  let cases = [
    (vec![(0.0, a), (3.0, a)], vec![(0.0, 5.0, a)]),
    (vec![(0.0, a), (6.0, a)], vec![(0.0, 2.0, a), (6.0, 8.0, a)]),
    (vec![(0.0, a), (3.0, b)], vec![(0.0, 2.0, a), (3.0, 5.0, b)]),
    (vec![], vec![]),
  ];
  for (ticks, want) in cases {
    assert_eq!(runs(ticks.clone()), want, "{ticks:?}");
  }
}

#[test]
fn reflow_keeps_user_motion_deadlines() {
  let mut bar = Bar::default();
  let now = Instant::now();
  bar.observe(0.0, now);
  assert_eq!(bar.deadline(), None);
  bar.reflow(0.0, 100.0);
  bar.observe(100.0, now);
  assert_eq!(bar.deadline(), None);
  bar.observe(110.0, now);
  let due = now + Bar::SHOWN;
  assert_eq!(bar.deadline(), Some(due));
  assert!(!bar.expire(due.checked_sub(Duration::from_nanos(1)).unwrap()));
  assert!(bar.expire(due));
  assert!(!bar.expire(due));
  bar.reflow(100.0, 200.0);
  bar.observe(200.0, due);
  assert_eq!(bar.deadline(), Some(due + Bar::SHOWN));
}

#[test]
fn presses_record_only_at_their_boundary() {
  use vpdf_model::input::{
    MouseButton::Left,
    MouseEventKind::{Down, Drag},
  };
  let layout = layout();
  let mut bar = Bar::default();
  let mut reader = Reader::default();
  let origin = Location::default();
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 0))
  else {
    panic!("Missing thumb press")
  };
  assert!(!bar.scrubbing());
  assert_eq!(press.apply(&mut bar, origin, &mut reader), None);
  assert_eq!(reader.jumps().count(), 0);
  assert!(bar.release(&mut reader));
  assert_eq!(reader.jumps().count(), 0);
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 0))
  else {
    panic!("Missing thumb press")
  };
  press.apply(&mut bar, origin, &mut reader);
  let Input::Scroll(scroll) =
    bar.input(&view(&layout, 0.0), &route(Drag(Left), 5))
  else {
    panic!("Missing thumb drag")
  };
  reader.scroll = scroll;
  assert!(reader.scroll > 0.0);
  assert_eq!(reader.jumps().count(), 0);
  assert!(bar.release(&mut reader));
  assert_eq!(reader.jumps().count(), 1);
  assert!(!bar.release(&mut reader));
  let mut reader = Reader::default();
  let Input::Press(press) =
    bar.input(&view(&layout, 0.0), &route(Down(Left), 20))
  else {
    panic!("Missing track press")
  };
  assert!(!bar.scrubbing());
  assert!(press.apply(&mut bar, origin, &mut reader).unwrap() > 0.0);
  assert_eq!(reader.jumps().count(), 1);
}

#[test]
fn recorded_scrub_keeps_track_and_label() {
  use vpdf_model::scene::FillArea;
  let layout = layout();
  let (_, page, scene) = fixture::scrub(&layout, Instant::now());
  let Op::Fill(track) = &scene[0] else { panic!("Expected scrollbar track") };
  assert_eq!(
    track.area(),
    FillArea::Cells { column: 119, columns: 1, rows: 39 }
  );
  assert_eq!(track.layer(), Layer::BelowText);
  assert_eq!(page, PageId(0));
  assert!(crate::testing::text(&scene).contains("Section"));
}
