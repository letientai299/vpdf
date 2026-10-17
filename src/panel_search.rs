use crate::document::Target;
use crate::panel::{Destination, Panel, Row, Style};

/// Row search inside a panel, like `/` in a Vim buffer.
#[derive(Default)]
pub struct RowSearch {
  pub query: String,
  pub backward: bool,
  /// Selection and query when the prompt opened; `Some` while typing.
  origin: Option<(usize, String)>,
}

/// Case-insensitive unless the query has uppercase (Vim's smartcase).
fn matcher(query: &str) -> impl Fn(&str) -> bool {
  let exact = query.chars().any(char::is_uppercase);
  let query = if exact { query.to_owned() } else { query.to_lowercase() };
  move |label| {
    if exact {
      label.contains(&query)
    } else {
      label.to_lowercase().contains(&query)
    }
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
  let folded;
  let (haystack, needle) = if query.chars().any(char::is_uppercase) {
    (text, query.to_owned())
  } else if query.is_ascii() {
    folded = text.to_ascii_lowercase();
    (folded.as_str(), query.to_ascii_lowercase())
  } else {
    // Unicode folding can shift byte offsets; skip marking.
    return Vec::new();
  };
  haystack
    .match_indices(&needle)
    .map(|(start, found)| (start, start + found.len()))
    .collect()
}

impl RowSearch {
  /// Keeps the saved selection within `rows` rows.
  pub fn clamp(&mut self, rows: usize) {
    if let Some((origin, _)) = &mut self.origin {
      *origin = (*origin).min(rows.saturating_sub(1));
    }
  }
}

impl Panel {
  pub fn searching(&self) -> bool {
    self.search.origin.is_some()
  }

  pub fn begin_search(&mut self, backward: bool) {
    self.search.origin = Some((self.selected, self.search.query.clone()));
    self.search.backward = backward;
  }

  /// Selects the first match after the origin while typing.
  pub fn preview(&mut self, query: &str) {
    let Some((origin, _)) = self.search.origin else { return };
    self.selected = origin;
    query.clone_into(&mut self.search.query);
    if !query.is_empty() {
      self.find(query, origin, self.search.backward, 1);
    }
  }

  /// Keeps the match; an empty query repeats the last one.
  pub fn finish_search(&mut self, query: &str) -> bool {
    let (origin, previous) =
      self.search.origin.take().unwrap_or((self.selected, String::new()));
    self.search.query =
      if query.is_empty() { previous } else { query.to_owned() };
    let query = self.search.query.clone();
    self.selected = origin;
    !query.is_empty() && self.find(&query, origin, self.search.backward, 1)
  }

  pub fn abort_search(&mut self) {
    if let Some((origin, previous)) = self.search.origin.take() {
      self.selected = origin;
      self.search.query = previous;
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
    let total = self.rows.len();
    let mut current = from;
    for _ in 0..count {
      let next = (1..=total).map(|step| {
        if backward {
          (current + total - step % total) % total
        } else {
          (current + step) % total
        }
      });
      let Some(index) =
        next.into_iter().find(|&i| matches(&self.rows[i], &hit))
      else {
        return false;
      };
      current = index;
    }
    self.selected = current;
    self.reveal();
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
  fn marks_matches_by_smartcase() {
    assert_eq!(ranges("Next match, MATCH", "match"), [(5, 10), (12, 17)]);
    assert_eq!(ranges("Next match, MATCH", "MATCH"), [(12, 17)]);
    assert_eq!(ranges("Ärger", "är"), []);
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
}
