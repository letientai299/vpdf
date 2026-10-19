use crate::Result;
use crate::layout::Layout;
use crate::page::{Viewport, scroll::Scroll};
use crate::paint;
use crate::source::Pages;
use crate::text::Position;
use crate::text_view::{self, TextView};
use std::time::Instant;
use vpdf_model::geometry::Screen;
use vpdf_reader::reader::{Mode, Reader, TextSource};

mod hints;
mod hit;
pub use hit::{HitFrame, glyph_point, link_at};
mod jump;
mod location;
mod navigation;
mod outline;
pub use outline::Outline;
mod pane;
mod pointer;
mod preview;
mod reflow;
mod region;
pub use pane::{Pane, Tracking};
mod search;
pub use search::{RestoreSearch, Submission};
mod section;
mod transitions;
mod viewport;
pub use section::{Section, Sections};
pub use viewport::ViewportEvent;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy)]
pub struct Frame {
  pub pane: paint::Frame,
  pub visible: Screen,
}

#[derive(Clone, Copy)]
pub enum ScrollInput {
  Wheel,
  Keyboard { follow: bool },
}

pub struct View<'a, S> {
  reader: &'a mut Reader,
  target: &'a mut Option<Position>,
  animation: &'a mut Scroll,
  landing: &'a mut crate::page::landing::Landing,
  layout: &'a mut Layout,
  text: &'a mut TextView,
  frame: Frame,
  source: S,
}

pub fn navigation(
  reader: &Reader,
  layout: &Layout,
  text_top: Position,
  text: bool,
) -> Position {
  if !reader.viewport_mode() {
    return reader.cursor;
  }
  if text {
    return Position { glyph: 0, ..text_top };
  }
  Position { page: layout.page_at(reader.scroll), ..Position::default() }
}

impl<S: TextSource + Pages> View<'_, S> {
  fn text(&mut self) -> text_view::Pane<'_, &mut S> {
    let frame = self.frame.pane;
    self.text.bind(
      &mut self.source,
      text_view::Frame { screen: frame.screen, at: frame.at, left: frame.left },
    )
  }

  fn page(&mut self) -> Viewport<'_, &mut S> {
    Viewport::new(
      self.layout,
      &mut self.reader.scroll,
      &mut self.source,
      self.frame.visible,
    )
  }

  pub fn visible(&mut self, position: Position) -> Result<bool> {
    if self.frame.pane.text {
      return Ok(self.text().cell(position)?.is_some());
    }
    self.page().visible(position)
  }

  pub fn first(&mut self) -> Result<Option<Position>> {
    if self.frame.pane.text {
      return self.text().first();
    }
    self.page().first()
  }

  pub fn position(&mut self) -> Result<Position> {
    if !self.reader.viewport_mode() {
      return Ok(self.reader.cursor);
    }
    if let Some(target) = *self.target
      && self.visible(target)?
    {
      return Ok(target);
    }
    Ok(self.first()?.unwrap_or_else(|| self.navigation()))
  }

  fn navigation(&self) -> Position {
    navigation(self.reader, self.layout, self.text.top(), self.frame.pane.text)
  }

  pub fn explicit_target(&mut self) -> Result<Option<Position>> {
    let target = if self.reader.uses_cursor() {
      Some(self.reader.cursor)
    } else {
      *self.target
    };
    if let Some(target) = target
      && self.visible(target)?
    {
      return Ok(Some(target));
    }
    self.reader.message = "Click text or jump to select a target".into();
    Ok(None)
  }

  pub fn enter_text(&mut self) -> Result<bool> {
    if let Some(target) = *self.target
      && self.visible(target)?
    {
      self.reader.cursor = target;
      self.reader.desired = None;
      self.reader.message.clear();
      return Ok(true);
    }
    if let Some(position) = self.first()? {
      self.reader.cursor = position;
      self.reader.desired = None;
      self.reader.message.clear();
      return Ok(true);
    }
    self.reader.message = "Selection unavailable: no visible text".into();
    Ok(false)
  }

  pub fn reveal(&mut self) -> Result<()> {
    self.animation.cancel();
    let cursor = self.reader.cursor;
    if self.frame.pane.text {
      return self.text().reveal(cursor);
    }
    let margin = f32::from(self.frame.pane.screen.cell_height);
    self.page().reveal(cursor, margin)
  }

  pub fn land(&mut self) -> Result<()> {
    let cursor = self.reader.cursor;
    if self.frame.pane.text {
      return self.text().land(cursor);
    }
    self.animation.cancel();
    let margin = f32::from(self.frame.pane.screen.cell_height);
    self.page().land(cursor, margin)
  }

  pub fn align(&mut self, fraction: f32) -> Result<()> {
    if !self.frame.pane.text {
      self.animation.cancel();
    }
    if self.reader.viewport_mode() {
      self.reader.cursor = self.position()?;
    }
    let cursor = self.reader.cursor;
    if self.frame.pane.text {
      return self.text().align(cursor, fraction);
    }
    self.page().align(cursor, fraction)
  }

  pub fn pan(&mut self, delta: f32) -> Result<()> {
    if self.frame.pane.text {
      let columns =
        (delta / f32::from(self.frame.pane.screen.cell_width)).round();
      return self.text().pan(columns as isize);
    }
    let page = self.navigation().page;
    self.layout.pan_by(page, delta);
    Ok(())
  }

  fn cursor_point(&mut self) -> Result<(f32, f32)> {
    let cursor = self.reader.cursor;
    if self.frame.pane.text {
      let left = self.frame.pane.left;
      let (column, row) = self.text().cell(cursor)?.unwrap_or((left, 0));
      return Ok((f32::from(column), f32::from(row)));
    }
    self.page().point(cursor)
  }

  fn nearest(&mut self, point: (f32, f32)) -> Result<Option<Position>> {
    if self.frame.pane.text {
      return self.text().nearest(point);
    }
    self.page().nearest(point)
  }

  pub fn scroll(&mut self, delta: f32, input: ScrollInput) -> Result<()> {
    let (follow, sync) = match input {
      ScrollInput::Wheel => (false, self.reader.mode() == Mode::Cursor),
      ScrollInput::Keyboard { follow } => (follow, self.reader.uses_cursor()),
    };
    self.reader.disarm();
    let displayed = self.reader.scroll;
    if let Some(target) = self.animation.finish() {
      self.reader.scroll = target;
    }
    let mut point = if sync { self.cursor_point()? } else { (0.0, 0.0) };
    if self.frame.pane.text {
      let rows =
        (delta / f32::from(self.frame.pane.screen.cell_height)).round();
      self.text().scroll(rows as isize)?;
      if !follow {
        point.1 -= rows;
      }
    } else {
      let start = self.reader.scroll;
      let target = self.layout.clamp_scroll(start + delta);
      self.reader.scroll = target;
      if !follow {
        point.1 -= target - start;
      }
      self.animation.start(target, Instant::now());
    }
    if sync
      && (follow || !self.visible(self.reader.cursor)?)
      && let Some(cursor) = self.nearest(point)?
    {
      self.reader.cursor = cursor;
      self.reader.desired = None;
      *self.target = Some(cursor);
    }
    if !self.frame.pane.text {
      self.reader.scroll = displayed;
    }
    Ok(())
  }
}
