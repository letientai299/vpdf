use crate::app::App;
use std::time::{Duration, Instant};
use vpdf_ui::pointer::Items;

impl App {
  pub fn clue_due(&mut self) -> bool {
    if self.prompt.is_some() || !self.clues.pending() {
      return false;
    }
    if !self.clues.poll(Instant::now()) {
      return false;
    }
    let Some(prefix) = self.keys.awaiting().map(str::to_owned) else {
      return false;
    };
    let registers = self
      .reader
      .registers
      .entries()
      .map(|(&name, text)| (name, text.as_ref()));
    let marks = self.marks.iter().map(|(&name, &at)| (name, at));
    self.clues.show(vpdf_ui::clues::items(&prefix, registers, marks))
  }

  pub fn clue_wait(&self) -> Option<Duration> {
    if self.prompt.is_some() || !self.clues.pending() {
      return None;
    }
    self.clues.wait(Instant::now())
  }

  pub fn after_key(&mut self) {
    self.clues.restart(self.keys.awaiting().map(|_| Instant::now()));
  }

  pub(crate) fn scroll_clues(&mut self, rows: isize) {
    self.clues.scroll(rows, self.screen);
  }

  pub fn clue_rows(&self) -> u16 {
    self.clues.rows(self.screen)
  }

  pub(crate) fn clue_cells(&self) -> Items {
    self.clues.shown().map_or_else(Items::default, |list| {
      list.cells(self.screen, self.menu_top())
    })
  }
}
