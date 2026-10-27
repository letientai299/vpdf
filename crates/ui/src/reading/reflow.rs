use super::View;
use crate::Result;
use crate::page::Point;
use crate::source::Pages;
use crate::text::{Bounds, PageId};
use vpdf_reader::reader::TextSource;

impl<S: TextSource + Pages> View<'_, S> {
  pub fn resize(
    &mut self,
    screen: vpdf_model::geometry::Screen,
    bar: &mut crate::scrollbar::Bar,
  ) -> Result<()> {
    let landed = self.target.filter(|&target| self.matches_landing(target));
    let aimed = landed.and_then(|_| self.landing.aimed().cloned());
    let anchor = self
      .layout
      .anchor_at((self.layout.screen().width() / 2.0, 0.0), self.reader.scroll);
    let scrolled = self.reader.scroll;
    let before = self.layout.checkpoint(anchor.0);
    self.layout.resize(
      screen,
      (0..self.source.count()).map(|page| self.source.bounds(PageId(page))),
    );
    if self.frame.pane.text {
      self.text().reclamp()?;
    }
    if before == self.layout.checkpoint(anchor.0) {
      self.clamp_scroll();
      self.layout.clamp_pan(anchor.0);
    } else {
      self.restore_at(anchor, (self.layout.screen().width() / 2.0, 0.0));
    }
    bar.reflow(scrolled, self.reader.scroll);
    if let Some(target) = landed {
      self.landing.record(self.landed_view(target));
      if let Some(aimed) = aimed {
        self.landing.aim(aimed);
      }
    }
    Ok(())
  }

  pub fn suspend_scroll(&mut self) -> Option<Point> {
    let target = self.animation.finish()?;
    let page = self.layout.page_at(target);
    let (x, y) = self.layout.unproject(
      page,
      (self.layout.screen().width() / 2.0, 0.0),
      target,
    )?;
    Some(Point { page, x, y })
  }

  pub fn resume_scroll(&mut self, Point { page, x, y }: Point) {
    let bounds = Bounds { x0: x, y0: y, x1: x, y1: y };
    let rect = self.layout.project(page, bounds, 0.0);
    let shown = &self.layout.pages()[page.0];
    let target = self
      .layout
      .clamp_scroll(rect.y0.clamp(shown.top, shown.top + shown.height));
    self.animation.start(target, std::time::Instant::now());
  }
}
