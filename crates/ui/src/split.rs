//! Window splits within a tab, as a binary tree like Vim's layout.

use vpdf_model::scene::{Draw, Op, Text};
use vpdf_model::style::Role;

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

/// A split's boundary, which drags move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Divider {
  /// The split's place in tree order.
  pub index: usize,
  pub axis: Axis,
  /// Separator column or first half's last row.
  pub rect: Rect,
}

/// Where a new pane goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Place {
  /// Above or left, as in Vim.
  Before,
  After,
}

#[derive(Debug)]
pub struct Node {
  kind: Kind,
}

#[derive(Debug)]
enum Kind {
  Leaf(u32),
  Split { axis: Axis, ratio: f32, first: Box<Node>, second: Box<Node> },
}

/// Smallest pane: a status row plus three rows, or ten columns.
const MIN_ROWS: u16 = 4;
const MIN_COLUMNS: u16 = 10;

/// Cells a split divides, minus separators.
fn span(axis: Axis, area: Rect) -> u16 {
  match axis {
    Axis::Rows => area.rows,
    Axis::Columns => area.columns.saturating_sub(1),
  }
}

impl Node {
  pub fn leaf(id: u32) -> Self {
    Self { kind: Kind::Leaf(id) }
  }

  pub fn scene_dividers<'a>(&self, scene: &mut impl Draw<'a>, area: Rect) {
    let mut dividers = Vec::new();
    self.dividers(area, &mut dividers);
    let columns =
      dividers.iter().filter(|divider| divider.axis == Axis::Columns);
    for rect in columns.map(|divider| divider.rect) {
      for row in 0..rect.rows {
        scene.extend([
          Op::Move {
            column: usize::from(rect.column),
            row: usize::from(rect.row) + usize::from(row),
          },
          Op::Style(Role::Muted),
          Op::Text(Text::new("│")),
          Op::Style(Role::Reset),
        ]);
      }
    }
  }

  pub fn replace(&mut self, from: u32, to: u32) {
    match &mut self.kind {
      Kind::Leaf(id) if *id == from => *id = to,
      Kind::Leaf(_) => {}
      Kind::Split { first, second, .. } => {
        first.replace(from, to);
        second.replace(from, to);
      }
    }
  }

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
    match &self.kind {
      Kind::Leaf(id) => out.push((*id, area)),
      Kind::Split { axis, ratio, first, second } => {
        let (a, b) = Self::halves(*axis, *ratio, area);
        first.layout(a, out);
        second.layout(b, out);
      }
    }
  }

  /// Every split's divider, in tree order.
  pub fn dividers(&self, area: Rect, out: &mut Vec<Divider>) {
    if let Kind::Split { axis, ratio, first, second } = &self.kind {
      let (a, b) = Self::halves(*axis, *ratio, area);
      let rect = match axis {
        Axis::Columns => {
          Rect { column: a.column + a.columns, columns: 1, ..area }
        }
        Axis::Rows => {
          Rect { row: a.row + a.rows.saturating_sub(1), rows: 1, ..area }
        }
      };
      out.push(Divider { index: out.len(), axis: *axis, rect });
      first.dividers(a, out);
      second.dividers(b, out);
    }
  }

  /// Moves divider `index` to cell `at`.
  pub fn drag(&mut self, index: usize, at: u16, area: Rect) {
    let Some((node, area)) = self.nth(index, area) else { return };
    let cells = match &node.kind {
      Kind::Split { axis: Axis::Columns, .. } => at.saturating_sub(area.column),
      // Its status row is the divider.
      Kind::Split { axis: Axis::Rows, .. } => (at + 1).saturating_sub(area.row),
      Kind::Leaf(_) => return,
    };
    node.set_first(f32::from(cells), area);
    if let Kind::Split { axis, .. } = &node.kind {
      let axis = *axis;
      node.refit(axis, area);
    }
  }

  /// Equalizes the panes divider `index` splits.
  pub fn equalize_at(&mut self, index: usize, area: Rect) {
    if let Some((node, _)) = self.nth(index, area)
      && let Kind::Split { axis, .. } = &node.kind
    {
      let axis = *axis;
      node.equalize_along(axis);
    }
  }

  /// `equalize`, leaving cross-axis splits.
  fn equalize_along(&mut self, along: Axis) {
    let Kind::Split { axis, ratio, first, second } = &mut self.kind else {
      return;
    };
    if *axis == along {
      let count = |node: &Self| node.count_along(along) as f32;
      *ratio = count(first) / (count(first) + count(second));
    }
    first.equalize_along(along);
    second.equalize_along(along);
  }

  /// Split `index` in tree order, with area.
  fn nth(&mut self, index: usize, area: Rect) -> Option<(&mut Self, Rect)> {
    let Kind::Split { axis, ratio, first, .. } = &mut self.kind else {
      return None;
    };
    if index == 0 {
      return Some((self, area));
    }
    let (a, b) = Self::halves(*axis, *ratio, area);
    let before = first.splits();
    let Kind::Split { first, second, .. } = &mut self.kind else { return None };
    if index <= before {
      first.nth(index - 1, a)
    } else {
      second.nth(index - 1 - before, b)
    }
  }

  fn splits(&self) -> usize {
    match &self.kind {
      Kind::Leaf(_) => 0,
      Kind::Split { first, second, .. } => 1 + first.splits() + second.splits(),
    }
  }

  pub fn leaves(&self) -> Vec<u32> {
    match &self.kind {
      Kind::Leaf(id) => vec![*id],
      Kind::Split { first, second, .. } => {
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

  /// Puts `new` beside `target`.
  pub fn split(
    &mut self,
    target: u32,
    axis: Axis,
    new: u32,
    place: Place,
  ) -> bool {
    match &mut self.kind {
      Kind::Leaf(id) if *id == target => {
        let (new, old) =
          (Box::new(Self::leaf(new)), Box::new(Self::leaf(target)));
        let (first, second) = match place {
          Place::Before => (new, old),
          Place::After => (old, new),
        };
        self.kind = Kind::Split { axis, ratio: 0.5, first, second };
        true
      }
      Kind::Leaf(_) => false,
      Kind::Split { first, second, .. } => {
        first.split(target, axis, new, place)
          || second.split(target, axis, new, place)
      }
    }
  }

  /// Drops `target`; its sibling takes the space.
  pub fn remove(&mut self, target: u32) -> bool {
    let Kind::Split { first, second, .. } = &mut self.kind else {
      return false;
    };
    let survivor = match (&first.kind, &second.kind) {
      (Kind::Leaf(id), _) if *id == target => {
        Some(std::mem::replace(second.as_mut(), Self::leaf(0)))
      }
      (_, Kind::Leaf(id)) if *id == target => {
        Some(std::mem::replace(first.as_mut(), Self::leaf(0)))
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
    match &self.kind {
      Kind::Leaf(id) => *id == target,
      Kind::Split { first, second, .. } => {
        first.contains(target) || second.contains(target)
      }
    }
  }

  /// Grows `target` by `cells` along `along` through its nearest split on
  /// that axis.
  pub fn resize(&mut self, target: u32, along: Axis, cells: i32, area: Rect) {
    let Kind::Split { axis, ratio, first, second } = &mut self.kind else {
      return;
    };
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
    self.grow(in_first, cells, area);
  }

  /// Moves the nearest boundary facing `direction`.
  pub fn extend(
    &mut self,
    target: u32,
    direction: Direction,
    cells: i32,
    area: Rect,
  ) -> bool {
    let Kind::Split { axis, ratio, first, second } = &mut self.kind else {
      return false;
    };
    let in_first = first.contains(target);
    if !in_first && !second.contains(target) {
      return false;
    }
    let (a, b) = Self::halves(*axis, *ratio, area);
    let (child, child_area) = if in_first { (first, a) } else { (second, b) };
    if child.extend(target, direction, cells, child_area) {
      return true;
    }
    let (along, toward_first) = match direction {
      Direction::Left => (Axis::Columns, true),
      Direction::Right => (Axis::Columns, false),
      Direction::Up => (Axis::Rows, true),
      Direction::Down => (Axis::Rows, false),
    };
    if *axis != along || in_first == toward_first {
      return false;
    }
    self.grow(in_first, cells, area);
    true
  }

  fn grow(&mut self, in_first: bool, cells: i32, area: Rect) {
    let Kind::Split { axis, ratio, .. } = self.kind else { return };
    let current = (f32::from(span(axis, area)) * ratio).round();
    let delta = if in_first { cells as f32 } else { -(cells as f32) };
    self.set_first(current + delta, area);
    self.refit(axis, area);
  }

  /// Sizes the first half within both minimums.
  fn set_first(&mut self, cells: f32, area: Rect) {
    let Some((low, high)) = self.bounds(area) else { return };
    let Kind::Split { axis, ratio, .. } = &mut self.kind else { return };
    let cells = cells.clamp(f32::from(low), f32::from(high));
    *ratio = cells / f32::from(span(*axis, area).max(1));
  }

  /// The first half's fewest and most cells.
  fn bounds(&self, area: Rect) -> Option<(u16, u16)> {
    let Kind::Split { axis, first, second, .. } = &self.kind else {
      return None;
    };
    let total = span(*axis, area);
    // Cramped spans still move, within halves.
    let low = first.least(*axis).min(total / 2);
    let high = total.saturating_sub(second.least(*axis).min(total / 2));
    Some((low, high.max(low)))
  }

  /// Re-clamps nested `along` splits a drag squeezed.
  fn refit(&mut self, along: Axis, area: Rect) {
    let bounds = self.bounds(area);
    let Kind::Split { axis, ratio, first, second } = &mut self.kind else {
      return;
    };
    if *axis == along
      && let Some((low, high)) = bounds
    {
      let total = f32::from(span(along, area).max(1));
      let cells = (total * *ratio).round() as u16;
      let fit = cells.clamp(low, high);
      // Untouched splits keep their ratios.
      if fit != cells {
        *ratio = f32::from(fit) / total;
      }
    }
    let (a, b) = Self::halves(*axis, *ratio, area);
    first.refit(along, a);
    second.refit(along, b);
  }

  /// Fewest cells keeping every pane usable.
  fn least(&self, along: Axis) -> u16 {
    match &self.kind {
      Kind::Leaf(_) => match along {
        Axis::Rows => MIN_ROWS,
        Axis::Columns => MIN_COLUMNS,
      },
      Kind::Split { axis, first, second, .. } if *axis == along => {
        let separator = u16::from(along == Axis::Columns);
        first.least(along) + separator + second.least(along)
      }
      Kind::Split { first, second, .. } => {
        first.least(along).max(second.least(along))
      }
    }
  }

  fn has_split(&self, target: u32, along: Axis) -> bool {
    match &self.kind {
      Kind::Leaf(_) => false,
      Kind::Split { axis, first, second, .. } => {
        (*axis == along && self.contains(target))
          || first.has_split(target, along)
          || second.has_split(target, along)
      }
    }
  }

  /// `Ctrl-w =`: splits share space by how many panes each side holds.
  pub fn equalize(&mut self) {
    if let Kind::Split { axis, ratio, first, second } = &mut self.kind {
      let count = |node: &Self| node.count_along(*axis) as f32;
      *ratio = count(first) / (count(first) + count(second));
      first.equalize();
      second.equalize();
    }
  }

  fn count_along(&self, along: Axis) -> usize {
    match &self.kind {
      Kind::Leaf(_) => 1,
      Kind::Split { axis, first, second, .. } if *axis == along => {
        first.count_along(along) + second.count_along(along)
      }
      Kind::Split { first, second, .. } => {
        first.count_along(along).max(second.count_along(along))
      }
    }
  }

  /// `Ctrl-w H J K L`: makes `target` span that whole edge of the tab.
  pub fn move_to_edge(&mut self, target: u32, direction: Direction) {
    if matches!(&self.kind, Kind::Leaf(_)) || !self.remove(target) {
      return;
    }
    let rest = std::mem::replace(self, Self::leaf(target));
    let (axis, first) = match direction {
      Direction::Left => (Axis::Columns, true),
      Direction::Right => (Axis::Columns, false),
      Direction::Up => (Axis::Rows, true),
      Direction::Down => (Axis::Rows, false),
    };
    let moved = Box::new(Self::leaf(target));
    let rest = Box::new(rest);
    let (first, second) = if first { (moved, rest) } else { (rest, moved) };
    self.kind = Kind::Split { axis, ratio: 0.5, first, second };
  }

  /// `Ctrl-w r` and `R`: rotates the panes of the row or column holding
  /// `target`. Like Vim, refuses when one of them is itself split.
  pub fn rotate(&mut self, target: u32, forward: bool) -> bool {
    let Kind::Split { axis, .. } = &mut self.kind else { return false };
    let axis = *axis;
    if !self.chain_holds(axis, target) {
      let Kind::Split { first, second, .. } = &mut self.kind else {
        return false;
      };
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
    match &self.kind {
      Kind::Leaf(id) => *id == target,
      Kind::Split { axis, first, second, .. } if *axis == along => {
        first.chain_holds(along, target) || second.chain_holds(along, target)
      }
      Kind::Split { .. } => false,
    }
  }

  /// Panes of this row or column; false when one is split across.
  fn chain_leaves(&self, along: Axis, out: &mut Vec<u32>) -> bool {
    match &self.kind {
      Kind::Leaf(id) => {
        out.push(*id);
        true
      }
      Kind::Split { axis, first, second, .. } if *axis == along => {
        first.chain_leaves(along, out) && second.chain_leaves(along, out)
      }
      Kind::Split { .. } => false,
    }
  }

  fn assign(&mut self, ids: &mut impl Iterator<Item = u32>) {
    match &mut self.kind {
      Kind::Leaf(id) => *id = ids.next().unwrap_or(*id),
      Kind::Split { first, second, .. } => {
        first.assign(ids);
        second.assign(ids);
      }
    }
  }

  /// `Ctrl-w x`: swaps `target` with its sibling pane.
  pub fn exchange(&mut self, target: u32) -> bool {
    let Kind::Split { first, second, .. } = &mut self.kind else {
      return false;
    };
    if let (Kind::Leaf(a), Kind::Leaf(b)) = (&mut first.kind, &mut second.kind)
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
  fn replacing_preserves_layout() {
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Columns, 2, Place::Before);
    tree.split(1, Axis::Rows, 3, Place::After);
    tree.drag(0, 35, AREA);
    for old in [1, 2, 3, 99] {
      let mut expected = rects(&tree);
      for (id, _) in &mut expected {
        if *id == old {
          *id += 10;
        }
      }
      tree.replace(old, old + 10);
      assert_eq!(rects(&tree), expected);
    }
  }

  #[test]
  fn splits_place_new_panes_first_and_close_cleanly() {
    let mut tree = Node::leaf(1);
    assert!(tree.split(1, Axis::Columns, 2, Place::Before));
    assert!(tree.split(1, Axis::Rows, 3, Place::Before));
    assert_eq!(tree.leaves(), [2, 3, 1]);
    let layout = rects(&tree);
    assert_eq!(layout[0].1, Rect { column: 0, row: 0, columns: 50, rows: 40 });
    assert_eq!(layout[1].1, Rect { column: 51, row: 0, columns: 50, rows: 20 });
    assert_eq!(
      layout[2].1,
      Rect { column: 51, row: 20, columns: 50, rows: 20 }
    );
    let mut dividers = Vec::new();
    tree.dividers(AREA, &mut dividers);
    let rects: Vec<_> = dividers.iter().map(|d| (d.index, d.rect)).collect();
    assert_eq!(
      rects,
      [
        (0, Rect { column: 50, row: 0, columns: 1, rows: 40 }),
        (1, Rect { column: 51, row: 19, columns: 50, rows: 1 }),
      ]
    );
    assert_eq!(neighbor(&layout, 2, Direction::Right), Some(3));
    assert_eq!(neighbor(&layout, 1, Direction::Up), Some(3));
    assert_eq!(neighbor(&layout, 3, Direction::Left), Some(2));
    assert_eq!(neighbor(&layout, 2, Direction::Left), None);
    assert!(tree.remove(3));
    assert_eq!(tree.leaves(), [2, 1]);
    assert!(tree.remove(2));
    assert_eq!(tree.leaves(), [1]);
  }

  #[test]
  fn draws_only_column_dividers() {
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Rows, 2, Place::After);
    tree.split(2, Axis::Columns, 3, Place::After);
    let area = Rect { column: 3, row: 2, columns: 21, rows: 8 };
    let mut scene = vpdf_model::scene::Scene::new();
    tree.scene_dividers(&mut scene, area);
    let moves: Vec<_> = scene
      .iter()
      .filter_map(|op| match op {
        Op::Move { row, column } => Some((*row, *column)),
        _ => None,
      })
      .collect();
    assert_eq!(moves, [(6, 13), (7, 13), (8, 13), (9, 13)]);
    assert_eq!(crate::testing::text(&scene), "││││");
    scene.clear();
    tree.scene_dividers(&mut scene, Rect { rows: 0, ..area });
    assert_eq!(scene, [] as [Op<'_>; 0]);
  }

  #[test]
  fn dividers_follow_drags_and_equalize() {
    // Columns [1 | Rows [2 / Columns [3 | 4]]].
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Columns, 2, Place::After);
    tree.split(2, Axis::Rows, 3, Place::After);
    tree.split(3, Axis::Columns, 4, Place::After);
    // (divider, to) -> panes 1-3 (columns, rows)
    let cases = [
      (0, 30, [(30, 40), (70, 20), (35, 20)]),
      (0, 2, [(10, 40), (90, 20), (45, 20)]),
      // Right panes keep ten columns.
      (0, 200, [(79, 40), (21, 20), (10, 20)]),
      (1, 9, [(79, 40), (21, 10), (10, 30)]),
      (1, 0, [(79, 40), (21, 4), (10, 36)]),
    ];
    for (index, at, want) in cases {
      tree.drag(index, at, AREA);
      let got: Vec<_> =
        rects(&tree).iter().take(3).map(|(_, r)| (r.columns, r.rows)).collect();
      assert_eq!(got, want, "divider {index} to {at}");
    }
    tree.equalize_at(1, AREA);
    let got: Vec<_> = rects(&tree).iter().map(|(_, r)| r.rows).collect();
    assert_eq!(got, [40, 20, 20, 20]);
    assert_eq!(rects(&tree)[0].1.columns, 79, "other splits stay");
  }

  #[test]
  fn shrinking_refits_nested_splits() {
    // (axis, area, drags) -> cells along axis
    let narrow = Rect { columns: 30, rows: 10, ..AREA };
    let cases = [
      (Axis::Columns, AREA, vec![(1, 0), (0, 200)], [79, 10, 10]),
      (Axis::Rows, AREA, vec![(1, 39), (0, 39)], [32, 4, 4]),
      // Cramped spans still move, within halves.
      (Axis::Columns, narrow, vec![(0, 3)], [10, 9, 9]),
    ];
    for (axis, area, drags, want) in cases {
      let mut tree = Node::leaf(1);
      tree.split(1, axis, 2, Place::After);
      tree.split(2, axis, 3, Place::After);
      for (index, at) in drags {
        tree.drag(index, at, area);
      }
      let mut out = Vec::new();
      tree.layout(area, &mut out);
      let got: Vec<_> = out
        .iter()
        .map(|(_, r)| if axis == Axis::Rows { r.rows } else { r.columns })
        .collect();
      assert_eq!(got, want, "{axis:?} {area:?}");
    }
    // Column drags keep stacked proportions: [1 | [2 / 3]].
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Columns, 2, Place::After);
    tree.split(2, Axis::Rows, 3, Place::After);
    tree.drag(1, 35, AREA);
    tree.drag(0, 30, Rect { rows: 12, ..AREA });
    let rows: Vec<_> = rects(&tree).iter().map(|(_, r)| r.rows).collect();
    assert_eq!(rows, [40, 36, 4]);
    // Unclamped nested splits keep exact ratios.
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Columns, 2, Place::After);
    tree.split(2, Axis::Columns, 3, Place::After);
    tree.drag(0, 78, AREA);
    tree.drag(0, 10, AREA);
    let columns: Vec<_> = rects(&tree).iter().map(|(_, r)| r.columns).collect();
    assert_eq!(columns, [10, 45, 44]);
  }

  #[test]
  fn places_put_new_panes_on_either_side() {
    // (place) -> leaves
    let cases = [(Place::Before, [2, 1]), (Place::After, [1, 2])];
    for (place, leaves) in cases {
      let mut tree = Node::leaf(1);
      assert!(tree.split(1, Axis::Columns, 2, place));
      assert_eq!(tree.leaves(), leaves, "{place:?}");
    }
  }

  #[test]
  fn tiny_areas_fit_their_panes() {
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Columns, 2, Place::Before);
    tree.split(2, Axis::Rows, 3, Place::Before);
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
  fn extend_moves_facing_divider() {
    use Direction::{Down, Left, Right, Up};
    for (axis, direction, target, grows) in [
      (Axis::Columns, Left, 1, false),
      (Axis::Columns, Right, 1, true),
      (Axis::Columns, Left, 2, true),
      (Axis::Columns, Right, 2, false),
      (Axis::Rows, Up, 1, false),
      (Axis::Rows, Down, 1, true),
      (Axis::Rows, Up, 2, true),
      (Axis::Rows, Down, 2, false),
      (Axis::Rows, Left, 1, false),
      (Axis::Columns, Down, 2, false),
    ] {
      let mut tree = Node::leaf(1);
      tree.split(1, axis, 2, Place::After);
      let before = rects(&tree);
      let mut dividers = Vec::new();
      tree.dividers(AREA, &mut dividers);
      let old = dividers[0].rect;
      assert_eq!(tree.extend(target, direction, 3, AREA), grows);
      let after = rects(&tree);
      let index = usize::try_from(target - 1).unwrap();
      let size = |r: Rect| if axis == Axis::Rows { r.rows } else { r.columns };
      assert_eq!(
        size(after[index].1),
        size(before[index].1) + if grows { 3 } else { 0 }
      );
      dividers.clear();
      tree.dividers(AREA, &mut dividers);
      let mut want = old;
      if grows {
        match direction {
          Left => want.column -= 3,
          Right => want.column += 3,
          Up => want.row -= 3,
          Down => want.row += 3,
        }
      }
      assert_eq!(dividers[0].rect, want);
    }
  }

  #[test]
  fn extend_selects_nested_boundary() {
    for (axis, backward, forward) in [
      (Axis::Columns, Direction::Left, Direction::Right),
      (Axis::Rows, Direction::Up, Direction::Down),
    ] {
      let mut tree = Node::leaf(1);
      tree.split(1, axis, 2, Place::After);
      tree.split(2, axis, 3, Place::After);
      let before = rects(&tree);
      assert!(tree.extend(2, backward, 2, AREA));
      let after = rects(&tree);
      assert_ne!(before[0].1, after[0].1);
      let edge = after[0].1;
      assert!(tree.extend(2, forward, 2, AREA));
      assert_eq!(rects(&tree)[0].1, edge);
      assert!(tree.extend(2, forward, i32::from(u16::MAX), AREA));
      let limited = rects(&tree);
      let minimum = if axis == Axis::Rows { MIN_ROWS } else { MIN_COLUMNS };
      assert_eq!(
        if axis == Axis::Rows {
          limited[2].1.rows
        } else {
          limited[2].1.columns
        },
        minimum
      );
      assert!(tree.extend(2, forward, 1, AREA));
      assert_eq!(rects(&tree), limited);
      assert!(!tree.extend(99, forward, 1, AREA));
    }
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Columns, 2, Place::After);
    tree.split(2, Axis::Rows, 3, Place::After);
    let before = rects(&tree);
    assert!(tree.extend(3, Direction::Left, 3, AREA));
    let after = rects(&tree);
    assert_eq!(after[2].1.columns, before[2].1.columns + 3);
    assert_eq!(after[2].1.row, before[2].1.row);
    assert_eq!(after[2].1.rows, before[2].1.rows);
    assert!(tree.extend(3, Direction::Up, 3, AREA));
    assert_eq!(rects(&tree)[0].1, after[0].1);
  }

  #[test]
  fn resize_equalize_and_exchange() {
    let mut tree = Node::leaf(1);
    tree.split(1, Axis::Rows, 2, Place::Before);
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
    tree.split(2, Axis::Rows, 3, Place::Before);
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
