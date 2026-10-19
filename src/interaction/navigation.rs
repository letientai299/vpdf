use super::{Action, App, Input, Result};
use vpdf_ui::reading::ViewportEvent;

impl App {
  pub fn navigate(&mut self, input: Input) -> Result<bool> {
    match input.action {
      Action::Section(forward) => self.move_section(forward, input.count)?,
      Action::History(forward) => {
        self.reader.cancel_scroll();
        for _ in 0..input.count {
          let Some(location) = self.reading_view().recall(forward)? else {
            break;
          };
          self.close_overview();
          self.reading_view().restore_history(location)?;
        }
      }
      Action::Quit => {
        self.quit = true;
        self.ensure_cursor()?;
      }
      _ => {
        let mut reading = self.reading_parts();
        return reading.view.navigate(input, reading.search);
      }
    }
    Ok(true)
  }

  pub fn viewport(&mut self, input: Input) -> Result<bool> {
    let mut reading = self.reading_parts();
    match reading.view.viewport(input, reading.search)? {
      ViewportEvent::Handled => {}
      ViewportEvent::Relayout => self.relayout()?,
      ViewportEvent::Crop => self.toggle_crop()?,
      ViewportEvent::Unhandled => return Ok(false),
    }
    Ok(true)
  }

  /// Zooms by `steps` of 15 %.
  fn zoom_by(&mut self, steps: f32) -> Result<()> {
    self.layout.request_zoom(vpdf_ui::layout::Zoom::Steps(steps));
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
    self.reading_view().pan(delta)
  }
}
