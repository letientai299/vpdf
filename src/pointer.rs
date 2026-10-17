//! Which pane and region each mouse event reaches.

use crate::app::App;
use crate::split::Rect;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

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

/// Pointer state a session keeps between events.
#[derive(Debug, Default)]
pub struct Pointer {
  capture: Option<Capture>,
}

impl Pointer {
  /// A press captures until its button's release.
  pub fn route(
    &mut self,
    panes: &[Pane],
    focused: u32,
    mouse: MouseEvent,
  ) -> Option<Route> {
    let held = self.capture.map(|capture| capture.button);
    if let MouseEventKind::Down(button) = mouse.kind
      && held.is_some_and(|held| held != button)
    {
      // One button at a time.
      return None;
    }
    let route = route(panes, focused, self.capture, mouse);
    match mouse.kind {
      MouseEventKind::Down(button) => {
        self.capture = route.map(|route| Capture {
          pane: route.pane,
          button,
          region: route.region,
        });
      }
      MouseEventKind::Up(button) if held == Some(button) => {
        self.capture = None;
      }
      MouseEventKind::Moved => self.hover(),
      _ => {}
    }
    route
  }

  /// Bare motion: no button is down.
  pub fn hover(&mut self) {
    // Drops a press whose release was lost.
    self.capture = None;
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use MouseButton::{Left, Right};
  use MouseEventKind::{Down, Drag, Moved, ScrollDown, Up};
  use crossterm::event::KeyModifiers;

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
        let Route { pane, region, focus, mouse } = route;
        (pane, region, focus, mouse.column, mouse.row)
      });
      assert_eq!(got, expected, "{mouse:?} {capture:?}");
    }
  }

  #[test]
  fn press_captures_until_its_release() {
    let mut pointer = Pointer::default();
    let mut feed = |kind, column| {
      let route = pointer.route(&PANES, 1, event(kind, column, 10));
      (route.map(|route| route.pane), pointer.capture)
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
      (Moved, 30, (Some(1), None)),
      (Down(Left), 90, (Some(2), Some(held(2, Region::Page)))),
    ];
    for (step, (kind, column, expected)) in cases.into_iter().enumerate() {
      assert_eq!(feed(kind, column), expected, "step {step}: {kind:?}");
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
