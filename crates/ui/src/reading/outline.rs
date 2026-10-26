use super::View;
use crate::keys::{Fold, PanelKind};
use crate::panel::Panel;
use crate::panel::tree::Tree;
use crate::source::{self, Pages};
use crate::text::{PageId, Position};
use vpdf_model::document::{Heading, Target, section};
use vpdf_reader::reader::TextSource;

#[derive(Default)]
pub struct Outline {
  current: Option<usize>,
  tree: Tree,
  headings: Vec<Heading>,
  closed: Vec<bool>,
  auto: bool,
  saved: Option<Preferences>,
  fingerprint: [u8; 32],
}

mod preferences;
pub use preferences::Preferences;
#[cfg(test)]
mod preferences_tests;
mod search;

impl Outline {
  pub fn new(headings: &[Heading]) -> Self {
    Self {
      current: None,
      tree: Tree::new(headings),
      headings: headings.to_vec(),
      closed: vec![false; headings.len()],
      auto: false,
      saved: None,
      fingerprint: preferences::fingerprint(headings),
    }
  }

  pub fn tree(&self) -> &Tree {
    &self.tree
  }

  pub fn closed(&self, source: usize) -> bool {
    self.closed.get(source).copied().unwrap_or(false)
  }

  fn visible_source(&self, source: usize) -> Option<usize> {
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

  pub fn project(&self, panel: &mut Panel) {
    self.project_folds(panel, &self.closed);
  }

  fn project_folds(&self, panel: &mut Panel, closed: &[bool]) {
    let previous = panel.selected();
    let projection = self.tree.project(|source| closed[source]);
    let selected = self.tree.visible_ancestor(panel.selected(), &projection);
    if let Some(selected) = selected {
      panel.choose(selected);
    }
    let current = self
      .current
      .and_then(|source| self.tree.visible_ancestor(source, &projection));
    panel.project(projection);
    panel.decorate_outline(
      &self.tree,
      |source| closed[source],
      current,
      self.auto,
    );
    if panel.selected() != previous {
      panel.reveal();
    }
  }

  pub fn fold(&mut self, panel: &mut Panel, operation: Fold, count: usize) {
    if panel.kind() != PanelKind::Outline
      || self.headings.is_empty() && !self.auto
    {
      return;
    }
    self.auto = false;
    match operation {
      Fold::ExpandAll => self.closed.fill(false),
      Fold::CollapseAll => {
        for (closed, node) in self.closed.iter_mut().zip(self.tree.nodes()) {
          *closed = node.descendants > 0;
        }
      }
      Fold::Close | Fold::Open => {
        for _ in 0..count {
          let Some(source) = self.step(panel.selected(), operation) else {
            break;
          };
          panel.choose(source);
        }
      }
    }
    self.commit_preferences();
    self.project(panel);
    panel.reveal();
  }

  pub fn toggle_auto(&mut self, panel: &mut Panel) {
    self.auto = !self.auto;
    if self.auto {
      self.expand_current();
    }
    self.commit_preferences();
    self.project(panel);
  }

  fn expand_current(&mut self) {
    for (closed, node) in self.closed.iter_mut().zip(self.tree.nodes()) {
      *closed = node.descendants > 0;
    }
    let mut source = self.current;
    while let Some(index) = source {
      let Some(node) = self.tree.nodes().get(index) else { break };
      self.closed[index] = false;
      source = node.parent;
    }
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

  pub fn replace(&mut self, headings: &[Heading]) {
    let saved = self.saved.take();
    *self = Self::new(headings);
    self.restore_preferences(saved);
  }

  pub fn sync(&mut self, headings: &[Heading]) -> bool {
    if self.headings.len() != headings.len()
      || self.headings.iter().zip(headings).any(|(old, new)| {
        old.depth != new.depth
          || old.title != new.title
          || old.target != new.target
      })
    {
      self.replace(headings);
      return true;
    }
    false
  }

  pub fn current(&self) -> Option<usize> {
    self.current
  }

  /// Follows section changes without interrupting panel browsing.
  pub fn track<S: TextSource + Pages + source::Outline>(
    &mut self,
    view: &mut View<'_, S>,
    panel: Option<&mut Panel>,
  ) {
    let section = view.current_section();
    let changed = self.current != section;
    self.current = section;
    if changed && self.auto {
      self.expand_current();
    }
    if let Some(panel) =
      panel.filter(|panel| panel.kind() == PanelKind::Outline)
    {
      if changed && self.auto {
        if panel.searching() {
          let query = panel.query().to_owned();
          self.preview_search(panel, &query);
        } else {
          self.project(panel);
        }
      }
      panel.mark_outline(&self.tree, section);
      panel.track(
        section.map(|source| self.visible_source(source).unwrap_or(source)),
        changed,
      );
    }
  }
}

impl<S: TextSource + Pages + source::Outline> View<'_, S> {
  /// Outline entry containing the cursor.
  pub fn current_section(&mut self) -> Option<usize> {
    if let Some(index) = self.landed_section() {
      return Some(index);
    }
    let cursor = self.navigation();
    if self.reader.viewport_mode() && !self.frame.pane.text {
      let y = self
        .layout
        .unproject(cursor.page, (0.0, 0.0), self.reader.scroll)
        .map_or(0.0, |(_, y)| y);
      return section(self.source.outline(), cursor.page, y);
    }
    self.glyph_section(cursor)
  }

  /// A page's label and the section the reader names at its top.
  pub fn page_line(&mut self, page: PageId) -> String {
    let section = self.top_section(page);
    let label = self.source.label(page);
    match section.map(|index| self.source.outline()[index].title.trim()) {
      Some(title) => format!("p. {label} · {title}"),
      None => format!("p. {label}"),
    }
  }

  fn top_section(&mut self, page: PageId) -> Option<usize> {
    if self.reader.viewport_mode() && !self.frame.pane.text {
      return section(self.source.outline(), page, 0.0);
    }
    self.glyph_section(Position { page, ..Position::default() })
  }

  fn glyph_section(&mut self, position: Position) -> Option<usize> {
    let y = self
      .source
      .page_text(position.page)
      .ok()
      .and_then(|text| text.glyph(position).map(|g| g.bounds.y1))
      .unwrap_or(f32::MAX);
    section(self.source.outline(), position.page, y)
  }

  fn landed_section(&mut self) -> Option<usize> {
    let cursor = self.reader.cursor;
    if !self.matches_landing(cursor) {
      return None;
    }
    // Followed entries name themselves, even sharing a line.
    if let Some(target) = self.landing.aimed()
      && let Some(index) = self
        .source
        .outline()
        .iter()
        .rposition(|heading| heading.target == *target)
    {
      return Some(index);
    }
    let text = self.source.page_text(cursor.page).ok()?;
    // Headings land on the glyph nearest their point, as `follow` does.
    self.source.outline().iter().rposition(|heading| match heading.target {
      Target::Point { page, x, y } if page == cursor.page => {
        text.nearest(page, x, y).line == cursor.line
      }
      _ => false,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn outline_generations() {
    let mut headings = vec![
      Heading {
        title: "Duplicate".into(),
        depth: 0,
        target: Target::Page(PageId(0)),
      };
      3
    ];
    headings[1].depth = 1;
    let mut outline = Outline::new(&headings);
    outline.current = Some(1);
    outline.sync(&headings);
    assert_eq!(outline.current(), Some(1));
    assert_eq!(outline.tree().nodes()[1].parent, Some(0));
    headings[1].depth = 0;
    outline.sync(&headings);
    assert_eq!(outline.current(), None);
    assert_eq!(outline.tree().nodes()[1].parent, None);
    for change in [true, false] {
      outline.current = Some(2);
      if change {
        headings[2].title = "Replacement".into();
      } else {
        headings[2].target = Target::Page(PageId(1));
      }
      outline.sync(&headings);
      assert_eq!(outline.current(), None);
    }
    outline.current = Some(2);
    outline.replace(&headings);
    assert_eq!(outline.current(), None);
    outline.replace(&[]);
    assert!(outline.tree().nodes().is_empty());
  }
}

#[cfg(test)]
mod fold_tests {
  use super::*;
  use crate::panel::Row;

  pub(super) fn setup() -> (Outline, Panel) {
    let headings: Vec<_> = [0, 3, 8, 3, 0, 2]
      .into_iter()
      .map(|depth| Heading {
        depth,
        title: "Duplicate".into(),
        target: Target::Page(PageId(0)),
      })
      .collect();
    let panel = Panel::new(
      PanelKind::Outline,
      headings
        .iter()
        .map(|heading| Row::outline(heading, "iv".into()))
        .collect(),
    );
    (Outline::new(&headings), panel)
  }

  #[test]
  fn nested_choices_survive() {
    let (mut outline, mut panel) = setup();
    panel.choose(1);
    outline.fold(&mut panel, Fold::Close, 1);
    assert!(outline.closed(1));
    outline.fold(&mut panel, Fold::Close, 2);
    assert_eq!(panel.selected(), 0);
    assert!(outline.closed(0));
    outline.fold(&mut panel, Fold::Open, 2);
    assert_eq!(panel.selected(), 1);
    assert!(outline.closed(1));
    panel.navigate(crate::keys::Input::plain(crate::keys::Action::Rows(1)), 20);
    assert_eq!(panel.selected(), 3);
    outline.fold(&mut panel, Fold::Open, usize::MAX);
    assert_eq!(panel.selected(), 3, "leaves stay selected");
    assert_eq!(outline.visible_source(2), Some(1));
    panel.choose(0);
    outline.fold(&mut panel, Fold::Close, 1);
    assert_eq!(outline.visible_source(2), Some(0));
    let mut reopened = setup().1;
    reopened.choose(2);
    outline.project(&mut reopened);
    assert_eq!(reopened.selected(), 0);
    outline.sync(&outline.headings.clone());
    assert!(outline.closed(0));
    outline.replace(&outline.headings.clone());
    assert!(outline.closed(0));
  }

  #[test]
  fn marker_and_auto_path() {
    let (mut outline, mut panel) = setup();
    outline.current = Some(2);
    panel.choose(5);
    outline.fold(&mut panel, Fold::CollapseAll, 1);
    assert_eq!(panel.selected(), 4);
    assert_eq!(outline.current(), Some(2));
    let mut scene = Vec::new();
    panel.scene_list(&mut scene, vpdf_model::geometry::At::default(), 30, 8);
    let text = crate::testing::text(&scene);
    assert!(text.contains("▌▸"));
    outline.toggle_auto(&mut panel);
    assert!(!outline.closed(0) && !outline.closed(1));
    assert!(outline.closed(4));
    assert_eq!(panel.selected(), 4);
    assert!(panel.auto_expand());
    outline.toggle_auto(&mut panel);
    assert!(outline.closed(4));
    assert!(!panel.auto_expand());
    outline.current = None;
    outline.toggle_auto(&mut panel);
    assert!(outline.closed(0) && outline.closed(1) && outline.closed(4));
  }

  #[test]
  fn auto_reveals_hidden_selection() {
    let headings: Vec<_> = (0..21)
      .map(|source| Heading {
        depth: usize::from((1..11).contains(&source)),
        title: format!("Entry {source}"),
        target: Target::Page(PageId(0)),
      })
      .collect();
    let mut outline = Outline::new(&headings);
    let mut panel = Panel::new(
      PanelKind::Outline,
      headings
        .iter()
        .map(|heading| Row::outline(heading, "i".into()))
        .collect(),
    );
    outline.project(&mut panel);
    panel.scene_list(
      &mut Vec::new(),
      vpdf_model::geometry::At::default(),
      30,
      6,
    );
    panel.select(9);
    assert!(panel.top() > 0);
    outline.toggle_auto(&mut panel);
    panel.scene_list(
      &mut Vec::new(),
      vpdf_model::geometry::At::default(),
      30,
      6,
    );
    assert_eq!(panel.selected(), 0);
    assert_eq!(panel.row_at(0, 1), Some(0));
  }

  #[test]
  fn global_folds_and_counts() {
    let (mut outline, mut panel) = setup();
    panel.choose(2);
    outline.fold(&mut panel, Fold::CollapseAll, 1);
    assert_eq!(panel.selected(), 0);
    panel.navigate(crate::keys::Input::plain(crate::keys::Action::Rows(1)), 20);
    assert_eq!(panel.selected(), 4);
    outline.fold(&mut panel, Fold::ExpandAll, 1);
    panel.choose(0);
    outline.fold(&mut panel, Fold::Open, usize::MAX);
    assert_eq!(panel.selected(), 2);
    outline.fold(&mut panel, Fold::Close, usize::MAX);
    assert_eq!(panel.selected(), 0);
    assert!(outline.closed(0) && outline.closed(1));
    panel.focus(false);
    panel.track(Some(4), true);
    assert_eq!(panel.selected(), 4);
    let (mut empty, mut view) =
      (Outline::default(), Panel::new(PanelKind::Outline, vec![]));
    for operation in
      [Fold::Close, Fold::Open, Fold::ExpandAll, Fold::CollapseAll]
    {
      empty.fold(&mut view, operation, 1);
      assert_eq!(view.selected(), 0);
    }
  }

  #[test]
  fn hidden_search_paths() {
    let (mut outline, mut panel) = setup();
    outline.fold(&mut panel, Fold::CollapseAll, 1);
    panel.begin_search(false);
    outline.preview_search(&mut panel, "iv");
    assert_eq!((panel.selected(), panel.count()), (1, Some((2, 6))));
    assert!(outline.closed(0), "preview leaves committed folds alone");
    assert_eq!(
      panel.control_action(crate::panel::Control::Branch(0)),
      Some(crate::keys::Action::Fold(Fold::Close))
    );
    outline.preview_search(&mut panel, "missing");
    assert_eq!(panel.selected(), 0);
    assert_eq!(
      panel.control_action(crate::panel::Control::Branch(0)),
      Some(crate::keys::Action::Fold(Fold::Open))
    );
    outline.abort_search(&mut panel);
    assert_eq!(panel.selected(), 0);
    assert!(outline.closed(0) && outline.closed(1));

    panel.begin_search(true);
    outline.preview_search(&mut panel, "iv");
    assert_eq!(panel.selected(), 5);
    assert!(outline.closed(4));
    assert!(outline.finish_search(&mut panel, "iv"));
    assert_eq!(panel.selected(), 5);
    assert!(!outline.closed(4));
    assert!(outline.closed(0) && outline.closed(1));
    outline.repeat_search(&mut panel, true, 3);
    assert_eq!(panel.selected(), 2);
    assert!(!outline.closed(0) && !outline.closed(1));
  }

  #[test]
  fn search_auto_transitions() {
    let (mut outline, mut panel) = setup();
    outline.current = Some(5);
    outline.toggle_auto(&mut panel);
    panel.begin_search(false);
    outline.preview_search(&mut panel, "iv");
    assert!(panel.auto_expand());
    outline.abort_search(&mut panel);
    assert!(panel.auto_expand() && outline.closed(0));
    panel.begin_search(false);
    assert!(!outline.finish_search(&mut panel, "missing"));
    assert!(panel.auto_expand() && outline.closed(0));
    panel.begin_search(false);
    assert!(outline.finish_search(&mut panel, "iv"));
    assert!(!panel.auto_expand());
    outline.toggle_auto(&mut panel);
    outline.repeat_search(&mut panel, false, 1);
    assert_eq!(panel.selected(), 1);
    assert!(!panel.auto_expand());
  }

  #[test]
  fn generation_preserves_mode() {
    let (mut outline, mut panel) = setup();
    outline.current = Some(2);
    outline.toggle_auto(&mut panel);
    let mut headings = outline.headings.clone();
    assert!(!outline.sync(&headings));
    assert_eq!(outline.current(), Some(2));
    headings[1].title = "Replacement".into();
    assert!(outline.sync(&headings));
    outline.project(&mut panel);
    assert!(panel.auto_expand());
    assert!(outline.closed(0) && outline.closed(4));
    outline.current = Some(5);
    outline.expand_current();
    let mut reopened = setup().1;
    outline.project(&mut reopened);
    assert!(reopened.auto_expand());
    assert!(outline.closed(0) && !outline.closed(4));
    outline.fold(&mut reopened, Fold::Open, 1);
    assert!(!reopened.auto_expand());
  }
}
