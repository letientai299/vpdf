use crate::document::Target;
use crate::panel::{Destination, Panel, Row, Style};

/// Row search inside a panel, like `/` in a Vim buffer.
#[derive(Default)]
pub struct RowSearch {
  pub query: String,
  pub backward: bool,
  /// Selection and query when the prompt opened; `Some` while typing.
  origin: Option<(usize, String, bool)>,
}

/// Case-insensitive unless the query has uppercase (Vim's smartcase).
fn matcher(query: &str) -> impl Fn(&str) -> bool {
  let pattern = crate::search::compile(query, false).ok();
  move |label| {
    pattern.as_ref().is_some_and(|pattern| !pattern.ranges(label).is_empty())
  }
}

fn matches(row: &Row, hit: &impl Fn(&str) -> bool) -> bool {
  hit(&row.label)
    || hit(&row.detail)
    || matches!(&row.destination, Destination::Link(Target::Uri(uri))
      if hit(uri))
}

/// Byte ranges of `query` in `text`, by the same rule as `hit`.
pub fn ranges(text: &str, query: &str) -> Vec<(usize, usize)> {
  if query.is_empty() {
    return Vec::new();
  }
  crate::search::compile(query, false)
    .map_or_else(|_| Vec::new(), |pattern| pattern.ranges(text))
}

impl RowSearch {
  /// Moves the saved selection with refilled rows.
  pub fn follow(&mut self, moved: impl Fn(usize) -> usize) {
    if let Some((origin, _, _)) = &mut self.origin {
      *origin = moved(*origin);
    }
  }
}

impl Panel {
  pub fn searching(&self) -> bool {
    self.search.origin.is_some()
  }

  pub fn begin_search(&mut self, backward: bool) {
    self.search.origin =
      Some((self.selected, self.search.query.clone(), self.search.backward));
    self.search.backward = backward;
  }

  /// Selects the first match after the origin while typing.
  pub fn preview(&mut self, query: &str) {
    let Some((origin, _, _)) = self.search.origin else { return };
    self.selected = origin;
    query.clone_into(&mut self.search.query);
    if !query.is_empty() {
      self.find(query, origin, self.search.backward, 1);
    }
  }

  /// Keeps the match; an empty query repeats the last one.
  pub fn finish_search(&mut self, query: &str) -> bool {
    let (origin, previous, _) = self.search.origin.take().unwrap_or((
      self.selected,
      String::new(),
      false,
    ));
    self.search.query =
      if query.is_empty() { previous } else { query.to_owned() };
    let query = self.search.query.clone();
    self.selected = origin;
    !query.is_empty() && self.find(&query, origin, self.search.backward, 1)
  }

  pub fn abort_search(&mut self) {
    if let Some((origin, previous, backward)) = self.search.origin.take() {
      self.selected = origin;
      self.search.query = previous;
      self.search.backward = backward;
      self.reveal();
    }
  }

  /// `n` and `N`: the `count`th match, wrapping around.
  pub fn repeat(&mut self, reverse: bool, count: usize) -> bool {
    let query = self.search.query.clone();
    let backward = self.search.backward != reverse;
    !query.is_empty() && self.find(&query, self.selected, backward, count)
  }

  fn find(
    &mut self,
    query: &str,
    from: usize,
    backward: bool,
    count: usize,
  ) -> bool {
    let hit = matcher(query);
    let hits: Vec<_> = self
      .rows
      .iter()
      .enumerate()
      .filter(|(_, row)| matches(row, &hit))
      .map(|(index, _)| index)
      .collect();
    let Some(index) =
      crate::search::next(&hits, &from, backward, count, |&row| row)
    else {
      return false;
    };
    self.select(hits[index]);
    true
  }

  /// Match under the selection and the total, for the status line.
  pub fn count(&self) -> Option<(usize, usize)> {
    let query = &self.search.query;
    if query.is_empty() {
      return None;
    }
    let hit = matcher(query);
    let hits =
      self.rows.iter().enumerate().filter(|(_, row)| matches(row, &hit));
    let (mut index, mut total) = (0, 0);
    for (i, _) in hits {
      total += 1;
      index += usize::from(i <= self.selected);
    }
    (total > 0).then_some((index, total))
  }

  /// `{` and `}`: previous or next heading row.
  pub fn heading(&mut self, forward: bool, count: usize) {
    let headings = |i: &usize| self.rows[*i].style == Style::Heading;
    for _ in 0..count {
      let next = if forward {
        (self.selected + 1..self.rows.len()).find(headings)
      } else {
        (0..self.selected).rev().find(headings)
      };
      let Some(index) = next else { return };
      self.selected = index;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::keys::PanelKind;
  use crate::panel::Row;

  fn panel(labels: &[&str]) -> Panel {
    let rows = labels.iter().map(|label| Row::parse(label)).collect();
    Panel::new(PanelKind::Help, rows)
  }

  #[test]
  fn large_counts_wrap_matches() {
    let mut panel = panel(&["match", "skip", "match", "skip", "match"]);
    panel.begin_search(false);
    assert!(panel.finish_search("match"));
    assert_eq!(panel.selected, 2);
    assert!(panel.repeat(false, usize::MAX));
    assert_eq!(panel.selected, 2);
    assert!(panel.repeat(true, usize::MAX - 1));
    assert_eq!(panel.selected, 4);
    panel.begin_search(false);
    assert!(!panel.finish_search("missing"));
    assert!(!panel.repeat(false, usize::MAX));
  }

  #[test]
  fn cancelled_search_restores_direction() {
    let mut panel = panel(&["match one", "match two", "match three"]);
    for backward in [false, true] {
      panel.begin_search(backward);
      assert!(panel.finish_search("match"));
      let before = panel.selected;
      panel.begin_search(!backward);
      panel.preview("missing");
      panel.abort_search();
      assert_eq!(panel.selected, before);
      assert_eq!(panel.search.backward, backward);
      assert!(panel.repeat(false, 1));
      assert_eq!(panel.selected, (before + if backward { 2 } else { 1 }) % 3);
    }
  }

  #[test]
  fn marks_matches_by_smartcase() {
    assert_eq!(ranges("Next match, MATCH", "match"), [(5, 10), (12, 17)]);
    assert_eq!(ranges("Next match, MATCH", "MATCH"), [(12, 17)]);
    assert_eq!(ranges("Ärger", "är"), [(0, 3)]);
    assert_eq!(ranges("Ⱥ ÄR", "är"), [(3, 6)]);
    assert_eq!(ranges("İ next", "next"), [(3, 7)]);
    assert_eq!(ranges("\u{212a}elvin", "kelvin"), [(0, 8)]);
  }

  #[test]
  fn searches_labels_and_destinations() {
    let mut panel = Panel::new(
      PanelKind::Links,
      vec![
        Row::new(
          "Project page".into(),
          String::new(),
          Destination::Link(Target::Uri("https://example.com/needle".into())),
        ),
        Row::new("Reference text".into(), "p. iv".into(), Destination::None),
      ],
    );
    for (query, selected) in [("project", 0), ("needle", 0), ("p. iv", 1)] {
      panel.begin_search(false);
      panel.preview(query);
      assert_eq!(panel.selected, selected);
      assert_eq!(panel.count(), Some((1, 1)));
      panel.abort_search();
    }
  }

  #[test]
  fn searches_rows_with_smartcase_and_wraps() {
    let mut panel =
      panel(&["# Move", "j: down", "# Find", "n: next Match", "N: match"]);
    panel.begin_search(false);
    panel.preview("ma");
    assert_eq!(panel.selected, 3);
    panel.preview("Ma");
    assert_eq!(panel.selected, 3);
    assert!(panel.finish_search("match"));
    assert_eq!((panel.selected, panel.count()), (3, Some((1, 2))));
    assert!(panel.repeat(false, 2));
    assert_eq!(panel.selected, 3);
    assert!(panel.repeat(true, 1));
    assert_eq!(panel.selected, 4);
    panel.begin_search(true);
    panel.preview("zzz");
    panel.abort_search();
    assert_eq!(panel.selected, 4);
    panel.heading(false, 1);
    assert_eq!(panel.selected, 2);
    panel.heading(false, 5);
    assert_eq!(panel.selected, 0);
  }

  #[test]
  fn fewer_rows_keep_the_search_origin_in_range() {
    let fewer = panel(&["# A", "b"]).rows;
    let mut panel = panel(&["# A", "b", "# C", "d", "e", "f"]);
    panel.selected = 5;
    panel.begin_search(false);
    panel.preview("row");
    panel.refill(fewer, "");
    panel.abort_search();
    assert_eq!(panel.selected, 1);
    panel.heading(false, 1);
    assert_eq!(panel.selected, 0);
  }

  use crate::Result;
  use crate::search::{Index, compile};
  use crate::text::{Bounds, Glyph, Line, PageId, Text};

  #[test]
  fn panels_share_search_contract() -> Result<()> {
    let label = "Ärger alpha Alpha beta Straße";
    let index = Index::new(
      PageId(0),
      &Text {
        lines: vec![Line {
          paragraph: true,
          glyphs: label
            .chars()
            .map(|ch| Glyph { ch, bounds: Bounds::default() })
            .collect(),
        }],
        ..Text::default()
      },
    );
    for (query, count) in [
      ("alpha", 2),
      ("Alpha", 1),
      ("ALPHA", 0),
      (r"\valpha|beta", 3),
      (r"\vAlpha|beta", 2),
      ("alpha|beta", 0),
      ("är", 1),
      ("strasse", 1),
      ("STRASSE", 0),
    ] {
      let pattern = compile(query, false)?;
      assert_eq!(index.matches(&pattern, 100)?.len(), count, "{query}");
      assert_eq!(ranges(label, query).len(), count, "{query}");
    }
    assert_eq!(ranges(label, r"\v["), [] as [(usize, usize); 0]);
    Ok(())
  }
}
