use vpdf_model::geometry::Screen;
use vpdf_ui::layout::Layout;
use vpdf_ui::layout::tiles::Frame;

pub fn layout() -> Layout {
  let page =
    vpdf_model::text::Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 1000.0 };
  Layout::new(
    [page; 5],
    Screen {
      columns: 100,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (7, 3),
    },
  )
}

pub fn frame(layout: &Layout) -> Frame {
  Frame { pane: 0, screen: layout.screen(), left: 4 }
}
