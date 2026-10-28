use super::fixtures::TextPages;
use anyhow::Result;
use std::sync::Arc;
use vpdf_model::text::{PageId, Text};
use vpdf_reader::reader::TextSource;

impl TextSource for &TextPages {
  fn page_count(&self) -> usize {
    self.text.len()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    Ok(Arc::clone(&self.text[page.0]))
  }
}
