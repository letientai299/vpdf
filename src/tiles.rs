use crate::Result;
use crate::document::Document;
use crate::layout::{Layout, page_matrix};
use crate::text::PageId;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::ZlibEncoder};
use mupdf::{Colorspace, Device, IRect, Pixmap, Rect};
use std::io::Write;

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
}

pub struct Tile {
  pub key: TileKey,
  pub encoded: String,
  pub render_ms: f64,
}

pub fn visible(layout: &Layout, scroll: f32) -> Vec<TileKey> {
  let mut tiles = Vec::new();
  for page in layout.visible(scroll) {
    let data = &layout.pages[page];
    let left = (layout.pan.max(0.0) as i32 / TILE_WIDTH) * TILE_WIDTH;
    let top = ((scroll - data.top).max(0.0) as i32 / TILE_HEIGHT) * TILE_HEIGHT;
    let right = (layout.pan + layout.screen.width()).min(data.width) as i32;
    let bottom =
      (scroll - data.top + layout.screen.height() + TILE_HEIGHT as f32)
        .min(data.height) as i32;
    for y in (top..bottom).step_by(TILE_HEIGHT as usize) {
      for x in (left..right).step_by(TILE_WIDTH as usize) {
        tiles.push(TileKey {
          page: PageId(page),
          scale: data.scale,
          rotation: layout.rotation,
          x,
          y,
          width: TILE_WIDTH.min(data.width as i32 - x),
          height: TILE_HEIGHT.min(data.height as i32 - y),
        });
      }
    }
  }
  tiles
}

pub fn raster(doc: &mut Document, key: TileKey) -> Result<Vec<u8>> {
  anyhow::ensure!(
    key.width > 0
      && key.width <= TILE_WIDTH
      && key.height > 0
      && key.height <= TILE_HEIGHT,
    "Invalid tile dimensions"
  );
  doc.validate(key.page)?;
  let bounds = doc.pages[key.page.0].bounds;
  let list = doc.display(key.page)?;
  let bbox = IRect::new(key.x, key.y, key.x + key.width, key.y + key.height);
  let mut pixels =
    Pixmap::new_with_rect(&Colorspace::device_rgb(), bbox, false)?;
  pixels.clear_with(255)?;
  let device = Device::from_pixmap(&pixels)?;
  let matrix = page_matrix(bounds, key.scale, key.rotation);
  list.run(
    &device,
    &matrix,
    Rect::new(
      key.x as f32,
      key.y as f32,
      (key.x + key.width) as f32,
      (key.y + key.height) as f32,
    ),
  )?;
  drop(device);
  Ok(pixels.samples().to_vec())
}

pub fn encode(bytes: &[u8]) -> Result<String> {
  let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
  encoder.write_all(bytes)?;
  Ok(STANDARD.encode(encoder.finish()?))
}

pub fn render(doc: &mut Document, key: TileKey) -> Result<Tile> {
  let started = std::time::Instant::now();
  let encoded = encode(&raster(doc, key)?)?;
  Ok(Tile { key, encoded, render_ms: started.elapsed().as_secs_f64() * 1000.0 })
}
