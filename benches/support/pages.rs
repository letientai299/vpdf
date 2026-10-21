use super::fixtures::TextPages;
use vpdf_model::text::{Bounds, PageId};
use vpdf_ui::source::Pages;
impl Pages for &TextPages {
  fn count(&self) -> usize {
    self.text.len()
  }
  fn bounds(&self, page: PageId) -> Bounds {
    self.metadata[page.0].1
  }
  fn label(&self, page: PageId) -> &str {
    &self.metadata[page.0].0
  }
}
