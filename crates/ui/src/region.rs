use crate::Result;
use crate::keys::Action;
use crate::layout::Layout;
use crate::paint::Frame;
use crate::pointer::Items;
use crate::split::Rect;
use crate::theme;
use std::io::Write;
use std::ops::Range;
use unicode_width::UnicodeWidthStr;
use vpdf_model::scene::{Draw, Layer, Op};
use vpdf_model::style::Role;
use vpdf_model::text::{Bounds, PageId};
use vpdf_reader::motion::Motion;
use vpdf_terminal::graphics::Graphics;

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
  bar: Option<Bar>,
}

struct Bar {
  action: Operation,
  status: String,
  cells: Option<BarCells>,
}

struct BarCells {
  area: Rect,
  items: Items,
}

type Spans = Vec<(Range<usize>, usize)>;

pub const BAR_HINT: &str = "h/l choose; Enter runs; Esc closes";

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
    Action::Quit | Action::Cancel | Action::Region => Route::Close,
    Action::Command => Route::Command,
    _ if crate::input::transfers_overlay(action) => Route::Transfer,
    _ => Route::Ignore,
  }
}

impl Region {
  pub fn selected(snapshot: Snapshot) -> Self {
    Self { snapshot, bar: None }
  }

  pub fn open(snapshot: Snapshot) -> Self {
    Self {
      snapshot,
      bar: Some(Bar {
        action: Operation::Image,
        status: String::new(),
        cells: None,
      }),
    }
  }

  pub fn snapshot(&self) -> &Snapshot {
    &self.snapshot
  }

  pub fn is_bar(&self) -> bool {
    self.bar.is_some()
  }

  pub fn action(&self) -> Option<Operation> {
    self.bar.as_ref().map(|bar| bar.action)
  }

  pub fn status(&self) -> Option<&str> {
    self.bar.as_ref().map(|bar| bar.status.as_str())
  }

  pub fn request(&mut self, id: CaptureId) -> Snapshot {
    if self.bar.is_some() {
      self.snapshot.id = id;
    }
    Snapshot { id, ..self.snapshot.clone() }
  }

  pub fn update(&mut self, id: CaptureId, message: &str) -> bool {
    if let Some(bar) = &mut self.bar {
      if self.snapshot.id != id {
        return false;
      }
      message.clone_into(&mut bar.status);
    }
    true
  }

  pub fn allows(&self, action: Action) -> bool {
    self.bar.is_none() || !matches!(route(action), Route::Ignore)
  }

  pub fn input(&mut self, action: Action) -> Transition {
    let Some(bar) = &mut self.bar else {
      return if matches!(
        action,
        Action::Figure(_) | Action::FigureFit | Action::Region
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
    self.bar.as_ref()?.cells.as_ref().map(|cells| cells.area)
  }

  pub fn items(&self) -> Items {
    self
      .bar
      .as_ref()
      .and_then(|bar| bar.cells.as_ref())
      .map_or_default(|cells| cells.items.clone())
  }

  pub fn select(&mut self, index: usize) {
    if let Some(bar) = &mut self.bar
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
    if let Some(bar) = &mut self.bar {
      bar.cells = Some(BarCells { area, items });
    }
  }
}

pub struct Canvas<'a> {
  pub layout: &'a Layout,
  pub scroll: f32,
  pub frame: Frame,
}

impl Canvas<'_> {
  pub fn draw(
    &self,
    region: Option<&mut Region>,
    sweep: Option<(PageId, Bounds)>,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let snap_to_cells = graphics.crop_quirk();
    let mut scene =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    self.scene(region, sweep, snap_to_cells, &mut scene);
    scene.finish()?;
    Ok(())
  }

  pub fn scene<'a>(
    &self,
    region: Option<&mut Region>,
    sweep: Option<(PageId, Bounds)>,
    snap_to_cells: bool,
    scene: &mut impl Draw<'a>,
  ) {
    if let Some((page, bounds)) = sweep {
      let mut rect = self.layout.project(page, bounds, self.scroll);
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
      self.outline(scene, rect);
    }
    let Some(region) = region else { return };
    let Some(bar) = &region.bar else { return };
    let snapshot = &region.snapshot;
    let (action, status) = (&bar.action, &bar.status);
    let (page, action, status) = (snapshot.page, *action, status.clone());
    let rect = self.layout.project(page, snapshot.bounds, self.scroll);
    self.outline(scene, rect);
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
    // Graphics fill the row; text stops.
    let width =
      if self.frame.text { text.width() } else { usize::from(screen.columns) };
    region.place_bar(row, width, items, self.frame.left);
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
    crate::paint::move_to(scene, self.frame.at, row, self.frame.left);
    scene.push(Op::Style(Role::Selected));
    scene.text(&text);
    scene.push(Op::Style(Role::Reset));
  }

  /// Dims around a rect; outlines it.
  fn outline<'a>(&self, scene: &mut impl Draw<'a>, rect: Bounds) {
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
      for bounds in frame_edges(Bounds { x0, y0, x1, y1 }, (w, h)) {
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

/// A 2 px frame within `(w, h)`.
fn frame_edges(rect: Bounds, (w, h): (f32, f32)) -> [Bounds; 4] {
  // Overlays round out to whole pixels.
  let (x0, y0) = (rect.x0.floor(), rect.y0.floor());
  let (x1, y1) = (rect.x1.ceil(), rect.y1.ceil());
  // U26: outside where room allows.
  let top = band(y0, -1.0, y0 >= 2.0);
  let bottom = band(y1, 1.0, y1 + 2.0 <= h);
  let left = band(x0, -1.0, x0 >= 2.0);
  let right = band(x1, 1.0, x1 + 2.0 <= w);
  [
    Bounds { x0: left.0, x1: right.1, y0: top.0, y1: top.1 },
    Bounds { x0: left.0, x1: right.1, y0: bottom.0, y1: bottom.1 },
    Bounds { x0: left.0, x1: left.1, y0, y1 },
    Bounds { x0: right.0, x1: right.1, y0, y1 },
  ]
}

/// A 2 px band, outward if room.
fn band(edge: f32, dir: f32, fits: bool) -> (f32, f32) {
  let far = if fits { edge + 2.0 * dir } else { edge - 2.0 * dir };
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
mod tests {
  use super::*;
  fn snapshot(id: u64) -> Snapshot {
    Snapshot {
      id: CaptureId::new(id),
      page: PageId(0),
      bounds: Bounds { x0: 10.0, y0: 20.0, x1: 60.0, y1: 80.0 },
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
  fn recorded_regions_keep_hits() -> Result<()> {
    use vpdf_model::geometry::At;
    use vpdf_model::geometry::Screen;
    use vpdf_model::scene::Scene;
    use vpdf_terminal::graphics::Presentation;
    use vpdf_terminal::payload::Encoder;
    let screen = Screen {
      columns: 20,
      rows: 10,
      cell_width: 8,
      cell_height: 16,
      origin: (2, 1),
    };
    let layout =
      Layout::new([Bounds { x0: 0.0, y0: 0.0, x1: 160.0, y1: 160.0 }], screen);
    let canvas = Canvas {
      layout: &layout,
      scroll: 0.0,
      frame: Frame {
        screen,
        at: At { column: 5, row: 1 },
        left: 3,
        text: false,
      },
    };
    let sweep =
      Some((PageId(0), Bounds { x0: 12.5, y0: 24.5, x1: 63.2, y1: 81.7 }));
    for crop_quirk in [false, true] {
      let mut recorded = Region::open(snapshot(1));
      let mut scene = Scene::new();
      canvas.scene(Some(&mut recorded), sweep, crop_quirk, &mut scene);
      let area = recorded.cells().unwrap();
      assert_eq!(area.column, 3);
      assert_eq!(recorded.items().at(area.column, area.row), Some(0));
      let profile = Presentation { crop_quirk, ..Presentation::default() };
      let mut graphics = Graphics::new(profile, &Encoder::default());
      let mut replay = Graphics::new(profile, &Encoder::default());
      let mut direct = Region::open(snapshot(1));
      for _ in 0..2 {
        let mut expected = Vec::new();
        graphics.begin(&mut expected)?;
        canvas.draw(Some(&mut direct), sweep, &mut graphics, &mut expected)?;
        graphics.end(&mut expected)?;
        assert_eq!(direct.cells(), Some(area));
        let mut actual = Vec::new();
        replay.begin(&mut actual)?;
        let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
          .with_graphics(&mut replay);
        painter.paint(&scene);
        painter.finish()?;
        replay.end(&mut actual)?;
        assert_eq!(actual, expected);
      }
    }
    Ok(())
  }

  #[test]
  fn narrow_bar_tracks_cells() -> Result<()> {
    use vpdf_model::geometry::At;
    use vpdf_model::geometry::Screen;
    let screen = Screen {
      columns: 20,
      rows: 10,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
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
    canvas.draw(
      Some(&mut region),
      None,
      &mut Graphics::default(),
      &mut output,
    )?;
    let area = region.cells().unwrap();
    assert_eq!(area.column, 7);
    assert_eq!(area.columns, 13);
    assert_eq!(region.items().at(7, area.row), Some(2));
    assert_eq!(region.items().at(16, area.row), Some(2));
    assert_eq!(region.items().at(17, area.row), None);
    assert!(String::from_utf8(output)?.contains("[Save PNG] p1"));
    Ok(())
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
      for (edge, inside) in frame_edges(capture, (w, h)).into_iter().zip(inside)
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
