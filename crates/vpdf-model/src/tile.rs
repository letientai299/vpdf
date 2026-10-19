use crate::geometry::Crop;
use crate::text::PageId;

pub const TILE_WIDTH: i32 = 512;
pub const TILE_HEIGHT: i32 = 256;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TileKey {
  pub page: PageId,
  pub scale: u32,
  pub rotation: u16,
  pub x: i32,
  pub y: i32,
  pub width: i32,
  pub height: i32,
  pub crop: Crop,
}
