use crate::Result;
use crate::app::App;
use crate::keys::{Action, Input};
use crate::reader::SelectionKind;
use crate::terminal;
use crossterm::event::{self, MouseButton, MouseEventKind};
use std::time::{Duration, Instant};

impl App {
  pub fn resize(&mut self) -> Result<()> {
    self.screen = terminal::size().unwrap_or(self.screen);
    self.relayout()
  }

  /// Fits the page area between the panel, menu, and status rows.
  pub fn relayout(&mut self) -> Result<()> {
    let menu = self.prompt.as_ref().is_some_and(|p| p.menu.is_some());
    self.layout.screen = self.screen;
    self.layout.screen.columns = self.screen.columns.saturating_sub(self.left);
    self.layout.screen.rows = self.screen.rows.saturating_sub(u16::from(menu));
    self.layout.reflow(&self.doc.pages);
    self.ensure_cursor()
  }

  pub fn ensure_cursor(&mut self) -> Result<()> {
    self.animation = None;
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    let Some(glyph) = text.glyph(cursor) else {
      self.reader.scroll = self.layout.pages[cursor.page.0].top;
      return Ok(());
    };
    let rect = self.layout.project(cursor.page, glyph.bounds, 0.0);
    let margin = f32::from(self.screen.cell_height);
    if rect.y0 < self.reader.scroll + margin {
      self.reader.scroll = rect.y0 - margin;
    }
    if rect.y1 > self.reader.scroll + self.layout.screen.height() - margin {
      self.reader.scroll = rect.y1 - self.layout.screen.height() + margin;
    }
    if rect.x0 < 0.0 {
      self.layout.pan = (self.layout.pan + rect.x0).max(0.0);
    }
    if rect.x1 > self.layout.screen.width() {
      self.layout.pan += rect.x1 - self.layout.screen.width();
    }
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll);
    Ok(())
  }

  pub fn navigate(&mut self, input: Input) -> Result<bool> {
    let count = input.count;
    let previous_page = self.reader.cursor.page;
    match input.action {
      Action::Motion(motion) => {
        self.reader.motion(motion, count, input.explicit, &mut self.doc)?;
      }
      Action::Yank => {
        self.reader.yank(count, input.register, &mut self.doc)?;
      }
      Action::YankLine => {
        self.reader.yank(count, input.register, &mut self.doc)?;
        self.reader.yank(1, input.register, &mut self.doc)?;
      }
      Action::Visual(kind) => self.reader.visual(kind),
      Action::Object(around) => self.reader.object(around, &mut self.doc)?,
      Action::Swap => self.reader.swap(),
      Action::Reselect => self.reader.reselect(),
      Action::History(forward) => {
        self.animation = None;
        self.reader.history(forward);
        return Ok(true);
      }
      Action::Cancel => {
        self.reader.cancel();
        self.requested.clear();
      }
      Action::Quit => self.quit = true,
      _ => return Ok(false),
    }
    if self.reader.cursor.page != previous_page {
      self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
    }
    self.ensure_cursor()?;
    Ok(true)
  }

  pub fn viewport(&mut self, input: Input) -> Result<bool> {
    let count = input.count as f32;
    match input.action {
      Action::Scroll(fraction) => {
        self.scroll(fraction * count * self.layout.screen.height())?
      }
      Action::Rows(rows) => {
        self.scroll(rows as f32 * count * f32::from(self.screen.cell_height))?
      }
      Action::Align(fraction) => {
        self.animation = None;
        let text = self.doc.text(self.reader.cursor.page)?;
        if let Some(glyph) = text.glyph(self.reader.cursor) {
          let rect =
            self.layout.project(self.reader.cursor.page, glyph.bounds, 0.0);
          self.reader.scroll = self.layout.clamp_scroll(
            rect.y0 - fraction * (self.layout.screen.height() - rect.height()),
          );
        }
      }
      Action::Zoom(direction) => {
        self.layout.zoom = (self.layout.zoom
          * 1.15_f32.powf(direction as f32 * count))
        .clamp(0.1, 8.0);
        self.reflow()?;
      }
      Action::Fit => {
        self.layout.zoom = 1.0;
        self.layout.pan = 0.0;
        self.reflow()?;
      }
      Action::FitPage => {
        let page = &self.layout.pages[self.reader.cursor.page.0];
        self.layout.zoom *= (self.layout.screen.height() / page.height)
          .min(self.layout.screen.width() / page.width);
        self.layout.pan = 0.0;
        self.reflow()?;
      }
      Action::Rotate => {
        self.layout.rotation = (self.layout.rotation + 90) % 360;
        self.reflow()?;
      }
      Action::Pan(direction) => {
        self.animation = None;
        let max = self.layout.pages[self.reader.cursor.page.0].width
          - self.layout.screen.width();
        self.layout.pan = (self.layout.pan
          + direction as f32 * count * f32::from(self.screen.cell_width) * 4.0)
          .clamp(0.0, max.max(0.0));
      }
      _ => return Ok(false),
    }
    Ok(true)
  }

  fn reflow(&mut self) -> Result<()> {
    self.layout.reflow(&self.doc.pages);
    self.ensure_cursor()
  }

  pub fn mouse(&mut self, mouse: event::MouseEvent) -> Result<()> {
    if mouse.column < self.left && mouse.row < self.screen.rows - 1 {
      return self.panel_mouse(mouse);
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        return self.scroll(3.0 * f32::from(self.screen.cell_height));
      }
      MouseEventKind::ScrollUp => {
        return self.scroll(-3.0 * f32::from(self.screen.cell_height));
      }
      MouseEventKind::Down(MouseButton::Left)
      | MouseEventKind::Drag(MouseButton::Left) => {}
      _ => return Ok(()),
    }
    self.animation = None;
    if mouse.column < self.left || mouse.row >= self.screen.rows - 1 {
      return Ok(());
    }
    let pos = if self.text_mode {
      self.text_position(mouse.column, mouse.row)?
    } else {
      let y = f32::from(mouse.row) * f32::from(self.screen.cell_height)
        + self.reader.scroll;
      let page = self.layout.page_at(y);
      let layout = &self.layout.pages[page.0];
      let Some(inverse) = layout.matrix.invert() else { return Ok(()) };
      let x = f32::from(mouse.column - self.left)
        * f32::from(self.screen.cell_width)
        + self.layout.pan;
      let (x, y) = inverse.transform_xy(x, y - layout.top);
      self.doc.text(page)?.nearest(page, x, y)
    };
    if matches!(mouse.kind, MouseEventKind::Drag(_)) {
      if self.reader.selection.is_none() {
        self.reader.visual(SelectionKind::Character);
      }
      self.reader.cursor = pos;
    } else {
      self.reader.end_selection();
      self.reader.cursor = pos;
      if self.last_click.is_some_and(|(when, old)| {
        old == pos && when.elapsed() < Duration::from_millis(350)
      }) {
        self.reader.object(false, &mut self.doc)?;
      }
      self.last_click = Some((Instant::now(), pos));
    }
    Ok(())
  }
}
