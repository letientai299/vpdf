use super::{
  edge::Edge,
  gesture::{Gesture, Swept},
  grab::Grab,
  wheel::{Turn, Wheel},
};
use crate::paint::Frame;
use crate::pointer::{Region, Route};
use crate::source::{Figures, Pages};
use crate::text::{Bounds, PageId, Position, Text};
use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use std::sync::Arc;
use std::time::Instant;
use vpdf_model::document::{Figure, FigureSource};
use vpdf_model::geometry::Screen;
use vpdf_reader::reader::{Reader, SelectionKind, TextSource};
use vpdf_terminal::terminal::At;

fn route(kind: MouseEventKind, region: Region) -> Route {
  Route {
    pane: 1,
    region,
    focus: false,
    clicks: 1,
    pixel: None,
    mouse: MouseEvent {
      kind,
      column: 10,
      row: 4,
      modifiers: KeyModifiers::NONE,
    },
  }
}

fn frame() -> Frame {
  Frame {
    at: At::default(),
    left: 0,
    text: false,
    screen: Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  }
}

#[test]
fn fractional_grabs_suppress_clicks() {
  let mut grab = Grab::default();
  grab.begin((10.0, 10.0));
  assert_eq!(grab.cells((10.0, 10.0), (8, 16)), None);
  assert!(grab.moved());
  assert!(!grab.release());
  grab.begin((10.0, 10.0));
  for (at, expected) in [
    ((17.0, 25.0), None),
    ((19.0, 27.0), Some((-1, -1))),
    ((25.0, 41.0), None),
    ((26.0, 42.0), Some((-1, -1))),
    ((17.0, 25.0), Some((1, 1))),
  ] {
    assert_eq!(grab.cells(at, (8, 16)), expected);
  }
  assert!(grab.cancel());
  assert!(!grab.release());
  grab.begin((10.0, 10.0));
  assert!(grab.release());
  assert!(!grab.release());
  grab.begin((10.0, 10.0));
  assert_eq!(grab.pixels((12.5, 7.0)), Some((-2.5, -3.0)));
  assert_eq!(grab.pixels((11.0, 8.0)), Some((1.5, 1.0)));
  assert!(!grab.release());
}

#[test]
fn edge_deadlines_survive_reports() {
  let mut edge = Edge::default();
  let now = Instant::now();
  let mut route = route(MouseEventKind::ScrollDown, Region::Page);
  let start = edge.follow(route, 1).expect("initial scroll");
  assert_eq!(start.rows(), 1);
  assert_eq!(edge.deadline(), None);
  start.begin(&mut edge, now);
  route.mouse.row = 0;
  assert!(edge.follow(route, -2).is_none());
  assert_eq!(edge.deadline(), Some(now + Edge::INTERVAL));
  assert!(edge.step(now).is_none());
  let step = edge.step(now + Edge::INTERVAL).expect("due scroll");
  assert_eq!((step.route, step.rows), (route, -2));
  let late = now + Edge::INTERVAL * 100;
  assert_eq!(edge.step(late).expect("capped scroll").rows, -8);
  assert_eq!(edge.deadline(), Some(late + Edge::INTERVAL));
  assert!(edge.follow(route, 0).is_none());
  assert_eq!(edge.deadline(), None);
  assert!(edge.follow(route, 1).is_some());
  assert_eq!(edge.deadline(), None);
}

#[test]
fn preview_wheel_fallback_is_temporary() {
  let mut wheel = Wheel::default();
  let preview = route(MouseEventKind::ScrollDown, Region::Preview);
  let page = route(MouseEventKind::ScrollDown, Region::Page);
  let rows = f32::from(crate::pointer::WHEEL_ROWS);
  assert_eq!(wheel.turn(preview, frame(), true), Turn::Preview(rows));
  assert_eq!(wheel.turn(page, frame(), false), Turn::Scroll(rows * 16.0));
  assert_eq!(wheel.turn(page, frame(), true), Turn::Preview(rows));
  let drift = route(MouseEventKind::ScrollRight, Region::Page);
  assert_eq!(wheel.turn(drift, frame(), true), Turn::None);
  wheel.cancel();
  assert_eq!(wheel.turn(drift, frame(), true), Turn::Pan(rows * 8.0));
  wheel.cancel();
  assert_eq!(wheel.turn(page, frame(), true), Turn::Scroll(rows * 16.0));
  assert_eq!(wheel.turn(preview, frame(), true), Turn::Scroll(rows * 16.0));
}

#[test]
fn ignored_zoom_still_latches_axis() {
  let mut wheel = Wheel::default();
  let mut zoom = route(MouseEventKind::ScrollDown, Region::Preview);
  zoom.mouse.modifiers = KeyModifiers::CONTROL;
  assert_eq!(
    wheel.turn(zoom, Frame { text: true, ..frame() }, true),
    Turn::None
  );
  assert_eq!(
    wheel.turn(route(MouseEventKind::ScrollRight, Region::Page), frame(), true),
    Turn::None
  );
  assert_eq!(
    wheel.turn(zoom, frame(), true),
    Turn::Zoom { steps: -0.5, at: (84.0, 72.0) }
  );
  zoom.mouse.modifiers = KeyModifiers::NONE;
  assert!(matches!(wheel.turn(zoom, frame(), true), Turn::Scroll(_)));
}

struct Source {
  figures: Arc<Vec<Figure>>,
  reads: Vec<&'static str>,
}

impl Pages for Source {
  fn count(&self) -> usize {
    1
  }
  fn bounds(&self, _: PageId) -> Bounds {
    unreachable!()
  }
  fn label(&self, _: PageId) -> &str {
    unreachable!()
  }
}

impl Figures for Source {
  fn figures(&mut self, _: PageId) -> crate::Result<Arc<Vec<Figure>>> {
    self.reads.push("figures");
    Ok(Arc::clone(&self.figures))
  }
}

impl TextSource for Source {
  fn page_count(&self) -> usize {
    1
  }
  fn page_text(&mut self, _: PageId) -> crate::Result<Arc<Text>> {
    self.reads.push("text");
    anyhow::bail!("Text unavailable")
  }
}

#[test]
fn sweep_resolution_is_lazy() -> crate::Result<()> {
  let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 20.0, y1: 20.0 };
  let mut source = Source { figures: Arc::default(), reads: Vec::new() };
  let mut gesture = Gesture::default();
  let idle = gesture.snapshot();
  gesture.begin_sweep(PageId(0), (2.0, 3.0));
  gesture.sweep_to((40.0, 50.0), bounds);
  assert_eq!(gesture.sweep().unwrap().corner(), Some((20.0, 20.0)));
  assert!(gesture.hint().is_some());
  assert_ne!(gesture.snapshot(), idle);
  let swept = gesture.finish_sweep().unwrap().resolve(&mut source)?;
  let expected = Bounds { x0: 2.0, y0: 3.0, ..bounds };
  assert!(matches!(swept, Swept::Region(PageId(0), b) if b == expected));
  assert_eq!(source.reads, [] as [&str; 0]);
  assert_eq!(gesture.snapshot(), idle);
  for kind in [FigureSource::Graphic, FigureSource::Caption] {
    source.figures =
      Arc::new(vec![Figure { bounds, caption: String::new(), source: kind }]);
    source.reads.clear();
    gesture.begin_sweep(PageId(0), (2.0, 3.0));
    let result = gesture.finish_sweep().unwrap().resolve(&mut source);
    if kind == FigureSource::Graphic {
      assert!(matches!(result?, Swept::Region(PageId(0), b) if b == bounds));
      assert_eq!(source.reads, ["figures"]);
    } else {
      assert!(result.is_err());
      assert_eq!(source.reads, ["figures", "text"]);
    }
    assert_eq!(gesture.snapshot(), idle);
  }
  Ok(())
}

#[test]
fn cancelled_sweep_waits_for_release() {
  let mut gesture = Gesture::default();
  let idle = gesture.snapshot();
  gesture.begin_sweep(PageId(0), (2.0, 3.0));
  assert!(gesture.drop_sweep());
  assert!(gesture.alt());
  assert!(gesture.dropped());
  assert!(!gesture.drop_sweep());
  assert!(gesture.finish_sweep().is_none());
  assert_eq!(gesture.snapshot(), idle);
}

#[test]
fn failed_click_resets_selection() -> crate::Result<()> {
  let mut source = Source { figures: Arc::default(), reads: Vec::new() };
  let mut gesture = Gesture::default();
  let idle = gesture.snapshot();
  let mut reader = Reader::default();
  let pos = Position { page: PageId(0), line: 3, glyph: 4 };
  assert!(!gesture.click(&mut reader, pos, 3, &mut source)?);
  assert_eq!(source.reads, [] as [&str; 0]);
  assert_eq!(reader.selection.unwrap().kind, SelectionKind::Line);
  reader.desired = Some(7);
  assert!(gesture.click(&mut reader, pos, 2, &mut source).is_err());
  assert_eq!(source.reads, ["text"]);
  assert!(reader.selection.is_none());
  assert_eq!(reader.cursor, pos);
  assert_eq!(reader.desired, None);
  assert_eq!(gesture.snapshot(), idle);
  gesture.drag(&mut reader, Position::default(), &mut source)?;
  assert_eq!(source.reads, ["text"]);
  assert_eq!(reader.selection.unwrap().anchor, pos);
  assert_eq!(reader.cursor, Position::default());
  Ok(())
}
