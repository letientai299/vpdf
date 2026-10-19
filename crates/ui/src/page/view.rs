use crate::Result;
use crate::layout::Layout;
use crate::source::Pages;
use crate::text::{Bounds, PageId, Position};
use vpdf_model::geometry::Screen;
use vpdf_reader::reader::TextSource;

pub struct Viewport<'a, S> {
  layout: &'a mut Layout,
  scroll: &'a mut f32,
  source: S,
  screen: Screen,
}

impl<'a, S> Viewport<'a, S> {
  pub fn new(
    layout: &'a mut Layout,
    scroll: &'a mut f32,
    source: S,
    screen: Screen,
  ) -> Self {
    Self { layout, scroll, source, screen }
  }
}

impl<S: TextSource + Pages> Viewport<'_, S> {
  pub fn shows(&self, page: PageId, bounds: Bounds) -> bool {
    let crop = self.layout.crop_for(page).apply(self.source.bounds(page));
    if bounds.x1 <= crop.x0
      || bounds.x0 >= crop.x1
      || bounds.y1 <= crop.y0
      || bounds.y0 >= crop.y1
    {
      return false;
    }
    let rect = self.layout.project(page, bounds, *self.scroll);
    rect.x1 > 0.0
      && rect.x0 < self.layout.screen().width()
      && rect.y1 > 0.0
      && rect.y0 < self.screen.height()
  }

  pub fn visible(&mut self, position: Position) -> Result<bool> {
    if !self.layout.visible(*self.scroll).contains(&position.page.0) {
      return Ok(false);
    }
    let text = self.source.page_text(position.page)?;
    let Some(glyph) = text.glyph(position) else { return Ok(false) };
    Ok(self.shows(position.page, glyph.bounds))
  }

  pub fn first(&mut self) -> Result<Option<Position>> {
    for page in self.layout.visible(*self.scroll).map(PageId) {
      let text = self.source.page_text(page)?;
      let positions = text.lines.iter().enumerate().flat_map(|(line, row)| {
        row
          .glyphs
          .iter()
          .enumerate()
          .filter(|(_, ch)| !ch.ch.is_whitespace())
          .map(move |(glyph, _)| Position { page, line, glyph })
      });
      for position in positions {
        if self.visible(position)? {
          return Ok(Some(position));
        }
      }
    }
    Ok(None)
  }

  pub fn point(&mut self, position: Position) -> Result<(f32, f32)> {
    let text = self.source.page_text(position.page)?;
    Ok(text.glyph(position).map_or((0.0, 0.0), |glyph| {
      self.layout.project(position.page, glyph.bounds, *self.scroll).centre()
    }))
  }

  pub fn nearest_at(
    &mut self,
    page: PageId,
    (x, y): (f32, f32),
  ) -> Result<Position> {
    Ok(self.source.page_text(page)?.nearest(page, x, y))
  }

  pub fn nearest(&mut self, point: (f32, f32)) -> Result<Option<Position>> {
    let mut nearest = None;
    let mut distance = (u8::MAX, f32::INFINITY, f32::INFINITY);
    for page in self.layout.visible(*self.scroll).map(PageId) {
      let text = self.source.page_text(page)?;
      let glyphs = text.lines.iter().enumerate().flat_map(|(line, row)| {
        row
          .glyphs
          .iter()
          .enumerate()
          .map(move |(glyph, item)| (line, glyph, item))
      });
      for (line, glyph, item) in glyphs {
        if item.ch.is_whitespace() || !self.shows(page, item.bounds) {
          continue;
        }
        let rect = self.layout.project(page, item.bounds, *self.scroll);
        let (x, y) = rect.centre();
        let partial = u8::from(rect.y0 < 0.0 || rect.y1 > self.screen.height());
        let next = (partial, (y - point.1).abs(), (x - point.0).abs());
        if next < distance {
          distance = next;
          nearest = Some(Position { page, line, glyph });
        }
      }
    }
    Ok(nearest)
  }

  pub fn reveal(&mut self, position: Position, margin: f32) -> Result<()> {
    let Some(rect) = self.rect(position)? else {
      *self.scroll = self.layout.pages()[position.page.0].top;
      return Ok(());
    };
    *self.scroll =
      self.layout.reveal(position.page, rect, *self.scroll, margin);
    Ok(())
  }

  pub fn land(&mut self, position: Position, margin: f32) -> Result<()> {
    let before = *self.scroll;
    self.reveal(position, margin)?;
    if self.scroll.total_cmp(&before).is_eq() {
      return Ok(());
    }
    if let Some(rect) = self.rect(position)? {
      *self.scroll = self.layout.clamp_scroll(rect.y0 - margin);
    }
    Ok(())
  }

  pub fn align(&mut self, position: Position, fraction: f32) -> Result<()> {
    if let Some(rect) = self.rect(position)? {
      *self.scroll = self.layout.clamp_scroll(
        rect.y0 - fraction * (self.layout.screen().height() - rect.height()),
      );
    }
    Ok(())
  }

  fn rect(&mut self, position: Position) -> Result<Option<Bounds>> {
    let text = self.source.page_text(position.page)?;
    Ok(
      text
        .glyph(position)
        .map(|glyph| self.layout.project(position.page, glyph.bounds, 0.0)),
    )
  }
}

#[cfg(test)]
mod tests;
