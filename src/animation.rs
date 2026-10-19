use crate::Result;
use crate::app::App;
use std::time::Instant;
use vpdf_ui::reading::ScrollInput;

impl App {
  pub fn animate(&mut self) -> bool {
    if !self.reader.animation().active() {
      return false;
    }
    self.reader.animate(Instant::now())
  }

  pub fn scroll(&mut self, delta: f32) -> Result<()> {
    self.search.forget_yank();
    self.reading_view().scroll(delta, ScrollInput::Wheel)
  }
}
