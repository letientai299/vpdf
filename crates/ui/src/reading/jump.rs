use super::View;
use crate::Result;
use crate::find::Search;
use crate::page::landing::{Landed, Span};
use crate::source::Pages;
use crate::text::{PageId, Position};
use std::time::Instant;
use vpdf_reader::reader::TextSource;

impl<S: TextSource + Pages> View<'_, S> {
  pub fn destination(
    &mut self,
    page: PageId,
    point: Option<(f32, f32)>,
  ) -> Result<Position> {
    let page = PageId(page.0.min(self.source.count() - 1));
    Ok(match point {
      Some((x, y)) => self.source.page_text(page)?.nearest(page, x, y),
      None => Position { page, ..Position::default() },
    })
  }

  pub fn go_to_page(&mut self, page: PageId) -> Result<()> {
    self.jump_cursor(Position { page, ..Position::default() })?;
    self.show_page(self.reader.cursor.page)?;
    self.reveal()?;
    self.flash_landing();
    Ok(())
  }

  pub fn land_at(&mut self, position: Position) -> Result<()> {
    let position = vpdf_reader::reader::clamp(position, &mut self.source)?;
    // Repeated landings keep the view.
    let here = self.landing.matches(self.landed_view(position))
      && self.reader.cursor == position;
    if !here {
      self.jump_cursor(position)?;
      self.land()?;
      self.landing.record(self.landed_view(self.reader.cursor));
    }
    self.flash_landing();
    Ok(())
  }

  pub fn matches_landing(&self, position: Position) -> bool {
    self.landing.matches(self.landed_view(position))
  }

  pub fn recall(
    &mut self,
    forward: bool,
  ) -> Result<Option<vpdf_reader::reader::Location>> {
    let current = self.location()?;
    Ok(self.reader.recall(forward, current))
  }

  pub fn restore_history(
    &mut self,
    location: vpdf_reader::reader::Location,
  ) -> Result<()> {
    self.restore_location(location)?;
    *self.target = Some(self.reader.cursor);
    self.flash_landing();
    Ok(())
  }

  pub(super) fn landed_view(&self, target: Position) -> Landed {
    Landed::new(target, self.reader.scroll, self.layout.pan(), self.text.top())
  }

  fn jump_cursor(&mut self, target: Position) -> Result<()> {
    let origin = self.location()?;
    self.reader.cursor = vpdf_reader::reader::clamp(target, &mut self.source)?;
    self.reader.record(origin);
    self.reader.describe_jump(None);
    self.reader.desired = None;
    *self.target = Some(self.reader.cursor);
    Ok(())
  }

  pub(super) fn flash_landing(&mut self) {
    self.landing.show(self.reader.cursor, Span::Line, Instant::now());
  }

  pub fn finish_search_yank(&mut self, search: &mut Search) -> Result<()> {
    let Some((origin, operator, target)) = search.take_yank() else {
      return Ok(());
    };
    if target != origin.cursor {
      let first = target.min(origin.cursor);
      self.reader.cursor = target.max(origin.cursor);
      if self.reader.step(-1, false, &mut self.source)? {
        self.reader.yank_range(
          first,
          self.reader.cursor,
          false,
          operator.register,
          &mut self.source,
        )?;
      }
    }
    self.restore_location(origin)
  }
}
