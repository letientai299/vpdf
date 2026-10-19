use super::{Action, App, Input, Result, screen_step};

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
    let count = input.count as f32;
    match input.action {
      Action::Scroll(fraction) => {
        self.keyboard_scroll(
          fraction * count * self.layout.screen().height(),
          true,
        )?;
      }
      Action::Screen(direction) => {
        let row = f32::from(self.screen.cell_height);
        let rows = usize::from(self.layout.screen().rows.saturating_sub(1));
        let distance = screen_step(rows) as f32 * row;
        self.keyboard_scroll(direction as f32 * count * distance, true)?;
      }
      Action::Rows(rows) => {
        self.keyboard_scroll(
          rows as f32 * count * f32::from(self.screen.cell_height),
          false,
        )?;
      }
      Action::Align(fraction) => self.reading_view().align(fraction)?,
      Action::Zoom(direction) => self.zoom_by(direction as f32 * count)?,
      Action::Fit => {
        self.layout.request_zoom(vpdf_ui::layout::Zoom::Width);
        self.relayout()?;
      }
      Action::FitPage => {
        self.layout.request_zoom(vpdf_ui::layout::Zoom::Page(
          self.navigation_position().page,
        ));
        self.relayout()?;
      }
      Action::Crop => self.toggle_crop()?,
      Action::Rotate => {
        self.layout.rotate();
        self.relayout()?;
      }
      _ => return Ok(false),
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
