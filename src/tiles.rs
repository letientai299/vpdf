use crate::Result;
use crate::document::Document;
use crate::layout::{Crop, Layout, page_matrix};
use crate::text::PageId;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::ZlibEncoder};
use mupdf::{Colorspace, Device, DisplayList, IRect, Matrix, Pixmap, Rect};
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
  pub crop: Crop,
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
          crop: layout.crop,
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
  let area = IRect::new(key.x, key.y, key.x + key.width, key.y + key.height);
  let pixels = raster_area(doc, &key, area)?;
  Ok(pixels.samples().to_vec())
}

/// RGB pixels of `area`, in the page space `key` describes.
pub fn raster_area(
  doc: &mut Document,
  key: &TileKey,
  area: IRect,
) -> Result<Pixmap> {
  doc.validate(key.page)?;
  let bounds = key.crop.apply(doc.pages[key.page.0].bounds);
  let list = doc.display(key.page)?;
  draw(&list, &page_matrix(bounds, key.scale, key.rotation), area)
}

/// RGB pixels of `area` with `list` drawn through `matrix`.
pub fn draw(
  list: &DisplayList,
  matrix: &Matrix,
  area: IRect,
) -> Result<Pixmap> {
  let mut pixels =
    Pixmap::new_with_rect(&Colorspace::device_rgb(), area, false)?;
  pixels.clear_with(255)?;
  let device = Device::from_pixmap(&pixels)?;
  list.run(
    &device,
    matrix,
    Rect::new(area.x0 as f32, area.y0 as f32, area.x1 as f32, area.y1 as f32),
  )?;
  drop(device);
  Ok(pixels)
}

/// Packed RGB rows of the tile `key` cut from `band`.
pub fn cut(band: &Pixmap, key: &TileKey) -> Result<Vec<u8>> {
  let stride = usize::try_from(band.stride())?;
  let left = usize::try_from(key.x - band.x())? * 3;
  let top = usize::try_from(key.y - band.y())?;
  let width = usize::try_from(key.width)? * 3;
  let height = usize::try_from(key.height)?;
  let mut pixels = Vec::with_capacity(width * height);
  for row in band.samples().chunks(stride).skip(top).take(height) {
    pixels.extend_from_slice(&row[left..left + width]);
  }
  anyhow::ensure!(pixels.len() == width * height, "Tile outside band");
  Ok(pixels)
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
