use crate::text::{Bounds, PageId, Position};
use unicode_width::UnicodeWidthStr;
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::{Draw, Layer, Op, Stage, Text};
use vpdf_reader::reader::Reader;
use vpdf_search::search::Hit;

pub fn move_to<'a>(
  scene: &mut impl Draw<'a>,
  at: At,
  row: impl Into<usize>,
  column: impl Into<usize>,
) {
  scene.push(Op::Move {
    row: row.into().saturating_add(usize::from(at.row)),
    column: column.into().saturating_add(usize::from(at.column)),
  });
}

pub fn clear<'a>(scene: &mut impl Draw<'a>, at: At, screen: Screen) {
  for row in 0..screen.rows {
    move_to(scene, at, row, 0u16);
    scene.push(Op::Erase(screen.columns));
  }
}

pub fn line<'a>(
  scene: &mut impl Draw<'a>,
  at: At,
  row: u16,
  text: &str,
  columns: u16,
) {
  let text = vpdf_model::text::clean(text, usize::from(columns));
  let pad = usize::from(columns).saturating_sub(text.width());
  move_to(scene, at, row, 0u16);
  scene.extend([Op::Text(Text::new(text)), Op::Spaces(pad)]);
}

/// Hints separate entries with two spaces.
pub fn key_hints<'a>(scene: &mut impl Draw<'a>, text: &str) {
  for hint in text.split_inclusive("  ") {
    let end = hint.find(' ').unwrap_or(hint.len());
    scene.push(Op::Style(vpdf_model::style::Role::Bar));
    scene.push(Op::Style(vpdf_model::style::Role::Key));
    scene.text(&hint[..end]);
    scene.push(Op::Style(vpdf_model::style::Role::Bar));
    scene.text(&hint[end..]);
  }
}

pub struct Marks<'a> {
  pub reader: &'a Reader,
  pub matches: &'a [Hit],
  pub flash: Option<Position>,
  pub landing: Option<Position>,
  pub cursorline: bool,
  pub dim: bool,
  pub hovered: &'a [(PageId, Bounds)],
}

#[derive(Clone, Copy)]
pub struct Frame {
  pub at: At,
  pub screen: Screen,
  pub left: u16,
  pub text: bool,
}

impl Frame {
  pub fn stage(self) -> Stage {
    Stage { screen: self.screen, left: self.left }
  }

  /// Page pixels at reported or centered cells.
  pub fn point(
    self,
    column: u16,
    row: u16,
    pixel: Option<crate::pointer::Pixel>,
  ) -> (f32, f32) {
    // Clamped drags report pixels past the cell.
    let inside = |at: Option<u16>, cell: u32, size: u16| {
      at.map_or(f32::from(size) / 2.0, |at| {
        let offset = u32::from(at).saturating_sub(cell * u32::from(size));
        let last = size.saturating_sub(1);
        f32::from(u16::try_from(offset).unwrap_or(last).min(last))
      })
    };
    let (width, height) = (self.screen.cell_width, self.screen.cell_height);
    let (screen_column, screen_row) = (
      u32::from(self.at.column) + u32::from(column),
      u32::from(self.at.row) + u32::from(row),
    );
    let left = f32::from(column.saturating_sub(self.left)) * f32::from(width);
    let x = left + inside(pixel.map(|p| p.x), screen_column, width);
    let top = f32::from(row) * f32::from(height);
    (x, top + inside(pixel.map(|p| p.y), screen_row, height))
  }
}

pub fn cell_bounds(
  screen: Screen,
  left: u16,
  cells: crate::split::Rect,
) -> Bounds {
  let (width, height) =
    (f32::from(screen.cell_width), f32::from(screen.cell_height));
  let x0 = f32::from(cells.column.saturating_sub(left)) * width;
  let y0 = f32::from(cells.row) * height;
  let x1 = x0 + f32::from(cells.columns) * width;
  Bounds { x0, y0, x1, y1: y0 + f32::from(cells.rows) * height }
}

/// Fills cells below text with graphics.
pub fn backdrop<'a>(
  scene: &mut impl Draw<'a>,
  frame: Frame,
  cells: crate::split::Rect,
  color: impl Into<crate::theme::Paint>,
) {
  if frame.text {
    return;
  }
  let rect = cell_bounds(frame.screen, frame.left, cells);
  scene.fill(rect, color, Layer::BelowText, frame.stage());
}

/// Parts of `rect` outside `hole`.
fn around(rect: Bounds, hole: Bounds) -> impl Iterator<Item = Bounds> {
  let (top, bottom) = (hole.y0.max(rect.y0), hole.y1.min(rect.y1));
  [
    Bounds { y1: hole.y0.min(rect.y1), ..rect },
    Bounds { y0: hole.y1.max(rect.y0), ..rect },
    Bounds { y0: top, y1: bottom, x1: hole.x0.min(rect.x1), ..rect },
    Bounds { y0: top, y1: bottom, x0: hole.x1.max(rect.x0), ..rect },
  ]
  .into_iter()
  .filter(|part| part.x1 > part.x0 && part.y1 > part.y0)
}

pub fn outside(
  rect: Bounds,
  hole: Option<Bounds>,
) -> impl Iterator<Item = Bounds> {
  let whole = hole.is_none().then_some(rect);
  let parts = hole.into_iter().flat_map(move |hole| around(rect, hole));
  whole.into_iter().chain(parts)
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::scene::Scene;

  #[test]
  fn overflowing_origins_saturate() {
    let mut scene = Scene::new();
    move_to(&mut scene, At { column: 1, row: 1 }, usize::MAX, usize::MAX);
    assert_eq!(scene, [Op::Move { row: usize::MAX, column: usize::MAX }]);
  }

  #[test]
  fn scene_lines_keep_width_and_origin() {
    let mut scene = Scene::new();
    line(&mut scene, At { column: 3, row: 2 }, 2, "界e\u{301}\x07", 4);
    assert_eq!(
      scene,
      [
        Op::Move { row: 4, column: 3 },
        Op::Text(Text::new("界e\u{301}")),
        Op::Spaces(1)
      ]
    );
  }

  #[test]
  fn points_clamp_reported_pixels_to_the_cell() {
    let frame = Frame {
      at: At { column: 7, row: 3 },
      screen: Screen {
        origin: (42, 5),
        ..crate::testing::screen::cells(120, 40)
      },
      left: 20,
      text: false,
    };
    assert_eq!(frame.point(22, 4, None), (20.0, 72.0));
    for (x, y, want) in [
      (239, 127, (23.0, 79.0)),
      (0, 0, (16.0, 64.0)),
      (u16::MAX, u16::MAX, (23.0, 79.0)),
    ] {
      let pixel = crate::pointer::Pixel { x, y, cell: (22, 4) };
      assert_eq!(frame.point(22, 4, Some(pixel)), want);
    }
  }

  #[test]
  fn tints_skip_the_menu() {
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
    let b = |x0, y0, x1, y1| Bounds { x0, y0, x1, y1 };
    // (hole) -> parts
    let cases = [
      (b(20.0, 20.0, 30.0, 30.0), vec![rect]),
      (b(-5.0, -5.0, 15.0, 15.0), vec![]),
      (
        b(2.0, 2.0, 8.0, 8.0),
        vec![
          b(0.0, 0.0, 10.0, 2.0),
          b(0.0, 8.0, 10.0, 10.0),
          b(0.0, 2.0, 2.0, 8.0),
          b(8.0, 2.0, 10.0, 8.0),
        ],
      ),
      (b(5.0, -5.0, 15.0, 15.0), vec![b(0.0, 0.0, 5.0, 10.0)]),
    ];
    for (hole, want) in cases {
      let got: Vec<_> = outside(rect, Some(hole)).collect();
      let key = |b: &Bounds| (b.x0, b.y0, b.x1, b.y1);
      let got: Vec<_> = got.iter().map(key).collect();
      let want: Vec<_> = want.iter().map(key).collect();
      assert_eq!(got, want, "{hole:?}");
    }
  }
}
