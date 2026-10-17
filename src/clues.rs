//! Next-key hints after a prefix key, like mini.clue.

use crate::app::App;
use crate::keys;
use std::time::{Duration, Instant};

/// Idle time after a prefix before clues show. mini.clue defaults to a
/// second; a shorter wait suits a reader with few prefixes.
pub const DELAY: Duration = Duration::from_millis(500);

/// Shown clues: next key and what it does.
pub struct Clues {
  pub items: Vec<(String, String)>,
}

impl App {
  /// Shows clues once a prefix has waited `DELAY`; true when shown.
  /// They cover the bottom of the page area, which keeps its size.
  pub fn clue_due(&mut self) -> bool {
    let due = self.clues.is_none()
      && self.prompt.is_none()
      && self.awaiting.is_some_and(|since: Instant| since.elapsed() >= DELAY);
    if !due {
      return false;
    }
    // The wait is over even when the prefix has no clues.
    self.awaiting = None;
    let Some(prefix) = self.keys.awaiting().map(str::to_owned) else {
      return false;
    };
    let items = self.clue_items(&prefix);
    if items.is_empty() {
      return false;
    }
    self.clues = Some(Clues { items });
    true
  }

  /// Time left before clues show, while a prefix waits for them.
  pub fn clue_wait(&self) -> Option<Duration> {
    if self.clues.is_some() || self.prompt.is_some() {
      return None;
    }
    self.awaiting.map(|since| DELAY.saturating_sub(since.elapsed()))
  }

  /// Hides clues and restarts the wait when a prefix remains.
  pub fn after_key(&mut self) {
    self.awaiting = self.keys.awaiting().map(|_| Instant::now());
    self.clues = None;
  }

  fn clue_items(&self, prefix: &str) -> Vec<(String, String)> {
    let owned = |(key, about): (&str, &str)| (key.to_owned(), about.to_owned());
    match prefix {
      "\"" => self.register_clues(),
      "m" => [
        ("a-z", "mark here in this document"),
        ("A-Z", "mark here across documents"),
      ]
      .map(owned)
      .to_vec(),
      "'" | "`" => {
        let mut items: Vec<_> = self
          .marks
          .iter()
          .map(|(name, at)| {
            (name.to_string(), format!("page {}", at.page.0 + 1))
          })
          .collect();
        items.push(("A-Z".into(), "mark in any document".into()));
        items.push((prefix.into(), "back to the last jump".into()));
        items
      }
      _ => keys::clues(prefix)
        .into_iter()
        .map(|(key, about)| (key.replace(' ', "Space"), about.to_owned()))
        .collect(),
    }
  }

  /// Registers holding text, with a one-line preview of each.
  fn register_clues(&self) -> Vec<(String, String)> {
    let mut items: Vec<_> = self
      .reader
      .registers
      .entries()
      .map(|(name, text)| {
        let preview: String = text
          .split_whitespace()
          .flat_map(|word| std::iter::once(' ').chain(word.chars()))
          .skip(1)
          .take(40)
          .collect();
        (name.to_string(), preview)
      })
      .collect();
    items.sort();
    items.push(("+ *".into(), "system clipboard".into()));
    items.push(("a-z".into(), "named register; A-Z appends".into()));
    items
  }

  /// Rows the clues take above the status line.
  pub fn clue_rows(&self) -> u16 {
    self.clues.as_ref().map_or(0, |clues| {
      let (_, grid, _) = self.clue_grid(clues);
      u16::try_from(grid.visible).unwrap_or(0)
    })
  }

  /// Layout and key column width.
  /// Labels, their placement, and the key width of each label.
  pub fn clue_grid(
    &self,
    clues: &Clues,
  ) -> (Vec<String>, crate::grid::Grid, usize) {
    let keys = clues.items.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let labels: Vec<_> = clues
      .items
      .iter()
      .map(|(key, about)| format!("{key:<keys$}  {about}"))
      .collect();
    let columns = usize::from(self.screen.columns);
    let limit = crate::grid::limit(self.screen);
    let grid = crate::grid::layout(&labels, columns, limit, None);
    (labels, grid, keys)
  }
}
