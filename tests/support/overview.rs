use vpdf_model::geometry::Screen;
use vpdf_model::text::{Bounds, PageId};
use vpdf_ui::overview::Frame;
use vpdf_ui::source::Pages;

pub struct Source;
impl Pages for Source {
  fn count(&self) -> usize {
    3
  }
  fn bounds(&self, _: PageId) -> Bounds {
    Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 }
  }
  fn label(&self, _: PageId) -> &'static str {
    "page"
  }
}

pub fn frame() -> Frame {
  let screen = Screen {
    columns: 100,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (7, 3),
  };
  Frame {
    pane: 1,
    screen,
    clip: screen,
    at: vpdf_model::geometry::At { column: 7, row: 3 },
    left: 4,
    hovered: Some(PageId(2)),
    mask: None,
  }
}
