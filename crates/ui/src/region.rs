use crate::input::gesture::Sweep;
use crate::keys::Action;
use crate::layout::Layout;
use crate::paint::Frame;
use crate::pointer::Items;
use crate::split::Rect;
use crate::theme;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;
use vpdf_model::scene::{Draw, Layer, Op};
use vpdf_model::style::Role;
use vpdf_model::text::{Bounds, PageId};
use vpdf_reader::motion::Motion;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureId(u64);

impl CaptureId {
  pub fn new(value: u64) -> Self {
    Self(value)
  }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
  pub id: CaptureId,
  pub page: PageId,
  pub bounds: Bounds,
}

pub struct Region {
  snapshot: Snapshot,
  state: State,
}

enum State {
  Selected,
  Drawing(Sweep),
  Actions(Bar),
}

struct Bar {
  sweep: Sweep,
  action: Operation,
  status: String,
  cells: Option<BarCells>,
}

struct BarCells {
  area: Rect,
  items: Items,
}

type Spans = Vec<(Range<usize>, usize)>;

pub const DRAW_HINT: &str =
  "hjkl resize; HJKL move; o swaps; Enter captures; Esc cancels";

pub const BAR_HINT: &str =
  "h/l choose; HJKL move; R edits; Enter runs; Esc closes";

const ACTIONS: [(&str, Operation); 4] = [
  ("Copy image", Operation::Image),
  ("Copy text", Operation::Text),
  ("Save PNG", Operation::Save),
  ("OCR text", Operation::Ocr),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
  Image,
  Text,
  Save,
  Ocr,
}

impl Operation {
  pub fn at(index: usize) -> Option<Self> {
    ACTIONS.get(index).map(|(_, operation)| *operation)
  }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Transition {
  Stay,
  Move(i8, i8),
  Shift(i8, i8),
  Confirm,
  Edit,
  Run(Operation),
  Close,
  Clear,
  Command,
  Transfer,
  Unhandled,
}

enum Route {
  Select(bool),
  Run,
  Close,
  Command,
  Transfer,
  Ignore,
}

fn route(action: Action) -> Route {
  match action {
    Action::Motion(Motion::Left | Motion::Up) => Route::Select(false),
    Action::Motion(Motion::Right | Motion::Down) => Route::Select(true),
    Action::Follow => Route::Run,
    Action::Quit | Action::Cancel | Action::Region | Action::RegionStart => {
      Route::Close
    }
    Action::Command => Route::Command,
    _ if crate::input::transfers_overlay(action) => Route::Transfer,
    _ => Route::Ignore,
  }
}

impl Region {
  pub fn selected(snapshot: Snapshot) -> Self {
    Self { snapshot, state: State::Selected }
  }

  pub fn open(snapshot: Snapshot) -> Self {
    let mut region = Self::selected(snapshot);
    region.state = State::Actions(Bar::new(region.sweep()));
    region
  }

  pub fn drawing(id: CaptureId, page: PageId, from: (f32, f32)) -> Self {
    let (x, y) = from;
    Self {
      snapshot: Snapshot {
        id,
        page,
        bounds: Bounds { x0: x, y0: y, x1: x, y1: y },
      },
      state: State::Drawing(Sweep::new(page, from)),
    }
  }

  pub fn draft(&self) -> Option<Sweep> {
    match self.state {
      State::Drawing(sweep) => Some(sweep),
      _ => None,
    }
  }

  pub fn move_to(&mut self, point: (f32, f32), shown: Bounds) {
    if let State::Drawing(sweep) = &mut self.state {
      sweep.move_to(point, shown);
      if let Some(bounds) = sweep.bounds() {
        self.snapshot.bounds = bounds;
      }
    }
  }

  fn bar(&self) -> Option<&Bar> {
    match &self.state {
      State::Actions(bar) => Some(bar),
      _ => None,
    }
  }

  fn bar_mut(&mut self) -> Option<&mut Bar> {
    match &mut self.state {
      State::Actions(bar) => Some(bar),
      _ => None,
    }
  }

  pub fn active(&self) -> bool {
    !matches!(self.state, State::Selected)
  }

  pub fn snapshot(&self) -> &Snapshot {
    &self.snapshot
  }

  pub fn is_bar(&self) -> bool {
    self.bar().is_some()
  }

  pub fn action(&self) -> Option<Operation> {
    self.bar().map(|bar| bar.action)
  }

  pub fn status(&self) -> Option<&str> {
    self.bar().map(|bar| bar.status.as_str())
  }

  pub fn request(&mut self, id: CaptureId) -> Snapshot {
    if self.bar().is_some() {
      self.snapshot.id = id;
    }
    Snapshot { id, ..self.snapshot.clone() }
  }

  pub fn update(&mut self, id: CaptureId, message: &str) -> bool {
    if self.active() && self.snapshot.id != id {
      return false;
    }
    if let Some(bar) = self.bar_mut() {
      message.clone_into(&mut bar.status);
    }
    true
  }

  fn sweep(&self) -> Sweep {
    if let Some(bar) = self.bar() {
      return bar.sweep;
    }
    self.draft().unwrap_or_else(|| {
      let bounds = self.snapshot.bounds;
      let mut sweep = Sweep::new(self.snapshot.page, (bounds.x0, bounds.y0));
      sweep.move_to((bounds.x1, bounds.y1), bounds);
      sweep
    })
  }

  pub fn confirm(&mut self, id: CaptureId, shown: Bounds) -> bool {
    let mut sweep = self.sweep();
    sweep.clip(shown);
    let Some(bounds) = sweep.bounds() else { return false };
    self.snapshot = Snapshot { id, page: sweep.page(), bounds };
    self.state = State::Actions(Bar::new(sweep));
    true
  }

  pub fn edit(&mut self, id: CaptureId) {
    self.state = State::Drawing(self.sweep());
    self.snapshot.id = id;
  }

  pub fn shift(&mut self, delta: (f32, f32), shown: Bounds, id: CaptureId) {
    let mut sweep = self.sweep();
    sweep.shift(delta, shown);
    if let State::Drawing(draft) = &mut self.state {
      *draft = sweep;
    }
    if let Some(bounds) = sweep.bounds()
      && bounds != self.snapshot.bounds
    {
      self.snapshot.bounds = bounds;
      self.snapshot.id = id;
      if let Some(bar) = self.bar_mut() {
        bar.sweep = sweep;
        bar.status.clear();
        bar.cells = None;
      }
    }
  }

  pub fn allows(&self, action: Action) -> bool {
    !self.active()
      || matches!(action, Action::RegionShift(..))
      || (self.draft().is_some() && action == Action::Swap)
      || !matches!(route(action), Route::Ignore)
  }

  pub fn input(&mut self, action: Action) -> Transition {
    if self.is_bar() && action == Action::RegionStart {
      return Transition::Edit;
    }
    if self.active()
      && let Action::RegionShift(dx, dy) = action
    {
      return Transition::Shift(dx, dy);
    }
    if let State::Drawing(sweep) = &mut self.state {
      return match action {
        Action::Swap => {
          sweep.swap();
          Transition::Stay
        }
        Action::Motion(Motion::Left) => Transition::Move(-1, 0),
        Action::Motion(Motion::Right) => Transition::Move(1, 0),
        Action::Motion(Motion::Up) => Transition::Move(0, -1),
        Action::Motion(Motion::Down) => Transition::Move(0, 1),
        Action::Follow => Transition::Confirm,
        _ => match route(action) {
          Route::Close => Transition::Close,
          Route::Command => Transition::Command,
          Route::Transfer => Transition::Transfer,
          _ => Transition::Stay,
        },
      };
    }
    let Some(bar) = self.bar_mut() else {
      return if matches!(
        action,
        Action::Figure(_)
          | Action::FigureFit
          | Action::Region
          | Action::RegionStart
      ) {
        Transition::Unhandled
      } else {
        Transition::Clear
      };
    };
    match route(action) {
      Route::Select(false) => {
        bar.action = ACTIONS[(bar.action as usize + 3) % ACTIONS.len()].1;
        Transition::Stay
      }
      Route::Select(true) => {
        bar.action = ACTIONS[(bar.action as usize + 1) % ACTIONS.len()].1;
        Transition::Stay
      }
      Route::Run => Transition::Run(bar.action),
      Route::Close => Transition::Close,
      Route::Command => Transition::Command,
      Route::Transfer => Transition::Transfer,
      Route::Ignore => Transition::Stay,
    }
  }

  pub fn cells(&self) -> Option<Rect> {
    self.bar()?.cells.as_ref().map(|cells| cells.area)
  }

  pub fn items(&self) -> Items {
    self
      .bar()
      .and_then(|bar| bar.cells.as_ref())
      .map_or_default(|cells| cells.items.clone())
  }

  pub fn select(&mut self, index: usize) {
    if let Some(bar) = self.bar_mut()
      && let Some(operation) = Operation::at(index)
    {
      bar.action = operation;
    }
  }

  /// Records the drawn bar's cells.
  fn place_bar(&mut self, row: usize, width: usize, spans: Spans, left: u16) {
    let column = |offset: usize| {
      u16::try_from(usize::from(left) + offset.min(width)).unwrap_or(u16::MAX)
    };
    let row = u16::try_from(row).unwrap_or(u16::MAX);
    let cells = |span: Range<usize>| {
      let (start, end) = (column(span.start), column(span.end));
      Rect { column: start, row, columns: end - start, rows: 1 }
    };
    let mut items = Items::default();
    for (span, index) in spans {
      items.push(cells(span), index);
    }
    let area = cells(0..width);
    if let Some(bar) = self.bar_mut() {
      bar.cells = Some(BarCells { area, items });
    }
  }
}

impl Bar {
  fn new(sweep: Sweep) -> Self {
    Self { sweep, action: Operation::Image, status: String::new(), cells: None }
  }
}

pub struct Canvas<'a> {
  pub layout: &'a Layout,
  pub scroll: f32,
  pub frame: Frame,
}

impl Canvas<'_> {
  pub fn scene<'a>(
    &self,
    region: Option<&mut Region>,
    sweep: Option<(PageId, Bounds)>,
    snap_to_cells: bool,
    scene: &mut impl Draw<'a>,
  ) {
    let active = if sweep.is_none() {
      region.as_ref().and_then(|region| region.draft()).map(|draft| {
        let (x, y) = draft.endpoint();
        self
          .layout
          .project(
            draft.page(),
            Bounds { x0: x, y0: y, x1: x, y1: y },
            self.scroll,
          )
          .centre()
      })
    } else {
      None
    };
    let sweep = sweep.or_else(|| {
      let draft = region.as_ref()?.draft()?;
      Some((draft.page(), draft.bounds()?))
    });
    if let Some((page, bounds)) = sweep {
      let mut rect = self.layout.project(page, bounds, self.scroll);
      let active = active.map(|(x, y)| {
        let (cx, cy) = rect.centre();
        (x >= cx, y >= cy)
      });
      // Quirk overlays cache by size.
      if snap_to_cells {
        let (width, height) = (
          f32::from(self.layout.screen().cell_width),
          f32::from(self.layout.screen().cell_height),
        );
        rect = Bounds {
          x0: (rect.x0 / width).floor() * width,
          y0: (rect.y0 / height).floor() * height,
          x1: (rect.x1 / width).ceil() * width,
          y1: (rect.y1 / height).ceil() * height,
        };
      }
      self.outline(scene, rect, active);
    }
    let Some(region) = region else { return };
    let Some(bar) = region.bar() else { return };
    let snapshot = &region.snapshot;
    let (action, status) = (&bar.action, &bar.status);
    let (page, action, status) = (snapshot.page, *action, status.clone());
    let rect = self.layout.project(page, snapshot.bounds, self.scroll);
    self.outline(scene, rect, None);
    let screen = self.frame.screen;
    let (mut label, mut items) = bar_label(page, action);
    if label.chars().count() > usize::from(screen.columns) {
      let name = ACTIONS[action as usize].0;
      label = format!("[{name}] p{}", page.0 + 1);
      items = vec![(0..name.len() + 2, action as usize)];
    }
    if !status.is_empty() {
      label.push_str(" │ ");
      label.push_str(&status);
    }
    let text = vpdf_model::text::clean(&label, usize::from(screen.columns));
    let last = usize::from(screen.rows.saturating_sub(2));
    let row = bar_row(rect, f32::from(screen.cell_height), last);
    let offset = usize::from(screen.columns).saturating_sub(text.width()) / 2;
    let column = usize::from(self.frame.left) + offset;
    // Graphics fill the row; text stops.
    let width =
      if self.frame.text { text.width() } else { usize::from(screen.columns) };
    let left = if self.frame.text {
      u16::try_from(column).unwrap_or(u16::MAX)
    } else {
      for (span, _) in &mut items {
        *span = span.start + offset..span.end + offset;
      }
      self.frame.left
    };
    region.place_bar(row, width, items, left);
    if let Some(cells) = region.cells().filter(|_| !self.frame.text) {
      let backdrop =
        crate::paint::cell_bounds(self.frame.screen, self.frame.left, cells);
      scene.fill(
        backdrop,
        theme::BADGE_FILL,
        Layer::BelowText,
        screen,
        self.frame.left,
      );
    }
    crate::paint::move_to(scene, self.frame.at, row, column);
    scene.push(Op::Style(Role::Selected));
    scene.text(&text);
    scene.push(Op::Style(Role::Reset));
  }

  /// Dims around a rect; outlines it.
  fn outline<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    rect: Bounds,
    active: Option<(bool, bool)>,
  ) {
    let screen = self.frame.screen;
    if !self.frame.text {
      let w = screen.width();
      let h = screen.height();
      let x0 = rect.x0.clamp(0.0, w);
      let x1 = rect.x1.clamp(x0, w);
      let y0 = rect.y0.clamp(0.0, h);
      let y1 = rect.y1.clamp(y0, h);
      for bounds in [
        Bounds { x0: 0.0, y0: 0.0, x1: w, y1: y0 },
        Bounds { x0: 0.0, y0: y1, x1: w, y1: h },
        Bounds { x0: 0.0, y0, x1: x0, y1 },
        Bounds { x0: x1, y0, x1: w, y1 },
      ] {
        scene.fill(
          bounds,
          theme::REGION_VEIL,
          Layer::BelowText,
          screen,
          self.frame.left,
        );
      }
      for bounds in frame_edges(Bounds { x0, y0, x1, y1 }, (w, h), active) {
        scene.fill(
          bounds,
          theme::REGION_EDGE,
          Layer::AboveText,
          screen,
          self.frame.left,
        );
      }
    }
  }
}

/// Bar text and item spans; `action` bracketed.
fn bar_label(page: PageId, action: Operation) -> (String, Spans) {
  let mut label = format!("p{}", page.0 + 1);
  let mut items = Vec::with_capacity(ACTIONS.len());
  for (index, (name, operation)) in ACTIONS.iter().enumerate() {
    label.push_str(if index == 0 { " " } else { " · " });
    let start = label.chars().count();
    if *operation == action {
      label.extend(["[", name, "]"]);
    } else {
      label.push_str(name);
    }
    items.push((start..label.chars().count(), index));
  }
  (label, items)
}

/// Active corner edges use thicker bands.
fn frame_edges(
  rect: Bounds,
  (w, h): (f32, f32),
  active: Option<(bool, bool)>,
) -> [Bounds; 4] {
  // Overlays round out to whole pixels.
  let (x0, y0) = (rect.x0.floor(), rect.y0.floor());
  let (x1, y1) = (rect.x1.ceil(), rect.y1.ceil());
  // U26: outside where room allows.
  let width = |selected| if selected { 4.0 } else { 2.0 };
  let top = band(y0, -1.0, width(active.is_some_and(|(_, bottom)| !bottom)), h);
  let bottom =
    band(y1, 1.0, width(active.is_some_and(|(_, bottom)| bottom)), h);
  let left = band(x0, -1.0, width(active.is_some_and(|(right, _)| !right)), w);
  let right = band(x1, 1.0, width(active.is_some_and(|(right, _)| right)), w);
  [
    Bounds { x0: left.0, x1: right.1, y0: top.0, y1: top.1 },
    Bounds { x0: left.0, x1: right.1, y0: bottom.0, y1: bottom.1 },
    Bounds { x0: left.0, x1: left.1, y0, y1 },
    Bounds { x0: right.0, x1: right.1, y0, y1 },
  ]
}

/// Bands extend outward where they fit.
fn band(edge: f32, dir: f32, width: f32, limit: f32) -> (f32, f32) {
  let outside = edge + width * dir;
  let far = if (0.0..=limit).contains(&outside) {
    outside
  } else {
    (edge - width * dir).clamp(0.0, limit)
  };
  (edge.min(far), edge.max(far))
}

/// The bar's row: below the frame, else above.
fn bar_row(rect: Bounds, cell: f32, last: usize) -> usize {
  let below = ((rect.y1.ceil() + 2.0) / cell).ceil() as usize;
  let above = ((rect.y0.floor() - 2.0) / cell).floor() as isize - 1;
  match usize::try_from(above) {
    _ if below <= last => below,
    Ok(above) => above,
    Err(_) => last,
  }
}
#[cfg(test)]
#[path = "../../../tests/support/region.rs"]
mod fixture;

#[cfg(test)]
mod tests {
  use super::fixture::snapshot;
  use super::*;

  #[test]
  fn confirmed_sweeps_keep_direction() {
    let shown = Bounds { x0: 0.0, y0: 0.0, x1: 200.0, y1: 200.0 };
    let mut region =
      Region::drawing(CaptureId::new(1), PageId(0), (10.0, 20.0));
    region.move_to((60.0, 80.0), shown);
    region.input(Action::Swap);
    let mut expected = region.draft().unwrap();
    assert!(region.confirm(CaptureId::new(2), shown));
    region.shift((7.0, 9.0), shown, CaptureId::new(3));
    expected.shift((7.0, 9.0), shown);
    region.edit(CaptureId::new(4));
    assert_eq!(region.draft(), Some(expected));
  }

  #[test]
  fn confirmation_clips_page_bounds() {
    let shown = Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 100.0 };
    let mut region =
      Region::drawing(CaptureId::new(1), PageId(0), (-10.0, -10.0));
    region.move_to(
      (110.0, 110.0),
      Bounds { x0: -10.0, y0: -10.0, x1: 110.0, y1: 110.0 },
    );
    assert!(region.confirm(CaptureId::new(2), shown));
    assert_eq!(region.snapshot().bounds, shown);
    region.edit(CaptureId::new(3));
    assert_eq!(region.draft().unwrap().endpoint(), (100.0, 100.0));
    region.move_to((0.0, 0.0), shown);
    assert!(!region.confirm(CaptureId::new(4), shown));
    assert!(region.draft().is_some());
  }

  #[test]
  fn drafts_reverse_and_clamp() {
    let mut region =
      Region::drawing(CaptureId::new(1), PageId(0), (50.0, 50.0));
    let shown = Bounds { x0: 10.0, y0: 10.0, x1: 90.0, y1: 90.0 };
    assert!(region.active());
    assert!(!region.is_bar());
    assert!(region.draft().unwrap().bounds().is_none());
    region.move_to((-100.0, 200.0), shown);
    assert_eq!(region.draft().unwrap().endpoint(), (10.0, 90.0));
    assert_eq!(
      region.snapshot().bounds,
      Bounds { x0: 10.0, y0: 50.0, x1: 50.0, y1: 90.0 }
    );
    region.move_to((80.0, 20.0), shown);
    assert_eq!(
      region.snapshot().bounds,
      Bounds { x0: 50.0, y0: 20.0, x1: 80.0, y1: 50.0 }
    );
    assert_eq!(
      region.input(Action::Motion(Motion::Left)),
      Transition::Move(-1, 0)
    );
    assert_eq!(region.input(Action::Follow), Transition::Confirm);
    assert_eq!(region.input(Action::Cancel), Transition::Close);
    assert_eq!(region.input(Action::Swap), Transition::Stay);
    assert_eq!(region.draft().unwrap().endpoint(), (50.0, 50.0));
    region.move_to((80.0, 50.0), shown);
    assert!(region.draft().unwrap().bounds().is_none());
  }

  #[test]
  fn shifting_preserves_size_and_identity() {
    let shown = Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 100.0 };
    let mut draft = Region::drawing(CaptureId::new(1), PageId(0), (10.0, 20.0));
    draft.move_to((60.0, 80.0), shown);
    for mut region in [draft, Region::open(snapshot(1))] {
      let original = region.snapshot().bounds;
      region.shift((1000.0, 1000.0), shown, CaptureId::new(2));
      let moved = region.snapshot().bounds;
      assert_eq!(
        (moved.width(), moved.height()),
        (original.width(), original.height())
      );
      assert_eq!((moved.x1, moved.y1), (shown.x1, shown.y1));
      assert_eq!(region.snapshot().id, CaptureId::new(2));
      if region.is_bar() {
        assert!(!region.update(CaptureId::new(1), "Stale capture"));
      }
      region.shift((-1000.0, -1000.0), shown, CaptureId::new(3));
      let moved = region.snapshot().bounds;
      assert_eq!(
        (moved.width(), moved.height()),
        (original.width(), original.height())
      );
      assert_eq!((moved.x0, moved.y0), (shown.x0, shown.y0));
      assert_eq!(
        region.input(Action::RegionShift(1, 0)),
        Transition::Shift(1, 0)
      );
    }
  }

  #[test]
  fn drafts_emphasize_active_corners() {
    let widths = |ops: &[Op<'_>]| {
      ops
        .iter()
        .filter_map(|op| match op {
          Op::Fill(fill) if fill.color() == theme::REGION_EDGE => {
            Some(fill.rect().width().min(fill.rect().height()))
          }
          _ => None,
        })
        .collect::<Vec<_>>()
    };
    let veil = |ops: &[Op<'_>]| {
      ops
        .iter()
        .filter_map(|op| match op {
          Op::Fill(fill) if fill.color() == theme::REGION_VEIL => Some(**fill),
          _ => None,
        })
        .collect::<Vec<_>>()
    };
    for rotation in 0..4 {
      let mut layout = fixture::layout();
      for _ in 0..rotation {
        layout.rotate();
      }
      layout.reflow([Bounds { x0: 0.0, y0: 0.0, x1: 160.0, y1: 160.0 }]);
      let canvas = fixture::canvas(&layout);
      let (page, bounds) = fixture::sweep();
      let mut region =
        Region::drawing(CaptureId::new(1), page, (bounds.x0, bounds.y0));
      region.move_to((bounds.x1, bounds.y1), bounds);
      for snap in [false, true] {
        let mut before = Vec::new();
        canvas.scene(Some(&mut region), None, snap, &mut before);
        region.input(Action::Swap);
        let mut after = Vec::new();
        canvas.scene(Some(&mut region), None, snap, &mut after);
        assert_eq!(veil(&before), veil(&after));
        let before = widths(&before);
        let after = widths(&after);
        assert_eq!(before.len(), 4);
        assert_eq!(after.len(), 4);
        for (a, b) in before.into_iter().zip(after) {
          assert_eq!(a + b, 6.0);
          assert_ne!(a, b);
        }
      }
    }
  }

  #[test]
  fn snapping_keeps_active_direction() {
    let layout = fixture::layout();
    let canvas = fixture::canvas(&layout);
    let bounds = Bounds { x0: 9.0, y0: 17.0, x1: 10.0, y1: 18.0 };
    let mut region =
      Region::drawing(CaptureId::new(1), PageId(0), (bounds.x0, bounds.y0));
    region.move_to((bounds.x1, bounds.y1), bounds);
    let mut output = Vec::new();
    canvas.scene(Some(&mut region), None, true, &mut output);
    let edges = output
      .iter()
      .filter_map(|op| match op {
        Op::Fill(fill) if fill.color() == theme::REGION_EDGE => {
          Some(fill.rect())
        }
        _ => None,
      })
      .collect::<Vec<_>>();
    assert_eq!(edges.len(), 4);
    assert_eq!(
      [
        edges[0].height(),
        edges[1].height(),
        edges[2].width(),
        edges[3].width()
      ],
      [2.0, 4.0, 2.0, 4.0]
    );
  }

  #[test]
  fn active_borders_stay_within_screen() {
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 50.0 };
    for (point, expected) in [
      ((false, false), [4.0, 2.0, 4.0, 2.0]),
      ((true, false), [4.0, 2.0, 2.0, 4.0]),
      ((false, true), [2.0, 4.0, 4.0, 2.0]),
      ((true, true), [2.0, 4.0, 2.0, 4.0]),
    ] {
      let edges = frame_edges(rect, (100.0, 50.0), Some(point));
      for (edge, width) in edges.into_iter().zip(expected) {
        assert_eq!(edge.width().min(edge.height()), width);
        assert!(edge.x0 >= 0.0 && edge.y0 >= 0.0);
        assert!(edge.x1 <= 100.0 && edge.y1 <= 50.0);
      }
    }
  }

  #[test]
  fn requests_reject_stale_status() {
    let mut region = Region::open(snapshot(1));
    let request = region.request(CaptureId::new(2));
    assert_eq!(request.id, CaptureId::new(2));
    assert_eq!(region.snapshot().id, request.id);
    assert!(region.update(request.id, "Running"));
    assert!(!region.update(CaptureId::new(1), "Old result"));
    assert_eq!(region.status(), Some("Running"));
    let mut selected = Region::selected(snapshot(3));
    let request = selected.request(CaptureId::new(4));
    assert_eq!(request.id, CaptureId::new(4));
    assert_eq!(selected.snapshot().id, CaptureId::new(3));
    assert!(selected.update(request.id, "Copied"));
    assert_eq!(selected.status(), None);
  }

  #[test]
  fn input_keeps_selection_separate() {
    let mut region = Region::open(snapshot(1));
    assert_eq!(region.input(Action::Motion(Motion::Left)), Transition::Stay);
    assert_eq!(region.action(), Some(Operation::Ocr));
    assert_eq!(region.input(Action::Follow), Transition::Run(Operation::Ocr));
    region.input(Action::Motion(Motion::Down));
    assert_eq!(region.action(), Some(Operation::Image));
    region.select(2);
    region.select(4);
    assert_eq!(region.action(), Some(Operation::Save));
    assert_eq!(region.input(Action::Command), Transition::Command);
    assert_eq!(region.input(Action::Region), Transition::Close);
    let mut selected = Region::selected(snapshot(2));
    for action in [Action::Figure(true), Action::FigureFit, Action::Region] {
      assert_eq!(selected.input(action), Transition::Unhandled);
    }
    assert_eq!(selected.input(Action::Follow), Transition::Clear);
    assert_eq!(selected.action(), None);
  }

  #[test]
  fn recorded_regions_keep_hits() {
    use vpdf_model::scene::Scene;
    let layout = fixture::layout();
    let canvas = fixture::canvas(&layout);
    let sweep = Some(fixture::sweep());
    for crop_quirk in [false, true] {
      let mut recorded = Region::open(snapshot(1));
      let mut scene = Scene::new();
      canvas.scene(Some(&mut recorded), sweep, crop_quirk, &mut scene);
      let area = recorded.cells().unwrap();
      assert_eq!(area.column, 3);
      assert_eq!(recorded.items().at(area.column + 2, area.row), Some(0));
    }
  }

  #[test]
  fn bars_center_text_and_hits() {
    for (columns, status) in
      [(1, ""), (20, ""), (80, ""), (81, ""), (80, "Copied"), (80, "已复制")]
    {
      for text in [false, true] {
        let layout = fixture::layout();
        let mut canvas = fixture::canvas(&layout);
        canvas.frame.screen.columns = columns;
        canvas.frame.text = text;
        let mut region = Region::open(snapshot(1));
        region.update(CaptureId::new(1), status);
        let mut output = Vec::new();
        canvas.scene(Some(&mut region), None, false, &mut output);
        let label = crate::testing::text(&output);
        let offset = (usize::from(columns) - label.width()) / 2;
        let column = usize::from(canvas.frame.left) + offset;
        let area = region.cells().unwrap();
        assert!(output.iter().any(|op| matches!(op,
          Op::Move { row, column: at }
            if *row == usize::from(area.row + canvas.frame.at.row)
              && *at == column + usize::from(canvas.frame.at.column)
        )));
        let start = if columns < 50 { 0 } else { 3 };
        let hit = u16::try_from(column + start).unwrap();
        assert_eq!(region.items().at(hit, area.row), Some(0));
        assert_eq!(region.items().at(hit - 1, area.row), None);
        assert_eq!(area.column, if text { column as u16 } else { 3 });
        assert_eq!(
          area.columns as usize,
          if text { label.width() } else { columns as usize }
        );
      }
    }
  }

  #[test]
  fn narrow_bar_tracks_cells() {
    use vpdf_model::geometry::At;

    let screen = crate::testing::screen::cells(20, 10);
    let layout =
      Layout::new([Bounds { x0: 0.0, y0: 0.0, x1: 160.0, y1: 160.0 }], screen);
    let canvas = Canvas {
      layout: &layout,
      scroll: 0.0,
      frame: Frame { screen, at: At::default(), left: 7, text: true },
    };
    let mut region = Region::open(snapshot(1));
    region.select(2);
    assert!(region.cells().is_none());
    assert_eq!(region.items().at(7, 1), None);
    let mut output = Vec::new();
    canvas.scene(Some(&mut region), None, false, &mut output);
    let area = region.cells().unwrap();
    assert_eq!(area.column, 10);
    assert_eq!(area.columns, 13);
    assert_eq!(region.items().at(9, area.row), None);
    assert_eq!(region.items().at(10, area.row), Some(2));
    assert_eq!(region.items().at(19, area.row), Some(2));
    assert_eq!(region.items().at(20, area.row), None);
    assert!(crate::testing::text(&output).contains("[Save PNG] p1"));
  }

  /// U26: frames sit outside when room allows.
  #[test]
  fn frames_sit_outside_captures() {
    let (w, h) = (100.0, 50.0);
    let rect = |x0, y0, x1, y1| Bounds { x0, y0, x1, y1 };
    // (capture, inside: top, bottom, left, right)
    for (capture, inside) in [
      (rect(10.0, 10.0, 60.0, 40.0), [false; 4]),
      (rect(10.5, 10.5, 60.5, 40.5), [false; 4]),
      (rect(0.0, 0.0, 100.0, 50.0), [true; 4]),
      (rect(1.0, 20.0, 99.5, 50.0), [false, true, true, true]),
    ] {
      for (edge, inside) in
        frame_edges(capture, (w, h), None).into_iter().zip(inside)
      {
        let covers = edge.x0.max(capture.x0) < edge.x1.min(capture.x1)
          && edge.y0.max(capture.y0) < edge.y1.min(capture.y1);
        assert_eq!(covers, inside, "{edge:?} on {capture:?}");
        assert_eq!(edge.width().min(edge.height()), 2.0, "{edge:?}");
        let within = edge.x0 >= 0.0 && edge.y0 >= 0.0;
        assert!(within && edge.x1 <= w && edge.y1 <= h, "{edge:?}");
        // Bands touch the whole-pixel capture.
        let touches = edge.x0 <= capture.x1.ceil()
          && edge.x1 >= capture.x0.floor()
          && edge.y0 <= capture.y1.ceil()
          && edge.y1 >= capture.y0.floor();
        assert!(touches, "{edge:?} off {capture:?}");
      }
    }
  }

  /// The bar never covers the capture.
  #[test]
  fn bars_clear_captures() {
    let rect = |y0, y1| Bounds { x0: 0.0, y0, x1: 10.0, y1 };
    // (capture, row); 20 px cells, last 38
    for (capture, row) in [
      (rect(100.0, 300.0), 16),
      (rect(100.0, 770.0), 3),
      (rect(10.0, 770.0), 38),
      (rect(100.0, 736.0), 37),
    ] {
      assert_eq!(bar_row(capture, 20.0, 38), row, "{capture:?}");
    }
  }
}
