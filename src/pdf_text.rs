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
