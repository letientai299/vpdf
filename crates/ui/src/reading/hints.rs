use super::View;
use crate::Result;
use crate::hints::{self, Hints};
use crate::source::{Markup, Pages};
use crate::text::{Bounds, PageId, Position};
use vpdf_reader::reader::TextSource;

impl<S: TextSource + Pages + Markup> View<'_, S> {
  pub fn hints(&mut self, copy: bool) -> Result<Hints> {
    let mut found = Vec::new();
    let pages = if self.frame.pane.text {
      self.text().pages()?
    } else {
      self.layout.visible(self.reader.scroll)
    };
    for page in pages {
      for link in self.source.links(PageId(page))?.iter() {
        if let Some(spot) =
          hints::spot(link, |bounds| self.link_cell(PageId(page), bounds))?
        {
          found.push((spot, link.target.clone()));
        }
      }
    }
    let frame = self.frame.pane;
    Hints::new(found, copy, frame.left, frame.screen.columns)
  }

  /// Screen cell of the glyph at `position`, when shown.
  pub fn glyph_cell(
    &mut self,
    position: Position,
  ) -> Result<Option<(u16, u16)>> {
    if self.frame.pane.text {
      return self.text().cell(position);
    }
    let text = self.source.page_text(position.page)?;
    let Some(glyph) = text.glyph(position) else { return Ok(None) };
    let bounds = glyph.bounds;
    self.link_cell(position.page, bounds)
  }

  fn link_cell(
    &mut self,
    page: PageId,
    bounds: Bounds,
  ) -> Result<Option<(u16, u16)>> {
    // Padded boxes reach the line above.
    let middle = f32::midpoint(bounds.y0, bounds.y1);
    if self.frame.pane.text {
      let position =
        self.source.page_text(page)?.nearest(page, bounds.x0, middle);
      return self.text().cell(position);
    }
    hints::page_cell(
      self.layout,
      self.reader.scroll,
      self.frame.pane.left,
      page,
      bounds,
    )
  }
}
