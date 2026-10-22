pub mod landing;
mod point;
mod raster;
pub use point::Point;
pub mod scroll;
mod view;
use crate::Result;
use crate::layout::Layout;
use crate::paint::{Marks, outside};
use crate::source::Pages;
use crate::text::{Bounds, PageId, Position};
use crate::theme;
pub use view::Viewport;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::{Draw, Layer};
use vpdf_model::text::{Glyph, Quad};
use vpdf_reader::motion::{covered_quads, range};
use vpdf_reader::reader::{SelectionKind, TextSource};

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
      let scale =
        self.canvas.layout.pages()[position.page.0].scale as f32 / 1024.0;
      let quad = glyph.quad().widen(2.0 / scale);
      self.polygon(position.page, quad, theme::CURSOR, Layer::AboveText);
    }
    Ok(())
  }

  fn polygon(
    &mut self,
    page: PageId,
    quad: Quad,
    color: [u8; 4],
    layer: Layer,
  ) {
    let layout = self.canvas.layout;
    let matrix = layout.pages()[page.0].matrix;
    let (left, top) = layout.origin(page, self.scroll);
    let quad = Quad(quad.0.map(|(x, y)| {
      let (x, y) = matrix.transform_xy(x, y);
      (x + left, y + top)
    }));
    let crop = layout.crop_for(page).apply(self.source.bounds(page));
    let crop = layout.project(page, crop, self.scroll);
    let screen = Bounds {
      x0: 0.0,
      y0: 0.0,
      x1: self.canvas.screen.width(),
      y1: self.canvas.screen.height(),
    };
    let Some(clip) = vpdf_model::geometry::clip(crop, screen) else { return };
    for rect in raster::strips(quad, clip) {
      for piece in outside(rect, self.canvas.menu) {
        self.scene.fill(
          piece,
          color,
          layer,
          self.canvas.screen,
          self.canvas.left,
        );
      }
    }
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
    let Some(quad) = text
      .lines
      .get(position.line)
      .and_then(|line| Quad::span(line.glyphs.iter().map(Glyph::quad)))
    else {
      return Ok(());
    };
    let page = self
      .canvas
      .layout
      .crop_for(position.page)
      .apply(self.source.bounds(position.page));
    self.polygon(position.page, quad.band(page), color, Layer::BelowText);
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
      for quad in covered_quads(&text, PageId(page), range, linewise) {
        self.polygon(PageId(page), quad, color, Layer::AboveText);
      }
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests;
