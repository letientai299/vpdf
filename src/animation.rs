use crate::Result;
use crate::app::App;
use std::time::Instant;
use vpdf_ui::reading::ScrollInput;

impl App {
  pub fn animate(&mut self) -> bool {
    if !self.animation.active() {
      return false;
    }
    let Some(scroll) = self.animation.step(self.reader.scroll, Instant::now())
    else {
      return false;
    };
    self.reader.scroll = scroll;
    true
  }

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
    self.search.forget_yank();
    self.reading_view().scroll(delta, ScrollInput::Wheel)
  }

  pub(crate) fn keyboard_scroll(
    &mut self,
    delta: f32,
    follow: bool,
  ) -> Result<()> {
    self.search.forget_yank();
    self.reading_view().scroll(delta, ScrollInput::Keyboard { follow })
  }
}
