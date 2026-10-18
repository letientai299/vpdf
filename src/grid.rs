//! Items in columns filled top to bottom, like zsh's completion list,
//! drawn in rows above the status line.

use crate::Result;
use crate::layout::Screen;
use crate::terminal::{self, At};
use crate::theme::{self, Color};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

/// Placement of labels in columns; the labels stay with the caller.
pub struct Grid {
  /// Cell width per item, including padding.
  pub width: usize,
  pub columns: usize,
  pub rows: usize,
  /// First shown row and how many are shown.
  pub first: usize,
  pub visible: usize,
}

/// At most `limit` rows show, scrolled so `selected` stays visible.
pub fn layout(
  labels: &[String],
  columns: usize,
  limit: usize,
  selected: Option<usize>,
) -> Grid {
  let widest = labels.iter().map(|label| label.width()).max().unwrap_or(0);
  let width = (widest + 2).min(columns.max(3));
  let per_row = (columns / width).max(1);
  let rows = labels.len().div_ceil(per_row).max(1);
  let visible = rows.min(limit.max(1));
  let selected = selected.map_or(0, |index| index % rows);
  let first = selected.saturating_sub(visible - 1);
  Grid { width, columns: per_row, rows, first, visible }
}

impl Grid {
  /// The label at a column and line.
  pub fn item_at(
    &self,
    column: usize,
    line: usize,
    count: usize,
  ) -> Option<usize> {
    if line >= self.visible || self.width == 0 {
      return None;
    }
    let slot = column / self.width;
    let index = slot * self.rows + self.first + line;
    (slot < self.columns && index < count).then_some(index)
  }
}

/// Most rows a grid may take.
pub fn limit(screen: Screen) -> usize {
  usize::from(screen.rows / 3).max(1)
}

/// Paints a label; its keys lead.
pub fn cell(
  out: &mut impl Write,
  label: &str,
  width: usize,
  keys: usize,
  selected: bool,
) -> Result<()> {
  let text = terminal::clean(label, width.saturating_sub(2));
  let pad = " ".repeat(width.saturating_sub(1 + text.width()));
  if selected {
    let badge = theme::badge(Color::Blue);
    write!(out, "{badge} {text}{pad}{}", theme::BAR)?;
  } else if keys > 0 && text.len() >= keys && text.is_char_boundary(keys) {
    let (key, rest) = text.split_at(keys);
    write!(out, " {}{key}{}{rest}{pad}", theme::KEY, theme::BAR)?;
  } else {
    write!(out, " {text}{pad}")?;
  }
  Ok(())
}

/// Draws `labels` placed by `grid`, ending at row `bottom`; the first
/// `keys` columns of each label are highlighted as keys.
pub fn draw(
  out: &mut impl Write,
  (at, bottom): (At, u16),
  labels: &[String],
  grid: &Grid,
  selected: Option<usize>,
  keys: usize,
  screen: Screen,
) -> Result<()> {
  let columns = usize::from(screen.columns);
  let top = usize::from(bottom) + 1 - grid.visible;
  for line in 0..grid.visible {
    let row = grid.first + line;
    write!(out, "{}{}", at.goto(top + line, 0u16), theme::BAR)?;
    let mut used = 0;
    for column in 0..grid.columns {
      let index = column * grid.rows + row;
      let Some(label) = labels.get(index) else { break };
      cell(out, label, grid.width, keys, selected == Some(index))?;
      used += grid.width;
    }
    let fill = " ".repeat(columns.saturating_sub(used));
    write!(out, "{fill}{}", theme::RESET)?;
  }
  Ok(())
}
