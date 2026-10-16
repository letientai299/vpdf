//! Items in columns filled top to bottom, like zsh's completion list,
//! drawn in rows above the status line.

use crate::Result;
use crate::layout::Screen;
use crate::terminal;
use crate::theme::{self, Color};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

pub struct Grid {
  pub labels: Vec<String>,
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
  labels: Vec<String>,
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
  Grid { labels, width, columns: per_row, rows, first, visible }
}

/// Most rows a grid may take.
pub fn limit(screen: Screen) -> usize {
  usize::from(screen.rows / 3).max(1)
}

/// Draws `grid` ending at row `bottom`; the first `keys` columns of each
/// label are highlighted as keys.
pub fn draw(
  out: &mut impl Write,
  bottom: u16,
  grid: &Grid,
  selected: Option<usize>,
  keys: usize,
  screen: Screen,
) -> Result<()> {
  let columns = usize::from(screen.columns);
  let top = usize::from(bottom) + 1 - grid.visible;
  for line in 0..grid.visible {
    let row = grid.first + line;
    write!(out, "\x1b[{};1H{}", top + line + 1, theme::BAR)?;
    let mut used = 0;
    for column in 0..grid.columns {
      let index = column * grid.rows + row;
      let Some(label) = grid.labels.get(index) else { break };
      let text = terminal::clean(label, grid.width - 2);
      let pad = " ".repeat(grid.width - 1 - text.width());
      if selected == Some(index) {
        let badge = theme::badge(Color::Blue);
        write!(out, "{badge} {text}{pad}{}", theme::BAR)?;
      } else if keys > 0 && text.len() >= keys && text.is_char_boundary(keys) {
        let (key, rest) = text.split_at(keys);
        write!(out, " {}{key}{}{rest}{pad}", theme::KEY, theme::BAR)?;
      } else {
        write!(out, " {text}{pad}")?;
      }
      used += grid.width;
    }
    let fill = " ".repeat(columns.saturating_sub(used));
    write!(out, "{fill}{}", theme::RESET)?;
  }
  Ok(())
}
