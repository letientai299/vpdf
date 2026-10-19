use super::View;
use crate::Result;
use crate::input::gesture::Gesture;
use crate::layout::Fit;
use crate::page::landing::Span;
use crate::source::Pages;
use crate::text::{Bounds, PageId, Position};
use crate::text_view::TextView;
use std::time::Instant;
use vpdf_reader::reader::{self, Location, Mode, SelectionKind, TextSource};

impl<S: TextSource + Pages> View<'_, S> {
  pub fn clamp_scroll(&mut self) {
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll);
  }

  pub fn grab_page(&mut self, delta: f32, pan: f32) -> Result<()> {
    self.animation.cancel();
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll - delta);
    self.pan(pan)
  }

  pub fn edge_scroll(&mut self, rows: isize) -> Result<()> {
    if self.frame.pane.text {
      self.text().scroll(rows)?;
      self.text.invalidate();
      return Ok(());
    }
    let delta = rows as f32 * f32::from(self.frame.pane.screen.cell_height);
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll + delta);
    Ok(())
  }

  pub fn reclamp_cursor(&mut self) -> Result<()> {
    self.reader.cursor = reader::clamp(self.reader.cursor, &mut self.source)?;
    Ok(())
  }

  pub fn reclamp_text(&mut self) -> Result<()> {
    let cursor = self.reader.cursor;
    self.reader.cursor = self.source.page_text(cursor.page)?.clamp(cursor);
    Ok(())
  }

  pub fn restore_position(&mut self, position: Position) -> Result<()> {
    let page = PageId(position.page.0.min(self.source.count() - 1));
    let cursor = Position { page, ..position };
    self.reader.cursor = self.source.page_text(page)?.clamp(cursor);
    self.show_page(page)?;
    *self.target = Some(self.reader.cursor);
    self.reveal()
  }

  pub fn open_page(&mut self, page: PageId) -> Result<()> {
    self.reader.edge(page, false, &mut self.source)?;
    self.show_page(self.reader.cursor.page)?;
    self.reveal()
  }

  pub fn mirror(
    &mut self,
    shown: Position,
    scroll: f32,
    text: &TextView,
  ) -> Result<()> {
    self.reader.cursor = reader::clamp(shown, &mut self.source)?;
    self.reader.cancel();
    *self.target = None;
    self.reader.scroll = scroll;
    self.text.copy_origin(text);
    Ok(())
  }

  pub fn preview(&mut self, origin: Location, target: Position) -> Result<()> {
    self.restore_location(origin)?;
    self.reader.cursor = target;
    *self.target = Some(target);
    self.land()
  }

  pub fn align_region(&mut self, page: PageId, bounds: Bounds) {
    if !self.frame.pane.text {
      let rect = self.layout.project(page, bounds, 0.0);
      self.reader.scroll = self.layout.clamp_scroll(rect.y0);
    }
  }

  pub fn finish_fit(&mut self, fit: Fit) {
    self.reader.scroll = fit.finish(self.layout);
    self.animation.cancel();
  }

  pub fn scrub_scroll(&mut self, scroll: f32) {
    self.animation.cancel();
    self.reader.scroll = scroll;
  }

  pub fn click_select(
    &mut self,
    gesture: &mut Gesture,
    position: Position,
    clicks: u8,
  ) -> Result<()> {
    *self.target = Some(position);
    if gesture.click(self.reader, position, clicks, &mut self.source)? {
      self.landing.show(position, Span::Glyph, Instant::now());
    }
    Ok(())
  }

  pub fn drag_select(
    &mut self,
    gesture: &mut Gesture,
    position: Position,
  ) -> Result<()> {
    gesture.drag(self.reader, position, &mut self.source)
  }

  pub fn extend_select(
    &mut self,
    gesture: &mut Gesture,
    position: Position,
  ) -> Result<()> {
    gesture.reset();
    if self.reader.selection.is_none() {
      let cursor = self.reader.mode() == Mode::Cursor
        && self.visible(self.reader.cursor)?;
      if !cursor && !self.enter_text()? {
        return Ok(());
      }
      self.reader.visual(SelectionKind::Character);
    }
    self.reader.cursor = position;
    self.reader.desired = None;
    Ok(())
  }
}
