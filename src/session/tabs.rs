use super::{App, Direction, Node, Rect, Session, Tab, TabMove, neighbor};

impl Tab {
  pub(super) fn new(app: App) -> Self {
    Self { tree: Node::Leaf(app.id), panes: vec![app], focus: 0, last: None }
  }

  pub(super) fn index(&self, id: u32) -> Option<usize> {
    self.panes.iter().position(|pane| pane.id == id)
  }

  /// Pane ids in screen order.
  pub fn order(&self) -> Vec<u32> {
    self.tree.leaves()
  }

  pub(super) fn detach(&mut self, area: Rect) -> App {
    let id = self.panes[self.focus].id;
    let mut rects = Vec::new();
    self.tree.layout(area, &mut rects);
    let peer =
      self.last.filter(|peer| *peer != id && self.index(*peer).is_some());
    let next = peer.or_else(|| {
      [Direction::Right, Direction::Down, Direction::Left, Direction::Up]
        .into_iter()
        .find_map(|direction| neighbor(&rects, id, direction))
    });
    let app = self.panes.remove(self.focus);
    self.tree.remove(id);
    self.focus = next.and_then(|id| self.index(id)).unwrap_or(0);
    self.last = None;
    app
  }
}

impl Session {
  pub fn tab(&mut self) -> &mut Tab {
    &mut self.tabs[self.active]
  }

  pub fn app(&mut self) -> &mut App {
    let tab = self.tab();
    &mut tab.panes[tab.focus]
  }

  /// Readies the newly focused pane to request its view again.
  pub(super) fn refocus(&mut self) {
    self.app().requested.clear();
  }

  pub(super) fn step(&self, step: TabMove) -> usize {
    let count = self.tabs.len();
    match step {
      TabMove::Next(n) => (self.active + n) % count,
      TabMove::Previous(n) => (self.active + count - n % count) % count,
      TabMove::Go(n) => n.clamp(1, count) - 1,
    }
  }
}
