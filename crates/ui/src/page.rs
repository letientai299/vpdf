pub mod landing;
mod point;
pub use point::Point;
pub mod scroll;
mod view;
use crate::Result;
use crate::layout::Layout;
use crate::paint::{Marks, outside};
use crate::source::Pages;
use crate::text::{Bounds, Line, PageId, Position};
use crate::theme;
use std::io::Write;
pub use view::Viewport;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::{Draw, Layer};
use vpdf_reader::motion::{covered_rows, range};
use vpdf_reader::reader::{SelectionKind, TextSource};
use vpdf_terminal::graphics::Graphics;

pub struct Canvas<'a> {
  pub layout: &'a Layout,
  pub screen: Screen,
  pub left: u16,
  pub menu: Option<Bounds>,
}

struct Painter<'a, S, D> {
  canvas: &'a Canvas<'a>,
  source: &'a mut S,
  scene: &'a mut D,
  scroll: f32,
}

impl Canvas<'_> {
  pub fn draw(
    &self,
    source: &mut (impl TextSource + Pages),
    marks: &Marks<'_>,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    let drawn = self.scene(source, marks, &mut painter);
    painter.finish()?;
    drawn
  }

  pub fn scene<'a>(
    &self,
    source: &mut (impl TextSource + Pages),
    marks: &Marks<'_>,
    scene: &mut impl Draw<'a>,
  ) -> Result<()> {
    let mut painter =
      Painter { canvas: self, source, scene, scroll: marks.reader.scroll };
    if marks.cursorline && marks.reader.selection.is_some() {
      painter.line(marks.reader.cursor, theme::CURSORLINE)?;
    }
    if let Some(position) = marks.landing {
      painter.line(position, theme::LANDING)?;
    }
    if let Some(position) = marks.flash {
      painter.glyph(position)?;
    }
    for &(page, bounds) in marks.hovered {
      let rect = self.layout.project(page, bounds, painter.scroll);
      painter.tint(rect, theme::LINK);
    }
    let visible = self.layout.visible(painter.scroll);
    let start =
      marks.matches.partition_point(|h| h.first.page.0 < visible.start);
    for hit in marks.matches[start..]
      .iter()
      .take_while(|h| h.first.page.0 < visible.end)
      .take(300)
    {
      painter.range((hit.first, hit.last), false, theme::MATCH)?;
    }
    if let Some(selection) = marks.reader.selection {
      painter.range(
        (selection.anchor, marks.reader.cursor),
        selection.kind == SelectionKind::Line,
        theme::SELECTION,
      )?;
    }
    if marks.reader.uses_cursor() {
      painter.glyph(marks.reader.cursor)?;
    }
    Ok(())
  }
}

impl<'a, S: TextSource + Pages, D: Draw<'a>> Painter<'_, S, D> {
  fn glyph(&mut self, position: Position) -> Result<()> {
    if let Some(glyph) = self.source.page_text(position.page)?.glyph(position) {
      let mut rect =
        self.canvas.layout.project(position.page, glyph.bounds, self.scroll);
      rect.x1 = rect.x1.max(rect.x0 + 2.0);
      self.tint(rect, theme::CURSOR);
    }
    Ok(())
  }

  fn tint(&mut self, rect: Bounds, color: [u8; 4]) {
    for piece in outside(rect, self.canvas.menu) {
      self.scene.fill(
        piece,
        color,
        Layer::AboveText,
        self.canvas.screen,
        self.canvas.left,
      );
    }
  }

  fn line(&mut self, position: Position, color: [u8; 4]) -> Result<()> {
    let text = self.source.page_text(position.page)?;
    let Some(bounds) = text.lines.get(position.line).and_then(Line::span)
    else {
      return Ok(());
    };
    let page = self
      .canvas
      .layout
      .crop_for(position.page)
      .apply(self.source.bounds(position.page));
    let band = Bounds {
      x0: page.x0,
      x1: page.x1,
      y0: bounds.y0.max(page.y0),
      y1: bounds.y1.min(page.y1),
    };
    if band.height() > 0.0 {
      let rect = self.canvas.layout.project(position.page, band, self.scroll);
      for piece in outside(rect, self.canvas.menu) {
        self.scene.fill(
          piece,
          color,
          Layer::BelowText,
          self.canvas.screen,
          self.canvas.left,
        );
      }
    }
    Ok(())
  }

  fn range(
    &mut self,
    (first, last): (Position, Position),
    linewise: bool,
    color: [u8; 4],
  ) -> Result<()> {
    let range = range(first, last);
    let (first, last) = range;
    for page in self.canvas.layout.visible(self.scroll) {
      if page < first.page.0 || page > last.page.0 {
        continue;
      }
      let text = self.source.page_text(PageId(page))?;
      for bounds in covered_rows(&text, PageId(page), range, linewise) {
        let rect =
          self.canvas.layout.project(PageId(page), bounds, self.scroll);
        self.tint(rect, color);
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests;
