use super::View;
use crate::keys::PanelKind;
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
}

impl Outline {
  pub fn new(headings: &[Heading]) -> Self {
    Self {
      current: None,
      tree: Tree::new(headings),
      headings: headings.to_vec(),
    }
  }

  pub fn tree(&self) -> &Tree {
    &self.tree
  }

  pub fn replace(&mut self, headings: &[Heading]) {
    *self = Self::new(headings);
  }

  pub fn sync(&mut self, headings: &[Heading]) {
    if self.headings.len() != headings.len()
      || self.headings.iter().zip(headings).any(|(old, new)| {
        old.depth != new.depth
          || old.title != new.title
          || old.target != new.target
      })
    {
      self.replace(headings);
    }
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
    if let Some(panel) =
      panel.filter(|panel| panel.kind() == PanelKind::Outline)
    {
      panel.track(section, changed);
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
