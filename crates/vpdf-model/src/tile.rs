use crate::geometry::Crop;
use crate::text::PageId;
use std::ops::Range;

pub const TILE_WIDTH: i32 = 512;
pub const TILE_HEIGHT: i32 = 256;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct TileKey {
  pub page: PageId,
  pub scale: u32,
  pub rotation: u16,
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
  pub crop: Crop,
  pub recolor: crate::recolor::Recolor,
}

/// Tile owners require explicit conversion.
///
/// ```compile_fail
/// let owner: vpdf_model::tile::TileOwner = 1;
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TileOwner(u32);

impl TileOwner {
  pub fn new(value: u32) -> Self {
    Self(value)
  }
  pub fn get(self) -> u32 {
    self.0
  }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TileId {
  pub owner: TileOwner,
  pub key: TileKey,
}

/// Tiles of the page `whole` spans, over the `x` and `y` pixel ranges.
pub fn cover(
  whole: TileKey,
  x: Range<i32>,
  y: Range<i32>,
) -> impl Iterator<Item = TileKey> {
  y.step_by(TILE_HEIGHT as usize).flat_map(move |y| {
    x.clone().step_by(TILE_WIDTH as usize).map(move |x| TileKey {
      x,
      y,
      width: TILE_WIDTH.min(whole.width - x),
      height: TILE_HEIGHT.min(whole.height - y),
      ..whole
    })
  })
}
