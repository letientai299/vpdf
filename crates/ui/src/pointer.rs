//! Mouse routing and what the pointer rests on.

use crate::split::Rect;
use crate::text::PageId;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::time::{Duration, Instant};
use vpdf_terminal::terminal::Shape;

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
  /// A divider or panel border.
  Edge,
  /// The drawn `K` or hover preview.
  Preview,
  /// The context menu, over the page.
  Menu,
  /// A region capture's action bar.
  Actions,
}

impl Region {
  /// Drawn over the page.
  pub fn popup(self) -> bool {
    matches!(self, Self::Preview | Self::Menu | Self::Actions)
  }
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
  /// Cells the drawn preview covers.
  pub preview: Option<Rect>,
  /// Cells the context menu covers.
  pub menu: Option<Rect>,
  /// Cells the capture's action bar covers.
  pub actions: Option<Rect>,
}

impl Pane {
  /// Region at a cell of this pane.
  fn region(&self, column: u16, row: u16) -> Region {
    let page = if self.overview { Region::Overview } else { Region::Page };
    // Topmost first.
    [
      Region::Status,
      Region::Overlay,
      Region::Menu,
      Region::Actions,
      Region::Preview,
      Region::Panel,
    ]
    .into_iter()
    .find(|region| self.area(*region).contains(column, row))
    .unwrap_or(page)
  }

  /// Cells of `region`, relative to the pane.
  pub fn area(&self, region: Region) -> Rect {
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
      Region::Preview => self.preview.unwrap_or_default(),
      Region::Menu => self.menu.unwrap_or_default(),
      Region::Actions => self.actions.unwrap_or_default(),
      Region::Edge => Rect::default(),
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
    /// Not an icon: presses select text.
    text: bool,
  },
  /// Index into the panel's rows.
  Row(usize),
  /// A panel row without a destination.
  Label(usize),
  /// An overview thumbnail.
  Thumbnail(PageId),
  /// The drawn preview; link previews follow clicks.
  Preview {
    follows: bool,
  },
  /// An overlay's item, if any, by index.
  Item(List, Option<usize>),
  /// A recent card row.
  Card(u16),
  /// A boundary drags resize.
  Edge {
    rows: bool,
  },
  /// A clickable status item.
  Chrome,
  /// The page's scrollbar.
  Scrollbar,
}

/// Overlays whose items the pointer picks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum List {
  /// The context menu.
  Menu,
  /// A capture's action bar.
  Actions,
  /// `f`/`F` link labels.
  Hints,
  /// Key clues that name keys.
  Clues,
  /// The prompt's completions.
  Completions,
}

/// Drawn items: pane cells and index.
#[derive(Clone, Debug, Default)]
pub struct Items(Vec<(Rect, usize)>);

impl Items {
  pub fn push(&mut self, cells: Rect, index: usize) {
    self.0.push((cells, index));
  }

  /// The topmost item at a pane cell.
  pub fn at(&self, column: u16, row: u16) -> Option<usize> {
    // Later items draw on top.
    let item = self.0.iter().rfind(|(cells, _)| cells.contains(column, row));
    item.map(|(_, index)| *index)
  }
}

impl Hit {
  pub fn item(self, list: List) -> Option<usize> {
    match self {
      Self::Item(hovered, index) if hovered == list => index,
      _ => None,
    }
  }

  pub fn shape(self) -> Shape {
    match self {
      Self::Nothing
      | Self::Preview { follows: false }
      | Self::Item(_, None)
      | Self::Card(_)
      | Self::Label(_)
      | Self::Scrollbar => Shape::Default,
      Self::Text | Self::Note { text: true, .. } => Shape::Text,
      Self::Link(..)
      | Self::Note { text: false, .. }
      | Self::Row(_)
      | Self::Thumbnail(_)
      | Self::Preview { follows: true }
      | Self::Item(_, Some(_))
      | Self::Chrome => Shape::Pointer,
      Self::Edge { rows: false } => Shape::ColResize,
      Self::Edge { rows: true } => Shape::RowResize,
    }
  }
}

/// Pane, region, and hit under the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Hover {
  pub pane: u32,
  pub region: Region,
  pub hit: Hit,
  /// The page link under it, popups too.
  pub beneath: Option<(PageId, usize)>,
}

impl Hover {
  /// What a frame shows of it.
  pub fn shown(self) -> (Shape, Option<(u32, Hit)>) {
    let inert = matches!(
      self.hit,
      Hit::Nothing | Hit::Text | Hit::Preview { .. } | Hit::Item(_, None)
    );
    (self.hit.shape(), (!inert).then_some((self.pane, self.hit)))
  }
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

/// Rows one wheel turn scrolls.
pub const WHEEL_ROWS: u8 = 3;

/// A wheel turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Wheel {
  Up,
  Down,
  Left,
  Right,
}

impl Wheel {
  pub fn of(kind: MouseEventKind) -> Option<Self> {
    match kind {
      MouseEventKind::ScrollUp => Some(Self::Up),
      MouseEventKind::ScrollDown => Some(Self::Down),
      MouseEventKind::ScrollLeft => Some(Self::Left),
      MouseEventKind::ScrollRight => Some(Self::Right),
      _ => None,
    }
  }

  /// Down and right are forward.
  pub fn forward(self) -> bool {
    matches!(self, Self::Down | Self::Right)
  }

  pub fn sideways(self) -> bool {
    matches!(self, Self::Left | Self::Right)
  }
}

/// Presses and wheel turns cancel keys.
pub fn interrupts(kind: MouseEventKind) -> bool {
  !matches!(
    kind,
    MouseEventKind::Drag(_) | MouseEventKind::Up(_) | MouseEventKind::Moved
  )
}

/// Wheel turns this close, at one spot, form one gesture.
const GESTURE: Duration = Duration::from_millis(250);

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
  /// Alt-clicks and plain clicks run apart.
  alt: bool,
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
  /// The last wheel turn; motion and presses end its gesture.
  turn: Option<Turn>,
}

/// Where a wheel turned: a cell, and the pixel under mode 1016.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
  column: u16,
  row: u16,
  pixel: Option<Pixel>,
}

/// Where and when a wheel turned.
#[derive(Clone, Copy, Debug)]
struct Turn {
  spot: Spot,
  at: Instant,
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
    // One button at a time.
    if self.busy(mouse) {
      return None;
    }
    let capture = self.capture();
    let held = capture.map(|capture| capture.button);
    if let MouseEventKind::Drag(button) = mouse.kind
      && held == Some(button)
      && let Some(press) = &mut self.press
    {
      press.dragged |= !press.near(mouse.column, mouse.row, pixel, drag);
      if !press.dragged {
        return None;
      }
    }
    self.place(mouse, pixel);
    let route = route(panes, focused, capture, mouse);
    match mouse.kind {
      MouseEventKind::Down(button) => {
        let on = route.map(|route| (route.pane, route.region));
        let alt = mouse.modifiers.contains(KeyModifiers::ALT);
        self.press(button, alt, on, mouse, pixel, now);
      }
      MouseEventKind::Up(button) if held == Some(button) => {
        self.hold(false);
      }
      // The view moved under the next press.
      kind if Wheel::of(kind).is_some() && held.is_none() => self.press = None,
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

  /// Captures a boundary press; its clicks.
  pub fn press_edge(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> u8 {
    self.place(mouse, pixel);
    let on = Some((0, Region::Edge));
    self.press(MouseButton::Left, false, on, mouse, pixel, now);
    self.press.map_or(1, |press| press.clicks)
  }

  /// Moves the spot without a hover test.
  fn place(&mut self, mouse: MouseEvent, pixel: Option<Pixel>) {
    self.spot =
      Some((MouseEvent { kind: MouseEventKind::Moved, ..mouse }, pixel));
  }

  /// Whether the press passed the threshold.
  pub fn dragged(&self) -> bool {
    self.press.is_some_and(|press| press.dragged)
  }

  /// Starts a press or extends its run.
  fn press(
    &mut self,
    button: MouseButton,
    alt: bool,
    on: Option<(u32, Region)>,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) {
    self.turn = None;
    let again = self
      .repeated(button, mouse, pixel, now)
      .filter(|press| press.alt == alt && press.on == on);
    self.press = Some(Press {
      button,
      alt,
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

  /// The last press when one of `button` at `mouse` continues its run.
  fn repeated(
    &self,
    button: MouseButton,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Option<Press> {
    self.press.filter(|press| {
      press.button == button
        && !press.dragged
        && press.near(mouse.column, mouse.row, pixel, click)
        && now.saturating_duration_since(press.at) < CLICK
    })
  }

  /// A repeated press, in any pane; it extends the run as a press would.
  pub fn repeats(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> bool {
    let MouseEventKind::Down(button) = mouse.kind else { return false };
    if self.repeated(button, mouse, pixel, now).is_none() {
      return false;
    }
    if let Some(press) = &mut self.press {
      press.at = now;
    }
    true
  }

  /// Whether a wheel turn continues the last one's gesture: at the same
  /// spot, with no motion or press between, and soon after.
  pub fn turn(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> bool {
    let spot = Spot { column: mouse.column, row: mouse.row, pixel };
    let continues = self.turn.is_some_and(|last| {
      last.spot == spot && now.saturating_duration_since(last.at) < GESTURE
    });
    self.turn = Some(Turn { spot, at: now });
    continues
  }

  /// When the last wheel gesture ends, unless motion ended it.
  pub fn turn_end(&self) -> Option<Instant> {
    self.turn.map(|turn| turn.at + GESTURE)
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
    self.turn = None;
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

  /// Whether another button holds the pointer.
  pub fn busy(&self, mouse: MouseEvent) -> bool {
    let MouseEventKind::Down(button) = mouse.kind else { return false };
    self.capture().is_some_and(|capture| capture.button != button)
  }

  /// Whether a press holds the pointer.
  pub fn holding(&self) -> bool {
    self.capture().is_some()
  }

  /// The pane a held press drags.
  pub fn held_pane(&self) -> Option<u32> {
    self.capture().map(|capture| capture.pane)
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
mod tests;
