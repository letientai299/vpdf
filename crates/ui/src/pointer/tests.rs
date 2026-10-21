use super::*;
use MouseButton::{Left, Right};
use MouseEventKind::{Down, Drag, Moved, ScrollDown, Up};
use std::time::Duration;
use vpdf_model::input::KeyModifiers;

const fn pane(id: u32, rect: Rect, panel: u16, overlay: u16) -> Pane {
  Pane {
    id,
    rect,
    panel,
    overlay,
    overview: false,
    preview: None,
    menu: None,
    actions: None,
  }
}

/// A left pane with a 20-column panel and 3 menu rows, a separator at
/// column 60, and two panes stacked on the right.
const PANES: [Pane; 3] = [
  pane(1, Rect { column: 0, row: 0, columns: 60, rows: 40 }, 20, 3),
  pane(2, Rect { column: 61, row: 0, columns: 60, rows: 20 }, 0, 0),
  pane(3, Rect { column: 61, row: 20, columns: 60, rows: 20 }, 0, 0),
];

fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
  MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
}

fn held(pane: u32, region: Region) -> Capture {
  Capture { pane, button: Left, region }
}

#[test]
fn regions_split_rows_then_columns() {
  let thumbnails = Pane { overview: true, ..PANES[0] };
  let popups = Pane {
    preview: Some(Rect { column: 25, row: 5, columns: 20, rows: 10 }),
    menu: Some(Rect { column: 40, row: 12, columns: 10, rows: 30 }),
    ..PANES[0]
  };
  let bar = Pane {
    actions: Some(Rect { column: 22, row: 20, columns: 30, rows: 1 }),
    ..PANES[0]
  };
  let cases = [
    (popups, (25, 5), Region::Preview),
    (popups, (30, 14), Region::Preview),
    (popups, (24, 5), Region::Page),
    (popups, (25, 15), Region::Page),
    (popups, (45, 13), Region::Menu),
    (popups, (42, 13), Region::Menu),
    (popups, (45, 30), Region::Menu),
    (popups, (45, 37), Region::Overlay),
    (popups, (45, 39), Region::Status),
    (bar, (22, 20), Region::Actions),
    (bar, (51, 20), Region::Actions),
    (bar, (22, 21), Region::Page),
    (bar, (52, 20), Region::Page),
    (PANES[0], (30, 0), Region::Page),
    (PANES[0], (20, 35), Region::Page),
    (PANES[0], (19, 35), Region::Panel),
    (PANES[0], (0, 36), Region::Overlay),
    (PANES[0], (40, 38), Region::Overlay),
    (PANES[0], (40, 39), Region::Status),
    (PANES[0], (5, 39), Region::Status),
    (thumbnails, (30, 0), Region::Overview),
    (thumbnails, (19, 0), Region::Panel),
    (PANES[1], (0, 18), Region::Page),
    (PANES[1], (0, 19), Region::Status),
  ];
  for (pane, (column, row), expected) in cases {
    let got = pane.region(column, row);
    assert_eq!(got, expected, "pane {} at {column},{row}", pane.id);
  }
}

#[test]
fn routes_to_pane_region_and_focus() {
  use Region::{Overlay, Page, Panel, Status};
  // (event, capture) -> (pane, region, focus, column, row)
  type Case =
    (MouseEvent, Option<Capture>, Option<(u32, Region, bool, u16, u16)>);
  let cases: [Case; 18] = [
    (event(Down(Left), 90, 10), None, Some((2, Page, true, 29, 10))),
    (event(Down(Left), 90, 25), None, Some((3, Page, true, 29, 5))),
    (event(Down(Left), 30, 10), None, Some((1, Page, false, 30, 10))),
    (event(Down(Left), 10, 5), None, Some((1, Panel, false, 10, 5))),
    (event(Down(Left), 30, 37), None, Some((1, Overlay, false, 30, 37))),
    (event(Down(Left), 90, 39), None, Some((3, Status, true, 29, 19))),
    (event(Down(Left), 60, 10), None, None),
    (event(ScrollDown, 90, 10), None, Some((2, Page, false, 29, 10))),
    (event(Moved, 90, 30), None, Some((3, Page, false, 29, 10))),
    (event(Drag(Left), 90, 10), None, None),
    (
      event(Drag(Left), 90, 39),
      Some(held(1, Page)),
      Some((1, Page, false, 59, 35)),
    ),
    (event(Up(Left), 5, 2), Some(held(1, Page)), Some((1, Page, false, 20, 2))),
    (
      event(Drag(Left), 40, 10),
      Some(held(1, Panel)),
      Some((1, Panel, false, 19, 10)),
    ),
    (
      event(Drag(Left), 10, 30),
      Some(held(2, Page)),
      Some((2, Page, false, 0, 18)),
    ),
    (
      event(Drag(Left), 90, 5),
      Some(held(3, Page)),
      Some((3, Page, false, 29, 0)),
    ),
    (
      event(Drag(Left), 30, 10),
      Some(held(1, Status)),
      Some((1, Status, false, 30, 39)),
    ),
    (event(Drag(Right), 30, 10), Some(held(1, Page)), None),
    (event(Up(Left), 30, 10), Some(held(4, Page)), None),
  ];
  for (mouse, capture, expected) in cases {
    let got = route(&PANES, 1, capture, mouse).map(|route| {
      let Route { pane, region, focus, mouse, .. } = route;
      (pane, region, focus, mouse.column, mouse.row)
    });
    assert_eq!(got, expected, "{mouse:?} {capture:?}");
  }
}

#[test]
fn press_captures_until_its_release() {
  let mut pointer = Pointer::default();
  let now = Instant::now();
  let mut feed = |kind, column| {
    let route = pointer.route(&PANES, 1, event(kind, column, 10), None, now);
    (route.map(|route| route.pane), pointer.capture())
  };
  let page = Some(held(1, Region::Page));
  let right = Capture { button: Right, ..held(1, Region::Page) };
  // (event, column) -> (pane, capture)
  let cases = [
    (Down(Left), 30, (Some(1), page)),
    (Drag(Left), 100, (Some(1), page)),
    (ScrollDown, 100, (Some(2), page)),
    (Down(Right), 90, (None, page)),
    (Up(Right), 90, (None, page)),
    (Up(Left), 100, (Some(1), None)),
    (Down(Left), 30, (Some(1), page)),
    (Down(Left), 60, (None, None)),
    (Down(Right), 90, (Some(2), Some(Capture { pane: 2, ..right }))),
    (Up(Left), 90, (None, Some(Capture { pane: 2, ..right }))),
  ];
  for (step, (kind, column, expected)) in cases.into_iter().enumerate() {
    assert_eq!(feed(kind, column), expected, "step {step}: {kind:?}");
  }
  // Bare motion means no button is down.
  pointer.hover(event(Moved, 30, 10), None, None);
  assert_eq!(pointer.capture(), None);
  let route = pointer.route(&PANES, 1, event(Down(Left), 90, 10), None, now);
  assert_eq!(route.map(|route| route.pane), Some(2));
  assert_eq!(pointer.capture(), Some(held(2, Region::Page)));
}

#[test]
fn clicks_count_and_drags_wait_for_the_threshold() {
  let mut pointer = Pointer::default();
  let start = Instant::now();
  // (event, column, ms since start) -> routed clicks, None when held
  let cases = [
    (Down(Left), 30, 0, Some(1)),
    (Up(Left), 30, 40, Some(1)),
    (Down(Left), 30, 200, Some(2)),
    (Drag(Left), 30, 210, None),
    (Up(Left), 30, 250, Some(2)),
    (Down(Left), 30, 400, Some(3)),
    (Up(Left), 30, 420, Some(3)),
    // Too slow.
    (Down(Left), 30, 800, Some(1)),
    (Up(Left), 30, 820, Some(1)),
    // Another cell.
    (Down(Left), 31, 900, Some(1)),
    (Drag(Left), 32, 950, Some(1)),
    // Back on the press cell, still dragging.
    (Drag(Left), 31, 960, Some(1)),
    (Up(Left), 31, 970, Some(1)),
    // A drag ends the run.
    (Down(Left), 31, 1000, Some(1)),
    (Up(Left), 31, 1010, Some(1)),
    // Another button.
    (Down(Right), 31, 1100, Some(1)),
    (Up(Right), 31, 1110, Some(1)),
    (Down(Left), 31, 1200, Some(1)),
    (Up(Left), 31, 1210, Some(1)),
    // The wheel moves the view.
    (ScrollDown, 31, 1220, Some(1)),
    (Down(Left), 31, 1230, Some(1)),
    (Up(Left), 31, 1240, Some(1)),
    (Down(Left), 31, 1250, Some(2)),
  ];
  for (step, (kind, column, ms, expected)) in cases.into_iter().enumerate() {
    let now = start + Duration::from_millis(ms);
    let route = pointer.route(&PANES, 1, event(kind, column, 10), None, now);
    let got = route.map(|route| route.clicks);
    assert_eq!(got, expected, "step {step}: {kind:?} at {column}");
  }
  // Focus changes and dismissed clicks end the run.
  pointer.release();
  let now = start + Duration::from_millis(1260);
  let route = pointer.route(&PANES, 1, event(Down(Left), 31, 10), None, now);
  assert_eq!(route.map(|route| route.clicks), Some(1));
  // A thumbnail press opens its page under the next press.
  let thumbnails = [Pane { overview: true, ..PANES[0] }];
  let press = |pointer: &mut Pointer, panes: &[Pane], ms| {
    let now = start + Duration::from_millis(ms);
    let route = pointer.route(panes, 1, event(Down(Left), 30, 10), None, now);
    pointer.route(panes, 1, event(Up(Left), 30, 10), None, now);
    route.map(|route| (route.region, route.clicks))
  };
  let mut pointer = Pointer::default();
  let overview = Some((Region::Overview, 1));
  assert_eq!(press(&mut pointer, &thumbnails, 0), overview);
  assert_eq!(press(&mut pointer, &PANES, 100), Some((Region::Page, 1)));
  assert_eq!(press(&mut pointer, &PANES, 200), Some((Region::Page, 2)));
}

#[test]
fn alt_clicks_run_apart() {
  let mut pointer = Pointer::default();
  let start = Instant::now();
  let (alt, plain) = (KeyModifiers::ALT, KeyModifiers::NONE);
  let shift = KeyModifiers::SHIFT;
  // modifiers -> clicks, one quick burst
  let cases =
    [(alt, 1), (plain, 1), (plain, 2), (shift, 3), (alt, 1), (alt, 2)];
  for (step, (modifiers, want)) in cases.into_iter().enumerate() {
    let now = start + Duration::from_millis(step as u64 * 20);
    let mut feed = |kind| {
      let mouse = MouseEvent { modifiers, ..event(kind, 30, 10) };
      pointer.route(&PANES, 1, mouse, None, now)
    };
    let route = feed(Down(Left));
    feed(Up(Left));
    assert_eq!(route.map(|route| route.clicks), Some(want), "step {step}");
  }
}

#[test]
fn pixels_refine_the_threshold() {
  let mut pointer = Pointer::default();
  let start = Instant::now();
  let at = |x| Some(Pixel { x, y: 165, cell: (8, 16) });
  // (event, column, pixel x, ms) -> routed clicks, None when held
  let cases = [
    (Down(Left), 30, 245, 0, Some(1)),
    (Drag(Left), 30, 247, 10, None),
    (Drag(Left), 31, 248, 20, None),
    (Up(Left), 31, 248, 30, Some(1)),
    // Within half a cell height: a double click across cells.
    (Down(Left), 31, 252, 100, Some(2)),
    (Up(Left), 31, 252, 110, Some(2)),
    (Down(Left), 32, 261, 200, Some(1)),
    // Half a cell width starts a drag inside one cell.
    (Drag(Left), 32, 265, 210, Some(1)),
    (Up(Left), 32, 265, 220, Some(1)),
  ];
  for (step, (kind, column, x, ms, expected)) in cases.into_iter().enumerate() {
    let now = start + Duration::from_millis(ms);
    let route = pointer.route(&PANES, 1, event(kind, column, 10), at(x), now);
    let got = route.map(|route| (route.clicks, route.pixel));
    assert_eq!(got, expected.map(|clicks| (clicks, at(x))), "step {step}");
  }
  pointer.hover(event(Moved, 32, 10), at(265), None);
  assert!(pointer.spot().is_some());
  pointer.leave();
  assert_eq!(pointer.spot(), None, "leaving forgets the spot");
}

#[test]
fn dropped_repeats_extend_the_run() {
  let mut pointer = Pointer::default();
  let start = Instant::now();
  let at = |ms| start + Duration::from_millis(ms);
  let middle = event(Down(MouseButton::Middle), 30, 10);
  pointer.route(&PANES, 1, middle, None, at(0));
  pointer.route(
    &PANES,
    1,
    event(Up(MouseButton::Middle), 30, 10),
    None,
    at(20),
  );
  // Each repeat counts from the last, not the first press.
  for ms in [200, 400, 600] {
    assert!(pointer.repeats(middle, None, at(ms)), "{ms} ms");
  }
  assert!(!pointer.repeats(middle, None, at(1000)), "too slow");
  let elsewhere = event(Down(MouseButton::Middle), 40, 10);
  assert!(!pointer.repeats(elsewhere, None, at(1100)), "another cell");
}

#[test]
fn hover_reports_target_changes() {
  let hover =
    |pane, region, hit| Some(Hover { pane, region, hit, beneath: None });
  let link = Hit::Link(PageId(1), 0);
  let mut pointer = Pointer::default();
  // hover -> changed
  let cases = [
    (hover(1, Region::Page, Hit::Text), true),
    (hover(1, Region::Page, Hit::Text), false),
    (hover(1, Region::Page, link), true),
    (hover(1, Region::Page, link), false),
    (hover(1, Region::Page, Hit::Link(PageId(1), 1)), true),
    (hover(2, Region::Page, Hit::Link(PageId(1), 1)), true),
    (hover(2, Region::Status, Hit::Nothing), true),
    (None, true),
    (None, false),
  ];
  for (step, (target, changed)) in cases.into_iter().enumerate() {
    let mouse = event(Moved, step as u16, 10);
    assert_eq!(pointer.hover(mouse, None, target), changed, "step {step}");
    assert_eq!(pointer.spot().map(|(spot, _)| spot.column), Some(step as u16));
  }
  // A press moves the spot without bare motion.
  let now = Instant::now();
  pointer.route(&PANES, 1, event(Down(Left), 30, 12), None, now);
  let spot = pointer.spot().map(|(spot, _)| (spot.kind, spot.column, spot.row));
  assert_eq!(spot, Some((Moved, 30, 12)));
}

#[test]
fn items_pick_the_topmost() {
  let cells = |column, columns| Rect { column, row: 4, columns, rows: 1 };
  let mut items = Items::default();
  items.push(cells(10, 3), 0);
  items.push(cells(12, 3), 1);
  // column -> item
  let cases = [(9, None), (10, Some(0)), (12, Some(1)), (14, Some(1))];
  for (column, want) in cases {
    assert_eq!(items.at(column, 4), want, "{column}");
  }
  assert_eq!(items.at(10, 5), None);
}

#[test]
fn hits_pick_pointer_shapes() {
  let cases = [
    (Hit::Nothing, Shape::Default),
    (Hit::Text, Shape::Text),
    (Hit::Note { page: PageId(0), index: 1, text: true }, Shape::Text),
    (Hit::Note { page: PageId(0), index: 1, text: false }, Shape::Pointer),
    (Hit::Link(PageId(0), 2), Shape::Pointer),
    (Hit::Row(3), Shape::Pointer),
    (Hit::Label(3), Shape::Default),
    (Hit::Thumbnail(PageId(4)), Shape::Pointer),
    (Hit::Preview { follows: true }, Shape::Pointer),
    (Hit::Preview { follows: false }, Shape::Default),
    (Hit::Item(List::Menu, Some(0)), Shape::Pointer),
    (Hit::Item(List::Hints, Some(1)), Shape::Pointer),
    (Hit::Item(List::Actions, None), Shape::Default),
    (Hit::Chrome, Shape::Pointer),
    (Hit::Edge { rows: false }, Shape::ColResize),
    (Hit::Edge { rows: true }, Shape::RowResize),
  ];
  for (hit, shape) in cases {
    assert_eq!(hit.shape(), shape, "{hit:?}");
  }
}

#[test]
fn presses_and_wheel_interrupt() {
  assert!(interrupts(Down(Left)));
  assert!(interrupts(ScrollDown));
  assert!(!interrupts(Drag(Left)));
  assert!(!interrupts(Up(Left)));
  assert!(!interrupts(Moved));
}

#[test]
fn turns_at_rest_form_one_gesture() {
  let start = Instant::now();
  let at = |ms| start + Duration::from_millis(ms);
  let pixel = |x| Some(Pixel { x, y: 40, cell: (8, 16) });
  // (column, pixel x, what came before, ms) -> continues the last turn
  let cases = [
    (5, pixel(44), None, 0, false),
    (5, pixel(44), None, 100, true),
    (5, pixel(44), None, 340, true),
    (5, pixel(45), None, 360, false),
    (6, pixel(52), None, 380, false),
    (6, pixel(52), Some(Moved), 400, false),
    (6, pixel(52), None, 420, true),
    (6, pixel(52), Some(Down(Left)), 440, false),
    (6, pixel(52), None, 680, true),
    (6, pixel(52), None, 930, false),
  ];
  let mut pointer = Pointer::default();
  for (column, pixel, before, ms, continues) in cases {
    match before {
      Some(Moved) => {
        pointer.hover(event(Moved, column, 2), pixel, None);
      }
      Some(kind) => {
        pointer.route(&PANES, 1, event(kind, column, 2), pixel, at(ms));
        pointer.release();
      }
      None => {}
    }
    let turn = event(ScrollDown, column, 2);
    assert_eq!(pointer.turn(turn, pixel, at(ms)), continues, "{ms} ms");
  }
}
