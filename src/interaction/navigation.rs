use super::{Action, App, Input, Motion, Result, screen_step};
use crate::pdf_text::PdfText;

impl App {
  fn normal_input(&mut self, input: Input) -> Result<bool> {
    let count = input.count;
    match input.action {
      Action::Cursor => {
        if self.enter_text()? {
          self.reader.enter_cursor();
        }
        return Ok(true);
      }
      Action::Motion(Motion::Left | Motion::Right) => {
        let direction =
          if input.action == Action::Motion(Motion::Left) { -1.0 } else { 1.0 };
        self
          .pan(direction * count as f32 * f32::from(self.screen.cell_width))?;
        return Ok(true);
      }
      Action::Motion(Motion::Down | Motion::Up) => {
        let direction =
          if input.action == Action::Motion(Motion::Up) { -1.0 } else { 1.0 };
        self.scroll(
          direction * count as f32 * f32::from(self.screen.cell_height),
        )?;
        return Ok(true);
      }
      Action::Motion(
        Motion::First | Motion::Last | Motion::NextPage | Motion::PreviousPage,
      ) => {
        self.reader.cursor = self.reading_position()?;
      }
      Action::Motion(_) | Action::Object(..) | Action::Swap => {
        return Ok(true);
      }
      Action::Visual(_) | Action::Yank | Action::YankLine => {
        if !self.enter_text()? {
          return Ok(true);
        }
      }
      _ => {}
    }
    Ok(false)
  }

  pub fn navigate(&mut self, input: Input) -> Result<bool> {
    let count = input.count;
    let normal = self.reader.viewport_mode();
    let copying = self.reader.operator.is_some();
    if normal && self.normal_input(input)? {
      return Ok(true);
    }
    let previous_page = self.reader.cursor.page;
    match input.action {
      Action::Motion(motion) => {
        let boundary = matches!(
          motion,
          Motion::First
            | Motion::Last
            | Motion::NextPage
            | Motion::PreviousPage
        );
        let origin = if boundary && self.reader.operator.is_none() {
          Some(self.location()?)
        } else {
          None
        };
        self.reader.motion(
          motion,
          count,
          input.explicit,
          &mut PdfText(&mut self.doc),
        )?;
        if let Some(origin) = origin {
          self.reader.record(origin);
        }
        if normal {
          self.target = Some(self.reader.cursor);
        }
      }
      Action::Section(forward) => {
        self.move_section(forward, count)?;
        return Ok(true);
      }
      Action::Yank => {
        self.reader.yank(count, input.register, &mut PdfText(&mut self.doc))?;
      }
      Action::YankLine => {
        self.reader.yank_lines(
          count,
          input.register,
          &mut PdfText(&mut self.doc),
        )?;
      }
      Action::YankRaw => {
        self.reader.yank_raw(count, &mut PdfText(&mut self.doc))?;
      }
      Action::Visual(kind) => self.reader.visual(kind),
      Action::Object(kind, around) => {
        self.reader.select_object(
          kind,
          around,
          count,
          &mut PdfText(&mut self.doc),
        )?;
      }
      Action::Swap => self.reader.swap(),
      Action::Reselect => {
        self.reader.reselect();
        if self.reader.selection.is_none() {
          return Ok(true);
        }
      }
      Action::History(forward) => {
        self.animation = None;
        for _ in 0..count {
          let current = self.location()?;
          let Some(location) = self.reader.recall(forward, current) else {
            break;
          };
          self.close_overview();
          self.restore_location(location)?;
          self.target = Some(self.reader.cursor);
          self.flash_landing();
        }
        return Ok(true);
      }
      Action::Quit => self.quit = true,
      _ => return Ok(false),
    }
    if matches!(
      input.action,
      Action::Yank | Action::YankLine | Action::YankRaw | Action::Visual(_)
    ) || (self.reader.selection.is_none()
      && copying
      && matches!(input.action, Action::Motion(_) | Action::Object(..)))
    {
      return Ok(true);
    }
    if self.reader.cursor.page != previous_page {
      self.show_page_of(self.reader.cursor)?;
    }
    self.ensure_cursor()?;
    Ok(true)
  }

  pub fn viewport(&mut self, input: Input) -> Result<bool> {
    let count = input.count as f32;
    match input.action {
      Action::Scroll(fraction) => {
        self.keyboard_scroll(
          fraction * count * self.layout.screen.height(),
          true,
        )?;
      }
      Action::Screen(direction) => {
        let row = f32::from(self.screen.cell_height);
        let rows = usize::from(self.layout.screen.rows.saturating_sub(1));
        let distance = screen_step(rows) as f32 * row;
        self.keyboard_scroll(direction as f32 * count * distance, true)?;
      }
      Action::Rows(rows) => {
        self.keyboard_scroll(
          rows as f32 * count * f32::from(self.screen.cell_height),
          false,
        )?;
      }
      Action::Align(fraction) if self.text_mode => {
        if self.reader.viewport_mode() {
          self.reader.cursor = self.reading_position()?;
        }
        self.align_text(fraction)?;
      }
      Action::Align(fraction) => {
        self.animation = None;
        if self.reader.viewport_mode() {
          self.reader.cursor = self.reading_position()?;
        }
        if let Some(rect) = self.glyph_rect(self.reader.cursor)? {
          self.reader.scroll = self.layout.clamp_scroll(
            rect.y0 - fraction * (self.layout.screen.height() - rect.height()),
          );
        }
      }
      Action::Zoom(direction) => self.zoom_by(direction as f32 * count)?,
      Action::Fit => {
        self.layout.zoom = 1.0;
        self.relayout()?;
      }
      Action::FitPage => {
        let page = &self.layout.pages[self.navigation_position().page.0];
        self.layout.zoom *= (self.layout.screen.height() / page.height)
          .min(self.layout.screen.width() / page.width);
        self.relayout()?;
      }
      Action::Crop => self.toggle_crop()?,
      Action::Rotate => {
        self.layout.rotation = (self.layout.rotation + 90) % 360;
        self.relayout()?;
      }
      _ => return Ok(false),
    }
    Ok(true)
  }

  /// Zooms by `steps` of 15 %.
  fn zoom_by(&mut self, steps: f32) -> Result<()> {
    self.layout.zoom =
      (self.layout.zoom * 1.15_f32.powf(steps)).clamp(0.1, 8.0);
    self.relayout()
  }

  /// Zooms keeping the page point at a page-area pixel in place, where
  /// the view allows.
  pub(super) fn zoom_at(&mut self, steps: f32, at: (f32, f32)) -> Result<()> {
    let anchor = self.anchor_at(at);
    self.zoom_by(steps)?;
    self.restore_at(anchor, at);
    Ok(())
  }

  pub(crate) fn pan(&mut self, delta: f32) -> Result<()> {
    if self.text_mode {
      let columns = (delta / f32::from(self.screen.cell_width)).round();
      return self.pan_text(columns as isize);
    }
    let page = self.navigation_position().page;
    let max = self.layout.pan_limit(page);
    self.layout.pan = (self.layout.pan + delta).clamp(0.0, max);
    Ok(())
  }
}
