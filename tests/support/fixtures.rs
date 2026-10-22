#[path = "screen.rs"]
pub mod screen;
#[path = "text.rs"]
mod text_fixture;
use anyhow::Result;
use std::sync::Arc;
pub use text_fixture::{glyph, line, plain};
use vpdf_model::text::{Bounds, Line, PageId, Text};
use vpdf_reader::reader::TextSource;
use vpdf_ui::source::Pages;

pub fn lines(lines: impl IntoIterator<Item = Line>) -> Arc<Text> {
  Arc::new(text_fixture::text(lines))
}

pub struct Source {
  pub text: Arc<Text>,
  pub count: usize,
  pub bounds: Bounds,
  pub label: &'static str,
}

impl Pages for Source {
  fn count(&self) -> usize {
    self.count
  }
  fn bounds(&self, _: PageId) -> Bounds {
    self.bounds
  }
  fn label(&self, _: PageId) -> &'static str {
    self.label
  }
}

impl TextSource for Source {
  fn page_count(&self) -> usize {
    self.count
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    anyhow::ensure!(page.0 < self.count, "No page");
    Ok(Arc::clone(&self.text))
  }
}
