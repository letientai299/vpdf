use super::{ScrollInput, View};
use crate::Result;
use crate::find::Search;
use crate::keys::{Action, Input};
use crate::layout::{Zoom, screen_step};
use crate::source::Pages;
use vpdf_reader::reader::TextSource;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewportEvent {
  Handled,
  Relayout,
  Crop,
  Unhandled,
}

impl<S: TextSource + Pages> View<'_, S> {
  pub fn viewport(
    &mut self,
    input: Input,
    search: &mut Search,
  ) -> Result<ViewportEvent> {
    let count = input.count as f32;
    match input.action {
      Action::Scroll(fraction) => {
        let delta = fraction * count * self.layout.screen().height();
        return self.scroll_key(delta, true, search);
      }
      Action::Screen(direction) => {
        let row = f32::from(self.frame.pane.screen.cell_height);
        let rows = usize::from(self.layout.screen().rows.saturating_sub(1));
        let distance = screen_step(rows) as f32 * row;
        return self.scroll_key(
          direction as f32 * count * distance,
          true,
          search,
        );
      }
      Action::Rows(rows) => {
        let delta =
          rows as f32 * count * f32::from(self.frame.pane.screen.cell_height);
        return self.scroll_key(delta, false, search);
      }
      Action::Align(fraction) => {
        self.align(fraction)?;
        return Ok(ViewportEvent::Handled);
      }
      Action::Zoom(direction) => {
        self.layout.request_zoom(Zoom::Steps(direction as f32 * count));
      }
      Action::Fit => self.layout.request_zoom(Zoom::Width),
      Action::FitPage => {
        self.layout.request_zoom(Zoom::Page(self.navigation().page));
      }
      Action::Rotate => self.layout.rotate(),
      Action::Crop => return Ok(ViewportEvent::Crop),
      _ => return Ok(ViewportEvent::Unhandled),
    }
    Ok(ViewportEvent::Relayout)
  }

  fn scroll_key(
    &mut self,
    delta: f32,
    follow: bool,
    search: &mut Search,
  ) -> Result<ViewportEvent> {
    search.forget_yank();
    self.scroll(delta, ScrollInput::Keyboard { follow })?;
    Ok(ViewportEvent::Handled)
  }
}
