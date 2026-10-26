use super::{Outline, Panel};

impl Outline {
  pub fn preview_search(&self, panel: &mut Panel, query: &str) {
    self.project(panel);
    if panel.preview(query) {
      self.folds.preview(panel, Some(self.auto));
    }
  }

  pub fn finish_search(&mut self, panel: &mut Panel, query: &str) -> bool {
    self.project(panel);
    let found = panel.finish_search(query);
    self.commit_match(panel, found);
    found
  }

  pub fn abort_search(&self, panel: &mut Panel) {
    self.project(panel);
    panel.abort_search();
  }

  pub fn repeat_search(
    &mut self,
    panel: &mut Panel,
    reverse: bool,
    count: usize,
  ) {
    let found = panel.repeat(reverse, count);
    self.commit_match(panel, found);
  }

  fn commit_match(&mut self, panel: &mut Panel, found: bool) {
    if !found {
      return;
    }
    self.folds.reveal(panel.selected());
    self.auto = false;
    self.commit_preferences();
    self.project(panel);
    panel.reveal();
  }
}
