use super::View;
use crate::Result;
use crate::source::Pages;
use crate::text::{PageId, Position};
use vpdf_reader::reader::{Location, TextSource};

impl<S: TextSource + Pages> View<'_, S> {
  pub fn location(&mut self) -> Result<Location> {
    Ok(Location {
      cursor: self.position()?,
      scroll: self.reader.scroll,
      anchor: Some(self.anchor_at((self.layout.screen().width() / 2.0, 0.0))),
      text_top: self.frame.pane.text.then_some(self.text.top()),
      target: *self.target,
    })
  }

  pub fn restore_location(&mut self, location: Location) -> Result<()> {
    self.animation.cancel();
    self.reader.cursor =
      vpdf_reader::reader::clamp(location.cursor, &mut self.source)?;
    self.reader.desired = None;
    *self.target = location.target;
    // Other presentations restore only their position.
    if location.text_top.is_some() != self.frame.pane.text {
      return self.land();
    }
    match location.anchor {
      Some(anchor) => {
        self.restore_at(anchor, (self.layout.screen().width() / 2.0, 0.0));
      }
      None => self.reader.scroll = self.layout.clamp_scroll(location.scroll),
    }
    if let Some(top) = location.text_top {
      self.text().restore(top)?;
    }
    Ok(())
  }

  fn anchor_at(&self, at: (f32, f32)) -> (PageId, (f32, f32)) {
    self.layout.anchor_at(at, self.reader.scroll)
  }

  pub fn restore_at(
    &mut self,
    (page, (x, y)): (PageId, (f32, f32)),
    (left, top): (f32, f32),
  ) {
    self.animation.cancel();
    self.reader.scroll = self.layout.restore_at(page, (x, y), (left, top));
  }

  pub fn show_page(&mut self, page: PageId) -> Result<()> {
    self.reader.scroll = self.layout.pages()[page.0].top;
    if self.frame.pane.text {
      let top = Position { page, ..Position::default() };
      self.text().restore(top)?;
    }
    Ok(())
  }

  pub(super) fn show_page_of(&mut self, position: Position) -> Result<()> {
    if self.frame.pane.text && self.text().row(position)?.is_some() {
      return Ok(());
    }
    self.show_page(position.page)
  }
}
