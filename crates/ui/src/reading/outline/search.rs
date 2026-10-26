use super::{Outline, Panel, Tree};

impl Outline {
  pub fn preview_search(&self, panel: &mut Panel, query: &str) {
    self.project(panel);
    if panel.preview(query) {
      let mut closed = self.closed.clone();
      Self::reveal_path(&self.tree, panel.selected(), &mut closed);
      self.project_folds(panel, &closed);
      panel.reveal();
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
    Self::reveal_path(&self.tree, panel.selected(), &mut self.closed);
    self.auto = false;
    self.commit_preferences();
    self.project(panel);
    panel.reveal();
  }

  fn reveal_path(tree: &Tree, source: usize, closed: &mut [bool]) {
    let mut parent = tree.nodes().get(source).and_then(|node| node.parent);
    while let Some(source) = parent {
      closed[source] = false;
      parent = tree.nodes()[source].parent;
    }
  }
}
