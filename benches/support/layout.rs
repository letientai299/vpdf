use vpdf_model::geometry::Screen;
use vpdf_model::text::Bounds;
use vpdf_ui::layout::Layout;

pub fn tiles(pages: impl IntoIterator<Item = Bounds>) -> Layout {
  Layout::new(
    pages,
    Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  )
}
