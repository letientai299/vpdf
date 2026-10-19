use super::View;
use crate::Result;
use crate::annotations;
use crate::pointer::{Hit, Pixel};
use crate::source::{Markup, Pages};
use crate::text::{PageId, Position};
use vpdf_reader::reader::TextSource;

#[derive(Clone, Copy)]
pub enum HitFrame {
  Drawn,
  Pending,
}

impl<S: TextSource + Pages + Markup> View<'_, S> {
  pub fn hit(
    &mut self,
    frame: HitFrame,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Hit> {
    if self.frame.pane.text {
      let position = match frame {
        HitFrame::Drawn => self.text().shown(column, row)?,
        HitFrame::Pending => self.text().hit(column, row)?,
      };
      let Some(position) = position else { return Ok(Hit::Nothing) };
      let Some((page, x, y)) = glyph_point(&mut self.source, position)? else {
        return Ok(Hit::Nothing);
      };
      return Ok(self.marked_hit(page, x, y)?.unwrap_or(Hit::Text));
    }
    let at = self.frame.pane.point(column, row, pixel);
    let Some((page, x, y)) = self.layout.point_at(at, self.reader.scroll)
    else {
      return Ok(Hit::Nothing);
    };
    if let Some(hit) = self.marked_hit(page, x, y)? {
      return Ok(hit);
    }
    let text = self.source.page_text(page)?;
    Ok(if text.covers(x, y) { Hit::Text } else { Hit::Nothing })
  }

  fn marked_hit(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<Hit>> {
    if let Some(index) = link_at(&mut self.source, page, x, y)? {
      return Ok(Some(Hit::Link(page, index)));
    }
    let notes = self.source.annotations(page)?;
    let Some(index) = annotations::hit(&notes, x, y) else { return Ok(None) };
    let text = !self.note_icon(page, index);
    Ok(Some(Hit::Note { page, index, text }))
  }

  fn note_icon(&mut self, page: PageId, index: usize) -> bool {
    let Ok(notes) = self.source.annotations(page) else { return false };
    let icon = notes.get(index).is_some_and(annotations::icon);
    !self.frame.pane.text && icon
  }
}

pub fn glyph_point(
  source: &mut impl TextSource,
  position: Position,
) -> Result<Option<(PageId, f32, f32)>> {
  let text = source.page_text(position.page)?;
  Ok(text.glyph(position).map(|glyph| {
    let (x, y) = glyph.bounds.centre();
    (position.page, x, y)
  }))
}

pub fn link_at(
  source: &mut impl Markup,
  page: PageId,
  x: f32,
  y: f32,
) -> Result<Option<usize>> {
  let links = source.links(page)?;
  Ok(links.iter().position(|link| link.contains(x, y)))
}
