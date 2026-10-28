pub mod landing;
mod point;
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
use vpdf_model::scene::{Draw, Layer, Stage};
use vpdf_model::text::Quad;
use vpdf_reader::motion::{covered_quads, range};
use vpdf_reader::reader::{SelectionKind, TextSource};

pub struct Canvas<'a> {
  pub layout: &'a Layout,
  pub screen: Screen,
  pub left: u16,
  pub menu: Option<Bounds>,
}

impl Canvas<'_> {
  fn stage(&self) -> Stage {
    Stage { screen: self.screen, left: self.left }
  }
}

struct Painter<'a, S, D> {
  canvas: &'a Canvas<'a>,
  source: &'a mut S,
  scene: &'a mut D,
  scroll: f32,
}

impl Canvas<'_> {
  pub fn rectangle<'a>(
    &self,
    source: &mut (impl TextSource + Pages),
    page: PageId,
    bounds: Bounds,
    scroll: f32,
    scene: &mut impl Draw<'a>,
  ) {
    let mut painter = Painter { canvas: self, source, scene, scroll };
    painter.polygon(page, bounds.into(), theme::MATCH, Layer::AboveText);
  }

  pub fn scene<'a>(
    &self,
    source: &mut (impl TextSource + Pages),
    marks: &Marks<'_>,
    scene: &mut impl Draw<'a>,
  ) -> Result<()> {
    let mut painter =
      Painter { canvas: self, source, scene, scroll: marks.reader.scroll };
    if marks.cursorline && marks.reader.uses_cursor() {
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
    if marks.reader.document_selected {
      painter.range(
        (
          Position::default(),
          Position {
            page: PageId(usize::MAX),
            line: usize::MAX,
            glyph: usize::MAX,
          },
        ),
        true,
        theme::SELECTION,
      )?;
    }
    if let Some(selection) = marks.reader.selection {
      painter.range(
        (selection.anchor, marks.reader.cursor),
        selection.kind == SelectionKind::Line,
        theme::SELECTION,
      )?;
    }
    painter.links(&marks.links);
    if marks.reader.uses_cursor() {
      painter.glyph(marks.reader.cursor)?;
    }
    Ok(())
  }
}

impl<'a, S: TextSource + Pages, D: Draw<'a>> Painter<'_, S, D> {
  fn links(&mut self, links: &crate::paint::LinkMarks) {
    for &(page, bounds) in &links.underlined {
      self.underline(page, bounds, links);
    }
    for &(page, bounds) in &links.focused {
      let scale = self.canvas.layout.pages()[page.0].scale as f32 / 1024.0;
      let pad = (links.width + 1.0) / scale;
      let width = links.width / scale;
      let outer = Bounds {
        x0: bounds.x0 - pad,
        y0: bounds.y0 - pad,
        x1: bounds.x1 + pad,
        y1: bounds.y1 + pad,
      };
      let inner = Bounds {
        x0: outer.x0 + width,
        y0: outer.y0 + width,
        x1: outer.x1 - width,
        y1: outer.y1 - width,
      };
      for edge in outside(outer, Some(inner)) {
        self.polygon(page, edge.into(), links.color, Layer::AboveText);
      }
      if !links.underlined.contains(&(page, bounds)) {
        self.underline(page, bounds, links);
      }
    }
  }

  fn underline(
    &mut self,
    page: PageId,
    bounds: Bounds,
    links: &crate::paint::LinkMarks,
  ) {
    let scale = self.canvas.layout.pages()[page.0].scale as f32 / 1024.0;
    let line =
      Bounds { y0: (bounds.y1 - links.width / scale).max(bounds.y0), ..bounds };
    self.polygon(page, line.into(), links.color, Layer::AboveText);
  }

  fn glyph(&mut self, position: Position) -> Result<()> {
    let text = self.source.page_text(position.page)?;
    if let Some(quad) = text.lines.get(position.line).and_then(|line| {
      line.quad(position.glyph..position.glyph.saturating_add(1))
    }) {
      let scale =
        self.canvas.layout.pages()[position.page.0].scale as f32 / 1024.0;
      let quad = quad.widen(2.0 / scale);
      self.polygon(position.page, quad, theme::CURSOR, Layer::AboveText);
    }
    Ok(())
  }

  fn polygon(
    &mut self,
    page: PageId,
    quad: Quad,
    color: theme::Paint,
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
    let screen = self.canvas.screen.bounds();
    let Some(clip) = vpdf_model::geometry::clip(crop, screen) else { return };
    for piece in outside(clip, self.canvas.menu) {
      self.scene.quad(quad, piece, color, layer, self.canvas.stage());
    }
  }

  fn tint(&mut self, rect: Bounds, color: theme::Paint) {
    for piece in outside(rect, self.canvas.menu) {
      self.scene.fill(piece, color, Layer::AboveText, self.canvas.stage());
    }
  }

  fn line(&mut self, position: Position, color: theme::Paint) -> Result<()> {
    let text = self.source.page_text(position.page)?;
    let Some(quad) = text
      .lines
      .get(position.line)
      .and_then(|line| line.quad(0..line.glyphs.len()))
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
    color: theme::Paint,
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
