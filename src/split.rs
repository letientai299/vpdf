//! Window splits within a tab, as a binary tree like Vim's layout.

/// `Rows` stacks panes (`Ctrl-w s`); `Columns` sets them side by side
/// (`Ctrl-w v`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Axis {
  Rows,
  Columns,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
  Left,
  Down,
  Up,
  Right,
}

/// A rectangle of terminal cells.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
  pub column: u16,
  pub row: u16,
  pub columns: u16,
  pub rows: u16,
}

impl Rect {
  pub fn contains(self, column: u16, row: u16) -> bool {
    (self.column..self.column.saturating_add(self.columns)).contains(&column)
      && (self.row..self.row.saturating_add(self.rows)).contains(&row)
  }
}

#[derive(Debug)]
pub enum Node {
  Leaf(u32),
  Split { axis: Axis, ratio: f32, first: Box<Self>, second: Box<Self> },
}

/// Smallest pane: a status row plus three rows, or ten columns.
const MIN_ROWS: u16 = 4;
const MIN_COLUMNS: u16 = 10;

impl Node {
  /// Divides `area` between the halves; side-by-side halves give one
  /// column to the separator.
  fn halves(axis: Axis, ratio: f32, area: Rect) -> (Rect, Rect) {
    match axis {
      Axis::Rows => {
        let first = ((f32::from(area.rows) * ratio).round() as u16)
          .clamp(1, area.rows.saturating_sub(1).max(1))
          .min(area.rows);
        (
          Rect { rows: first, ..area },
          Rect { row: area.row + first, rows: area.rows - first, ..area },
        )
      }
      Axis::Columns => {
        let usable = area.columns.saturating_sub(1);
        let first = ((f32::from(usable) * ratio).round() as u16)
          .clamp(1, usable.saturating_sub(1).max(1))
          .min(usable);
        (
          Rect { columns: first, ..area },
          Rect {
            column: area.column + first + 1,
            columns: usable - first,
            ..area
          },
        )
      }
    }
  }

  /// Pane areas, in tree order.
  pub fn layout(&self, area: Rect, out: &mut Vec<(u32, Rect)>) {
    match self {
      Self::Leaf(id) => out.push((*id, area)),
      Self::Split { axis, ratio, first, second } => {
        let (a, b) = Self::halves(*axis, *ratio, area);
        first.layout(a, out);
        second.layout(b, out);
      }
    }
  }

  /// Separator columns between side-by-side panes.
  pub fn separators(&self, area: Rect, out: &mut Vec<Rect>) {
    if let Self::Split { axis, ratio, first, second } = self {
      let (a, b) = Self::halves(*axis, *ratio, area);
      if *axis == Axis::Columns {
        out.push(Rect { column: a.column + a.columns, columns: 1, ..area });
      }
      first.separators(a, out);
      second.separators(b, out);
    }
  }

  pub fn leaves(&self) -> Vec<u32> {
    match self {
      Self::Leaf(id) => vec![*id],
      Self::Split { first, second, .. } => {
        let mut ids = first.leaves();
        ids.extend(second.leaves());
        ids
      }
    }
  }

  /// Whether `area` holds two panes split along `axis`.
  pub fn fits(axis: Axis, area: Rect) -> bool {
    match axis {
      Axis::Rows => area.rows >= 2 * MIN_ROWS,
      Axis::Columns => area.columns > 2 * MIN_COLUMNS,
    }
  }

  /// Puts `new` before `target`, above or to its left as in Vim.
  pub fn split(&mut self, target: u32, axis: Axis, new: u32) -> bool {
    match self {
      Self::Leaf(id) if *id == target => {
        *self = Self::Split {
          axis,
          ratio: 0.5,
          first: Box::new(Self::Leaf(new)),
          second: Box::new(Self::Leaf(target)),
        };
        true
      }
      Self::Leaf(_) => false,
      Self::Split { first, second, .. } => {
        first.split(target, axis, new) || second.split(target, axis, new)
      }
    }
  }

  /// Drops `target`; its sibling takes the space.
  pub fn remove(&mut self, target: u32) -> bool {
    let Self::Split { first, second, .. } = self else { return false };
    let survivor = match (first.as_ref(), second.as_ref()) {
      (Self::Leaf(id), _) if *id == target => {
        Some(std::mem::replace(second.as_mut(), Self::Leaf(0)))
      }
      (_, Self::Leaf(id)) if *id == target => {
        Some(std::mem::replace(first.as_mut(), Self::Leaf(0)))
      }
      _ => None,
    };
    if let Some(survivor) = survivor {
      *self = survivor;
      return true;
    }
    first.remove(target) || second.remove(target)
  }

  fn contains(&self, target: u32) -> bool {
    match self {
      Self::Leaf(id) => *id == target,
      Self::Split { first, second, .. } => {
        first.contains(target) || second.contains(target)
      }
    }
  }

  /// Grows `target` by `cells` along `along` through its nearest split on
  /// that axis.
  pub fn resize(&mut self, target: u32, along: Axis, cells: i32, area: Rect) {
    let Self::Split { axis, ratio, first, second } = self else { return };
    let (a, b) = Self::halves(*axis, *ratio, area);
    let inside = if first.contains(target) { Some(true) } else { None };
    let inside = inside.or_else(|| second.contains(target).then_some(false));
    let Some(in_first) = inside else { return };
    let (child, child_area) =
      if in_first { (first.as_mut(), a) } else { (second.as_mut(), b) };
    if child.has_split(target, along) {
      child.resize(target, along, cells, child_area);
      return;
    }
    if *axis != along {
      return;
    }
    let (total, minimum) = match along {
      Axis::Rows => (area.rows, MIN_ROWS),
      Axis::Columns => (area.columns.saturating_sub(1), MIN_COLUMNS),
    };
    let current = (f32::from(total) * *ratio).round();
    let delta = if in_first { cells as f32 } else { -(cells as f32) };
    let low = f32::from(minimum.min(total / 2));
    let high = (f32::from(total) - low).max(low);
    *ratio = ((current + delta).clamp(low, high)) / f32::from(total.max(1));
  }

  fn has_split(&self, target: u32, along: Axis) -> bool {
    match self {
      Self::Leaf(_) => false,
      Self::Split { axis, first, second, .. } => {
        (*axis == along && self.contains(target))
          || first.has_split(target, along)
          || second.has_split(target, along)
      }
    }
  }

  /// `Ctrl-w =`: splits share space by how many panes each side holds.
  pub fn equalize(&mut self) {
    if let Self::Split { axis, ratio, first, second } = self {
      let count = |node: &Self| node.count_along(*axis) as f32;
      *ratio = count(first) / (count(first) + count(second));
      first.equalize();
      second.equalize();
    }
  }

  fn count_along(&self, along: Axis) -> usize {
    match self {
      Self::Leaf(_) => 1,
      Self::Split { axis, first, second, .. } if *axis == along => {
        first.count_along(along) + second.count_along(along)
      }
      Self::Split { first, second, .. } => {
        first.count_along(along).max(second.count_along(along))
      }
    }
  }

  /// `Ctrl-w H J K L`: makes `target` span that whole edge of the tab.
  pub fn move_to_edge(&mut self, target: u32, direction: Direction) {
    if matches!(self, Self::Leaf(_)) || !self.remove(target) {
      return;
    }
    let rest = std::mem::replace(self, Self::Leaf(target));
    let (axis, first) = match direction {
      Direction::Left => (Axis::Columns, true),
      Direction::Right => (Axis::Columns, false),
      Direction::Up => (Axis::Rows, true),
      Direction::Down => (Axis::Rows, false),
    };
    let moved = Box::new(Self::Leaf(target));
    let rest = Box::new(rest);
    let (first, second) = if first { (moved, rest) } else { (rest, moved) };
    *self = Self::Split { axis, ratio: 0.5, first, second };
  }

  /// `Ctrl-w r` and `R`: rotates the panes of the row or column holding
  /// `target`. Like Vim, refuses when one of them is itself split.
  pub fn rotate(&mut self, target: u32, forward: bool) -> bool {
    let Self::Split { axis, .. } = self else { return false };
    let axis = *axis;
    if !self.chain_holds(axis, target) {
      let Self::Split { first, second, .. } = self else { return false };
      return first.rotate(target, forward) || second.rotate(target, forward);
    }
    let mut ids = Vec::new();
    if !self.chain_leaves(axis, &mut ids) {
      return false;
    }
    if forward {
      ids.rotate_right(1);
    } else {
      ids.rotate_left(1);
    }
    self.assign(&mut ids.into_iter());
    true
  }

  /// Whether `target` is a pane directly in this row or column.
  fn chain_holds(&self, along: Axis, target: u32) -> bool {
    match self {
      Self::Leaf(id) => *id == target,
      Self::Split { axis, first, second, .. } if *axis == along => {
        first.chain_holds(along, target) || second.chain_holds(along, target)
      }
      Self::Split { .. } => false,
    }
  }

  /// Panes of this row or column; false when one is split across.
  fn chain_leaves(&self, along: Axis, out: &mut Vec<u32>) -> bool {
    match self {
      Self::Leaf(id) => {
        out.push(*id);
        true
      }
      Self::Split { axis, first, second, .. } if *axis == along => {
        first.chain_leaves(along, out) && second.chain_leaves(along, out)
      }
      Self::Split { .. } => false,
    }
  }

  fn assign(&mut self, ids: &mut impl Iterator<Item = u32>) {
    match self {
      Self::Leaf(id) => *id = ids.next().unwrap_or(*id),
      Self::Split { first, second, .. } => {
        first.assign(ids);
        second.assign(ids);
      }
    }
  }

  /// `Ctrl-w x`: swaps `target` with its sibling pane.
  pub fn exchange(&mut self, target: u32) -> bool {
    let Self::Split { first, second, .. } = self else { return false };
    if let (Self::Leaf(a), Self::Leaf(b)) = (first.as_mut(), second.as_mut())
      && (*a == target || *b == target)
    {
      std::mem::swap(a, b);
      return true;
    }
    first.exchange(target) || second.exchange(target)
  }
}

/// The pane `from` reaches moving in `direction`: the nearest pane on
/// that side that overlaps it, preferring the one at its top-left.
pub fn neighbor(
  rects: &[(u32, Rect)],
  from: u32,
  direction: Direction,
) -> Option<u32> {
  let (_, origin) = rects.iter().find(|(id, _)| *id == from)?;
  let overlaps = |a0: u16, a1: u16, b0: u16, b1: u16| a0 < b1 && b0 < a1;
  rects
    .iter()
    .filter(|(id, _)| *id != from)
    .filter_map(|(id, rect)| {
      let rows = overlaps(
        rect.row,
        rect.row + rect.rows,
        origin.row,
        origin.row + origin.rows,
      );
      let columns = overlaps(
        rect.column,
        rect.column + rect.columns,
        origin.column,
        origin.column + origin.columns,
      );
      let distance = match direction {
        Direction::Left if rows && rect.column < origin.column => {
          origin.column - (rect.column + rect.columns)
        }
        Direction::Right if rows && rect.column > origin.column => {
          rect.column - (origin.column + origin.columns)
        }
        Direction::Up if columns && rect.row < origin.row => {
          origin.row - (rect.row + rect.rows)
        }
        Direction::Down if columns && rect.row > origin.row => {
          rect.row - (origin.row + origin.rows)
        }
        _ => return None,
      };
      Some((distance, rect.row, rect.column, *id))
    })
    .min()
    .map(|(.., id)| id)
}

#[cfg(test)]
mod tests {
  use super::*;

  const AREA: Rect = Rect { column: 0, row: 0, columns: 101, rows: 40 };

  fn rects(node: &Node) -> Vec<(u32, Rect)> {
    let mut out = Vec::new();
    node.layout(AREA, &mut out);
    out
  }

  #[test]
  fn splits_place_new_panes_first_and_close_cleanly() {
    let mut tree = Node::Leaf(1);
    assert!(tree.split(1, Axis::Columns, 2));
    assert!(tree.split(1, Axis::Rows, 3));
    assert_eq!(tree.leaves(), [2, 3, 1]);
    let layout = rects(&tree);
    assert_eq!(layout[0].1, Rect { column: 0, row: 0, columns: 50, rows: 40 });
    assert_eq!(layout[1].1, Rect { column: 51, row: 0, columns: 50, rows: 20 });
    assert_eq!(
      layout[2].1,
      Rect { column: 51, row: 20, columns: 50, rows: 20 }
    );
    let mut separators = Vec::new();
    tree.separators(AREA, &mut separators);
    assert_eq!(separators, [Rect { column: 50, row: 0, columns: 1, rows: 40 }]);
    assert_eq!(neighbor(&layout, 2, Direction::Right), Some(3));
    assert_eq!(neighbor(&layout, 1, Direction::Up), Some(3));
    assert_eq!(neighbor(&layout, 3, Direction::Left), Some(2));
    assert_eq!(neighbor(&layout, 2, Direction::Left), None);
    assert!(tree.remove(3));
    assert_eq!(tree.leaves(), [2, 1]);
    assert!(tree.remove(2));
    assert!(matches!(tree, Node::Leaf(1)));
  }

  #[test]
  fn tiny_areas_fit_their_panes() {
    let mut tree = Node::Leaf(1);
    tree.split(1, Axis::Columns, 2);
    tree.split(2, Axis::Rows, 3);
    for (columns, rows) in [(0, 0), (1, 1), (2, 2), (3, 0)] {
      let mut out = Vec::new();
      tree.layout(Rect { columns, rows, ..Rect::default() }, &mut out);
      assert!(out.iter().all(|(_, r)| r.columns <= columns && r.rows <= rows));
    }
    assert!(!Node::fits(Axis::Rows, Rect { rows: 7, ..AREA }));
    assert!(Node::fits(Axis::Rows, Rect { rows: 8, ..AREA }));
    assert!(!Node::fits(Axis::Columns, Rect { columns: 20, ..AREA }));
    assert!(Node::fits(Axis::Columns, Rect { columns: 21, ..AREA }));
  }

  #[test]
  fn resize_equalize_and_exchange() {
    let mut tree = Node::Leaf(1);
    tree.split(1, Axis::Rows, 2);
    tree.resize(2, Axis::Rows, 6, AREA);
    assert_eq!(rects(&tree)[0].1.rows, 26);
    tree.resize(1, Axis::Rows, 100, AREA);
    assert_eq!(rects(&tree)[0].1.rows, MIN_ROWS);
    tree.resize(1, Axis::Columns, 5, AREA);
    assert_eq!(rects(&tree)[0].1.rows, MIN_ROWS);
    tree.equalize();
    assert_eq!(rects(&tree)[0].1.rows, 20);
    assert!(tree.exchange(1));
    assert_eq!(tree.leaves(), [1, 2]);
    tree.split(2, Axis::Rows, 3);
    tree.move_to_edge(3, Direction::Right);
    assert_eq!(tree.leaves(), [1, 2, 3]);
    let layout = rects(&tree);
    assert_eq!(layout[2].1, Rect { column: 51, row: 0, columns: 50, rows: 40 });
    assert!(!tree.rotate(3, true), "its row holds a split column");
    assert!(tree.rotate(1, true));
    assert_eq!(tree.leaves(), [2, 1, 3]);
    assert!(tree.rotate(1, false));
    assert_eq!(tree.leaves(), [1, 2, 3]);
    tree.move_to_edge(1, Direction::Down);
    assert_eq!(
      rects(&tree)[2].1,
      Rect { column: 0, row: 20, columns: 101, rows: 20 }
    );
  }
}
