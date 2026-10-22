use crate::fixtures::screen;
use vpdf_model::geometry::Screen;
use vpdf_ui::layout::Layout;
use vpdf_ui::layout::tiles::Frame;

pub fn layout() -> Layout {
  let page =
    vpdf_model::text::Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 1000.0 };
  Layout::new([page; 5], Screen { origin: (7, 3), ..screen::cells(100, 40) })
}

pub fn frame(layout: &Layout) -> Frame {
  Frame { pane: 0, screen: layout.screen(), left: 4 }
}
