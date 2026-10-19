use super::{ScrollInput, View};
use crate::Result;
use crate::find::Search;
use crate::keys::{Action, Input};
use crate::source::Pages;
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::TextSource;

impl<S: TextSource + Pages> View<'_, S> {
  fn normal_input(
    &mut self,
    input: Input,
    search: &mut Search,
  ) -> Result<bool> {
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
        self.pan(
          direction
            * count as f32
            * f32::from(self.frame.pane.screen.cell_width),
        )?;
        return Ok(true);
      }
      Action::Motion(Motion::Down | Motion::Up) => {
        let direction =
          if input.action == Action::Motion(Motion::Up) { -1.0 } else { 1.0 };
        search.forget_yank();
        self.scroll(
          direction
            * count as f32
            * f32::from(self.frame.pane.screen.cell_height),
          ScrollInput::Wheel,
        )?;
        return Ok(true);
      }
      Action::Motion(
        Motion::First | Motion::Last | Motion::NextPage | Motion::PreviousPage,
      ) => {
        self.reader.cursor = self.position()?;
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

  pub fn navigate(
    &mut self,
    input: Input,
    search: &mut Search,
  ) -> Result<bool> {
    let count = input.count;
    let normal = self.reader.viewport_mode();
    let copying = self.reader.operator.is_some();
    if normal && self.normal_input(input, search)? {
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
        self.reader.motion(motion, count, input.explicit, &mut self.source)?;
        if let Some(origin) = origin {
          self.reader.record(origin);
        }
        if normal {
          *self.target = Some(self.reader.cursor);
        }
      }
      Action::Yank => {
        self.reader.yank(count, input.register, &mut self.source)?;
      }
      Action::YankLine => {
        self.reader.yank_lines(count, input.register, &mut self.source)?;
      }
      Action::YankRaw => {
        self.reader.yank_raw(count, &mut self.source)?;
      }
      Action::Visual(kind) => self.reader.visual(kind),
      Action::Object(kind, around) => {
        self.reader.select_object(kind, around, count, &mut self.source)?;
      }
      Action::Swap => self.reader.swap(),
      Action::Reselect => {
        self.reader.reselect();
        if self.reader.selection.is_none() {
          return Ok(true);
        }
      }
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
    self.reveal()?;
    Ok(true)
  }
}
