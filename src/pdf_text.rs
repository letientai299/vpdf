use crate::Result;
use crate::document::Document;
use crate::text::{PageId, Text};
use std::sync::Arc;
use vpdf_reader::reader::TextSource;

/// A native document as reader text.
pub struct PdfText<'a>(pub &'a mut Document);

impl TextSource for PdfText<'_> {
  fn page_count(&self) -> usize {
    self.0.pages.len()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.0.text(page)
  }
}

impl vpdf_ui::source::Pages for PdfText<'_> {
  fn count(&self) -> usize {
    self.0.pages.len()
  }
  fn bounds(&self, page: PageId) -> crate::text::Bounds {
    self.0.pages[page.0].bounds
  }
  fn label(&self, page: PageId) -> &str {
    self.0.label(page)
  }
}

impl vpdf_ui::source::Figures for PdfText<'_> {
  fn figures(
    &mut self,
    page: PageId,
  ) -> Result<Arc<Vec<vpdf_model::document::Figure>>> {
    self.0.figures(page)
  }
}
