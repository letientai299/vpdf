use vpdf_model::text::Bounds;
use vpdf_ui::layout::Layout;

pub fn tiles(pages: impl IntoIterator<Item = Bounds>) -> Layout {
  Layout::new(pages, crate::screen::cells(120, 40))
}
