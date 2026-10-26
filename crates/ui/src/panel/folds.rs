use super::Panel;
use super::tree::Tree;
use crate::keys::{Fold, PanelKind};

/// Closed branches over a panel's tree.
#[derive(Debug, Default)]
pub struct Folds {
  tree: Tree,
  closed: Vec<bool>,
}

impl Folds {
  pub fn new(tree: Tree) -> Self {
    let closed = vec![false; tree.nodes().len()];
    Self { tree, closed }
  }

  pub fn tree(&self) -> &Tree {
    &self.tree
  }

  pub fn closed(&self, source: usize) -> bool {
    self.closed.get(source).copied().unwrap_or(false)
  }

  pub fn closed_sources(&self) -> impl Iterator<Item = usize> + '_ {
    self
      .closed
      .iter()
      .enumerate()
      .filter_map(|(source, &closed)| closed.then_some(source))
  }

  /// The row showing `source`: its outermost closed ancestor, else itself.
  pub fn visible(&self, source: usize) -> Option<usize> {
    let mut node = self.tree.nodes().get(source)?;
    let mut visible = source;
    while let Some(parent) = node.parent {
      if self.closed(parent) {
        visible = parent;
      }
      node = &self.tree.nodes()[parent];
    }
    Some(visible)
  }

  /// Closes a branch; leaves and unknown rows stay as they are.
  pub fn close(&mut self, source: usize) {
    if self.tree.nodes().get(source).is_some_and(|node| node.descendants > 0) {
      self.closed[source] = true;
    }
  }

  pub fn expand_all(&mut self) {
    self.closed.fill(false);
  }

  fn collapse_all(&mut self) {
    for (closed, node) in self.closed.iter_mut().zip(self.tree.nodes()) {
      *closed = node.descendants > 0;
    }
  }

  /// Closes every branch off the path to `source`.
  pub fn focus(&mut self, source: Option<usize>) {
    self.collapse_all();
    let mut source = source;
    while let Some(index) = source {
      let Some(node) = self.tree.nodes().get(index) else { break };
      self.closed[index] = false;
      source = node.parent;
    }
  }

  pub fn reveal(&mut self, source: usize) {
    reveal(&self.tree, source, &mut self.closed);
  }

  /// Applies `operation` and projects; `h` and `l` move the selection.
  pub fn fold(
    &mut self,
    panel: &mut Panel,
    (operation, count): (Fold, usize),
    auto: Option<bool>,
  ) {
    match operation {
      Fold::ExpandAll => self.expand_all(),
      Fold::CollapseAll => self.collapse_all(),
      Fold::Close | Fold::Open => {
        for _ in 0..count {
          let Some(source) = self.step(panel.selected(), operation) else {
            break;
          };
          panel.choose(source);
        }
      }
    }
    self.project(panel, auto);
    panel.reveal();
  }

  fn step(&mut self, source: usize, operation: Fold) -> Option<usize> {
    let node = self.tree.nodes().get(source)?;
    match operation {
      Fold::Close if node.descendants > 0 && !self.closed[source] => {
        self.closed[source] = true;
        Some(source)
      }
      Fold::Close => node.parent,
      Fold::Open if node.descendants == 0 => None,
      Fold::Open if self.closed[source] => {
        self.closed[source] = false;
        Some(source)
      }
      Fold::Open => Some(source + 1),
      Fold::ExpandAll | Fold::CollapseAll => unreachable!(),
    }
  }

  pub fn project(&self, panel: &mut Panel, auto: Option<bool>) {
    self.project_closed(panel, &self.closed, auto);
  }

  /// Projects with the selection's ancestors open, keeping the folds.
  pub fn preview(&self, panel: &mut Panel, auto: Option<bool>) {
    let mut closed = self.closed.clone();
    reveal(&self.tree, panel.selected(), &mut closed);
    self.project_closed(panel, &closed, auto);
    panel.reveal();
  }

  fn project_closed(
    &self,
    panel: &mut Panel,
    closed: &[bool],
    auto: Option<bool>,
  ) {
    let previous = panel.selected();
    let projection = self.tree.project(|source| closed[source]);
    let selected = self.tree.visible_ancestor(panel.selected(), &projection);
    if let Some(selected) = selected {
      panel.choose(selected);
    }
    panel.project(projection);
    panel.decorate_tree(&self.tree, |source| closed[source], auto);
    if panel.selected() != previous {
      panel.reveal();
    }
  }
}

fn reveal(tree: &Tree, source: usize, closed: &mut [bool]) {
  let mut parent = tree.nodes().get(source).and_then(|node| node.parent);
  while let Some(source) = parent {
    closed[source] = false;
    parent = tree.nodes()[source].parent;
  }
}

impl Panel {
  /// Groups rows by their depth; every group starts open.
  pub fn group(&mut self) {
    let folds =
      Folds::new(Tree::from_depths(self.rows.iter().map(|row| row.depth)));
    folds.project(self, None);
    self.folds = Some(folds);
  }

  pub fn foldable(&self) -> bool {
    self.kind == PanelKind::Outline || self.folds.is_some()
  }

  /// Folds the panel's own groups; false when it has none.
  pub fn fold(&mut self, operation: Fold, count: usize) -> bool {
    let Some(mut folds) = self.folds.take() else { return false };
    folds.fold(self, (operation, count), None);
    self.folds = Some(folds);
    true
  }
}
