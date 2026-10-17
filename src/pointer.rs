//! Mouse routing and what the pointer rests on.

use crate::app::App;
use crate::split::Rect;
use crate::terminal::Shape;
use crate::text::PageId;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::time::{Duration, Instant};

/// Part of a pane under the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Region {
  Page,
  /// Page thumbnails in place of the page.
  Overview,
  Panel,
  /// Key clues or the completion menu.
  Overlay,
  Status,
}

/// A pane's cells and how they divide.
#[derive(Clone, Copy, Debug)]
pub struct Pane {
  pub id: u32,
  pub rect: Rect,
  /// Side panel columns at the left.
  pub panel: u16,
  /// Overlay rows above the status row.
  pub overlay: u16,
  pub overview: bool,
}

impl Pane {
  pub fn of(app: &App) -> Self {
    let rect = Rect {
      column: app.at.column,
      row: app.at.row,
      columns: app.screen.columns,
      rows: app.screen.rows,
    };
    Self {
      id: app.id,
      rect,
      panel: app.left,
      overlay: app.menu_rows(),
      overview: app.overview.is_some(),
    }
  }

  /// Region at a cell of this pane.
  fn region(&self, column: u16, row: u16) -> Region {
    let page = if self.overview { Region::Overview } else { Region::Page };
    [Region::Status, Region::Overlay, Region::Panel]
      .into_iter()
      .find(|region| self.area(*region).contains(column, row))
      .unwrap_or(page)
  }

  /// Cells of `region`, relative to the pane.
  fn area(&self, region: Region) -> Rect {
    let status = self.rect.rows.saturating_sub(1);
    let overlay = status.saturating_sub(self.overlay);
    let columns = self.rect.columns;
    match region {
      Region::Page | Region::Overview => Rect {
        column: self.panel,
        row: 0,
        columns: columns.saturating_sub(self.panel),
        rows: overlay,
      },
      Region::Panel => {
        Rect { column: 0, row: 0, columns: self.panel, rows: overlay }
      }
      Region::Overlay => {
        Rect { column: 0, row: overlay, columns, rows: status - overlay }
      }
      Region::Status => Rect { column: 0, row: status, columns, rows: 1 },
    }
  }
}

/// What a press hit, for its drags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capture {
  pub pane: u32,
  pub button: MouseButton,
  pub region: Region,
}

/// Where one event goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Route {
  pub pane: u32,
  pub region: Region,
  /// The pane takes keyboard focus.
  pub focus: bool,
  /// In pane cells, inside `region`.
  pub mouse: MouseEvent,
  /// Presses in a row on one spot; drags and releases keep it.
  pub clicks: u8,
  pub pixel: Option<Pixel>,
}

/// Screen pixels under mode 1016, with the cell size.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pixel {
  pub x: u16,
  pub y: u16,
  pub cell: (u16, u16),
}

/// What the pointer rests on inside a region.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Hit {
  #[default]
  Nothing,
  Text,
  /// Index into the page's `Document::links`.
  Link(PageId, usize),
  /// Index into the page's `Document::annotations`.
  Note {
    page: PageId,
    index: usize,
    /// Over text, not a margin icon.
    text: bool,
  },
  /// Index into the panel's rows.
  Row(usize),
  /// An overview thumbnail.
  Thumbnail(PageId),
}

impl Hit {
  pub fn shape(self) -> Shape {
    match self {
      Self::Nothing | Self::Note { text: false, .. } => Shape::Default,
      Self::Text | Self::Note { text: true, .. } => Shape::Text,
      Self::Link(..) | Self::Row(_) | Self::Thumbnail(_) => Shape::Pointer,
    }
  }
}

/// Pane, region, and hit under the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Hover {
  pub pane: u32,
  pub region: Region,
  pub hit: Hit,
}

/// Drags follow the press; only presses focus.
pub fn route(
  panes: &[Pane],
  focused: u32,
  capture: Option<Capture>,
  mouse: MouseEvent,
) -> Option<Route> {
  let (pane, region) = match mouse.kind {
    MouseEventKind::Drag(button) | MouseEventKind::Up(button) => {
      let capture = capture.filter(|capture| capture.button == button)?;
      (panes.iter().find(|pane| pane.id == capture.pane)?, capture.region)
    }
    _ => {
      let (column, row) = (mouse.column, mouse.row);
      let pane = panes.iter().find(|pane| pane.rect.contains(column, row))?;
      (pane, pane.region(column - pane.rect.column, row - pane.rect.row))
    }
  };
  let area = pane.area(region);
  let column = mouse.column.saturating_sub(pane.rect.column);
  let row = mouse.row.saturating_sub(pane.rect.row);
  Some(Route {
    pane: pane.id,
    region,
    focus: matches!(mouse.kind, MouseEventKind::Down(_)) && pane.id != focused,
    mouse: MouseEvent {
      column: within(column, area.column, area.columns),
      row: within(row, area.row, area.rows),
      ..mouse
    },
    clicks: 1,
    pixel: None,
  })
}

/// Clamps into `length` cells from `start`.
fn within(value: u16, start: u16, length: u16) -> u16 {
  value.clamp(start, start + length.saturating_sub(1))
}

/// Presses and wheel turns cancel keys.
pub fn interrupts(kind: MouseEventKind) -> bool {
  !matches!(
    kind,
    MouseEventKind::Drag(_) | MouseEventKind::Up(_) | MouseEventKind::Moved
  )
}

/// Presses this close form one gesture.
const CLICK: Duration = Duration::from_millis(350);

/// Cells a press moves before it drags.
const THRESHOLD: u16 = 1;

/// Pixels a press moves before it drags: half a cell width.
fn drag(cell: (u16, u16)) -> u16 {
  cell.0 / 2
}

/// Click run reach: half a cell height, like Kitty.
fn click(cell: (u16, u16)) -> u16 {
  cell.1 / 2
}

/// The latest press, for counts and drags.
#[derive(Clone, Copy, Debug)]
struct Press {
  button: MouseButton,
  /// Opening a thumbnail ends the run.
  on: Option<(u32, Region)>,
  /// Until the release, drags follow the press.
  held: bool,
  column: u16,
  row: u16,
  pixel: Option<Pixel>,
  at: Instant,
  clicks: u8,
  dragged: bool,
}

impl Press {
  /// Whether a spot stays within `reach` pixels, else one cell.
  fn near(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
    reach: fn((u16, u16)) -> u16,
  ) -> bool {
    match self.pixel.zip(pixel) {
      Some((from, to)) => {
        let reach = reach(to.cell).max(1);
        from.x.abs_diff(to.x) < reach && from.y.abs_diff(to.y) < reach
      }
      None => {
        self.column.abs_diff(column) < THRESHOLD
          && self.row.abs_diff(row) < THRESHOLD
      }
    }
  }
}

/// Pointer state a session keeps between events.
#[derive(Debug, Default)]
pub struct Pointer {
  press: Option<Press>,
  hover: Option<Hover>,
  /// Last spot, to retest after view changes.
  spot: Option<(MouseEvent, Option<Pixel>)>,
}

impl Pointer {
  /// A press captures until its button's release; drags start past the
  /// threshold.
  pub fn route(
    &mut self,
    panes: &[Pane],
    focused: u32,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Option<Route> {
    let capture = self.capture();
    let held = capture.map(|capture| capture.button);
    if let MouseEventKind::Down(button) = mouse.kind
      && held.is_some_and(|held| held != button)
    {
      // One button at a time.
      return None;
    }
    if let MouseEventKind::Drag(button) = mouse.kind
      && held == Some(button)
      && let Some(press) = &mut self.press
    {
      press.dragged |= !press.near(mouse.column, mouse.row, pixel, drag);
      if !press.dragged {
        return None;
      }
    }
    self.spot =
      Some((MouseEvent { kind: MouseEventKind::Moved, ..mouse }, pixel));
    let route = route(panes, focused, capture, mouse);
    match mouse.kind {
      MouseEventKind::Down(button) => {
        let on = route.map(|route| (route.pane, route.region));
        let again = self.press.filter(|press| {
          press.button == button
            && press.on == on
            && !press.dragged
            && press.near(mouse.column, mouse.row, pixel, click)
            && now.saturating_duration_since(press.at) < CLICK
        });
        self.press = Some(Press {
          button,
          on,
          held: true,
          column: mouse.column,
          row: mouse.row,
          pixel,
          at: now,
          clicks: again.map_or(1, |press| press.clicks.saturating_add(1)),
          dragged: false,
        });
      }
      MouseEventKind::Up(button) if held == Some(button) => {
        self.hold(false);
      }
      // The view moved under the next press.
      MouseEventKind::ScrollDown
      | MouseEventKind::ScrollUp
      | MouseEventKind::ScrollLeft
      | MouseEventKind::ScrollRight
        if held.is_none() =>
      {
        self.press = None;
      }
      _ => {}
    }
    let clicks = match mouse.kind {
      MouseEventKind::Down(_)
      | MouseEventKind::Drag(_)
      | MouseEventKind::Up(_) => self.press.map_or(1, |press| press.clicks),
      _ => 1,
    };
    route.map(|route| Route { clicks, pixel, ..route })
  }

  /// Bare motion; whether the target changed.
  pub fn hover(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    hover: Option<Hover>,
  ) -> bool {
    // No button is down: a release got lost.
    self.hold(false);
    self.spot = Some((mouse, pixel));
    self.refresh(hover)
  }

  /// The pointer left the window; whether a hover ended.
  pub fn leave(&mut self) -> bool {
    self.spot = None;
    self.refresh(None)
  }

  /// Sets what the pointer rests on; whether it changed.
  pub fn refresh(&mut self, hover: Option<Hover>) -> bool {
    std::mem::replace(&mut self.hover, hover) != hover
  }

  /// What the pointer rests on.
  pub fn target(&self) -> Option<Hover> {
    self.hover
  }

  /// Bare motion at the last reported spot.
  pub fn spot(&self) -> Option<(MouseEvent, Option<Pixel>)> {
    self.spot
  }

  /// Ends the held press and its click run.
  pub fn release(&mut self) {
    self.press = None;
  }

  /// The pane and region a held press drags.
  fn capture(&self) -> Option<Capture> {
    let press = self.press.filter(|press| press.held)?;
    let (pane, region) = press.on?;
    Some(Capture { pane, button: press.button, region })
  }

  fn hold(&mut self, held: bool) {
    if let Some(press) = &mut self.press {
      press.held = held;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use MouseButton::{Left, Right};
  use MouseEventKind::{Down, Drag, Moved, ScrollDown, Up};
  use crossterm::event::KeyModifiers;
  use std::time::Duration;

  const fn pane(id: u32, rect: Rect, panel: u16, overlay: u16) -> Pane {
    Pane { id, rect, panel, overlay, overview: false }
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
    let cases = [
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
      (
        event(Up(Left), 5, 2),
        Some(held(1, Page)),
        Some((1, Page, false, 20, 2)),
      ),
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
    for (step, (kind, column, x, ms, expected)) in cases.into_iter().enumerate()
    {
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
  fn hover_reports_target_changes() {
    let hover = |pane, region, hit| Some(Hover { pane, region, hit });
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
      assert_eq!(
        pointer.spot().map(|(spot, _)| spot.column),
        Some(step as u16)
      );
    }
    // A press moves the spot without bare motion.
    let now = Instant::now();
    pointer.route(&PANES, 1, event(Down(Left), 30, 12), None, now);
    let spot =
      pointer.spot().map(|(spot, _)| (spot.kind, spot.column, spot.row));
    assert_eq!(spot, Some((Moved, 30, 12)));
  }

  #[test]
  fn hits_pick_pointer_shapes() {
    let cases = [
      (Hit::Nothing, Shape::Default),
      (Hit::Text, Shape::Text),
      (Hit::Note { page: PageId(0), index: 1, text: true }, Shape::Text),
      (Hit::Note { page: PageId(0), index: 1, text: false }, Shape::Default),
      (Hit::Link(PageId(0), 2), Shape::Pointer),
      (Hit::Row(3), Shape::Pointer),
      (Hit::Thumbnail(PageId(4)), Shape::Pointer),
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
}
