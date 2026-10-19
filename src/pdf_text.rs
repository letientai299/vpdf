use crate::Result;
use std::sync::Arc;
use vpdf_model::text::{PageId, Text};
use vpdf_pdf::document::Document;
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
  fn bounds(&self, page: PageId) -> vpdf_model::text::Bounds {
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

impl vpdf_ui::source::Raster for PdfText<'_> {
  fn raster(
    &mut self,
    key: vpdf_model::tile::TileKey,
    visit: impl FnOnce(&[u8], usize) -> Result<()>,
  ) -> Result<()> {
    vpdf_pdf::raster::visit_area(self.0, key, visit)
  }
}

impl vpdf_ui::source::Outline for PdfText<'_> {
  fn outline(&self) -> &[vpdf_model::document::Heading] {
    &self.0.outline
  }
}

impl vpdf_ui::source::Markup for PdfText<'_> {
  fn links(
    &mut self,
    page: PageId,
  ) -> Result<Arc<Vec<vpdf_model::document::Link>>> {
    self.0.links(page)
  }
  fn annotations(
    &mut self,
    page: PageId,
  ) -> Result<Arc<Vec<vpdf_model::document::Annotation>>> {
    self.0.annotations(page)
  }
}
