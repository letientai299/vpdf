use super::View;
use crate::Result;
use crate::overlays::Overlays;
use crate::page::Point;
use crate::region::Region;
use crate::source::Pages;
use vpdf_reader::reader::TextSource;

impl<S: TextSource + Pages> View<'_, S> {
  pub fn figure_point(&mut self, region: Option<&Region>) -> Result<Point> {
    if let Some(region) = region {
      let selected = region.snapshot();
      return Ok(Point {
        page: selected.page,
        x: selected.bounds.x0,
        y: selected.bounds.y0,
      });
    }
    let position = self.position()?;
    let text = self.source.page_text(position.page)?;
    if let Some(glyph) = text.glyph(position) {
      return Ok(Point {
        page: position.page,
        x: glyph.bounds.x0,
        y: glyph.bounds.y0,
      });
    }
    let page = self.navigation().page;
    let (x, y) = self
      .layout
      .unproject(page, (0.0, 0.0), self.reader.scroll)
      .unwrap_or((0.0, 0.0));
    Ok(Point { page, x, y })
  }

  pub fn check_region(&mut self, overlays: &mut Overlays) {
    let region = &mut overlays.region;
    let Some(selected) =
      region.as_ref().filter(|region| !region.is_bar()).map(Region::snapshot)
    else {
      return;
    };
    let page = selected.page;
    let valid = page.0 < self.source.count()
      && crate::layout::clip(selected.bounds, self.source.bounds(page))
        .is_some();
    let visible = if self.frame.pane.text {
      self.text().pages().is_ok_and(|pages| pages.contains(&page.0))
    } else if valid {
      let rect = self.layout.project(page, selected.bounds, self.reader.scroll);
      rect.x1 > 0.0
        && rect.y1 > 0.0
        && rect.x0 < self.layout.screen().width()
        && rect.y0 < self.layout.screen().height()
    } else {
      false
    };
    if !valid || !visible {
      *region = None;
    }
  }
}
