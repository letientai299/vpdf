//! Items in columns filled top to bottom, like zsh's completion list,
//! drawn in rows above the status line.

use crate::layout::Screen;
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Draw, Op, Text};
use vpdf_model::style::Role;

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
  /// Each shown label's cells, below row `top`.
  pub fn cells(
    &self,
    count: usize,
    top: u16,
  ) -> impl Iterator<Item = (crate::split::Rect, usize)> + '_ {
    (0..count).filter_map(move |index| {
      let line = (index % self.rows).checked_sub(self.first)?;
      let slot = index / self.rows;
      (line < self.visible).then(|| {
        let at = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
        let cells = crate::split::Rect {
          column: at(slot * self.width),
          row: top.saturating_add(at(line)),
          columns: at(self.width),
          rows: 1,
        };
        (cells, index)
      })
    })
  }
}

/// Most rows a grid may take.
pub fn limit(screen: Screen) -> usize {
  usize::from(screen.rows / 3).max(1)
}

/// How a cell shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shown {
  Plain,
  /// Under the pointer.
  Pointed,
  /// Chosen; a key or click runs it.
  Chosen,
}

pub fn scene_cell<'a>(
  scene: &mut impl Draw<'a>,
  label: &str,
  width: usize,
  keys: usize,
  shown: Shown,
) {
  let text = vpdf_model::text::clean(label, width.saturating_sub(2));
  let pad = width.saturating_sub(1 + text.width());
  if shown == Shown::Chosen {
    scene.extend([
      Op::Style(Role::Chosen),
      Op::Text(Text::new(" ")),
      Op::Text(Text::new(text)),
      Op::Spaces(pad),
      Op::Style(Role::Bar),
    ]);
    return;
  }
  scene.push(Op::Text(Text::new(" ")));
  if shown == Shown::Pointed {
    scene.extend([
      Op::Style(Role::BarHover),
      Op::Text(Text::new(text)),
      Op::Style(Role::Bar),
    ]);
  } else if keys > 0 && text.len() >= keys && text.is_char_boundary(keys) {
    let (key, rest) = text.split_at(keys);
    scene.extend([
      Op::Style(Role::Key),
      Op::Text(Text::new(key.to_owned())),
      Op::Style(Role::Bar),
      Op::Text(Text::new(rest.to_owned())),
    ]);
  } else {
    scene.push(Op::Text(Text::new(text)));
  }
  scene.push(Op::Spaces(pad));
}

pub fn scene<'a>(
  scene: &mut impl Draw<'a>,
  (at, bottom): (At, u16),
  labels: &[String],
  grid: &Grid,
  (chosen, pointed): (Option<usize>, Option<usize>),
  keys: usize,
  screen: Screen,
) {
  let columns = usize::from(screen.columns);
  let top = usize::from(bottom) + 1 - grid.visible;
  for line in 0..grid.visible {
    let row = grid.first + line;
    crate::paint::move_to(scene, at, top + line, 0u16);
    scene.push(Op::Style(Role::Bar));
    let mut used = 0;
    for column in 0..grid.columns {
      let index = column * grid.rows + row;
      let Some(label) = labels.get(index) else { break };
      let shown = if chosen == Some(index) {
        Shown::Chosen
      } else if pointed == Some(index) {
        Shown::Pointed
      } else {
        Shown::Plain
      };
      scene_cell(scene, label, grid.width, keys, shown);
      used += grid.width;
    }
    scene.extend([
      Op::Spaces(columns.saturating_sub(used)),
      Op::Style(Role::Reset),
    ]);
  }
}
