use crate::Result;
use crate::document::Document;
use crate::layout::Layout;
use crate::text::PageId;

pub use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH, TileKey, cover};

pub use vpdf_terminal::payload::{Payload, Tile, encode};

pub fn visible(layout: &Layout, scroll: f32) -> Vec<TileKey> {
  let mut tiles = Vec::new();
  for page in layout.visible(scroll) {
    let data = &layout.pages[page];
    let (x, y) = layout.origin(PageId(page), scroll);
    let left = ((-x).max(0.0) as i32 / TILE_WIDTH) * TILE_WIDTH;
    let top = ((-y).max(0.0) as i32 / TILE_HEIGHT) * TILE_HEIGHT;
    let right = (layout.screen.width() - x).min(data.width) as i32;
    let bottom =
      (layout.screen.height() - y + TILE_HEIGHT as f32).min(data.height) as i32;
    let whole = TileKey {
      page: PageId(page),
      scale: data.scale,
      rotation: layout.rotation,
      x: 0,
      y: 0,
      width: data.width as i32,
      height: data.height as i32,
      crop: layout.crop_for(PageId(page)),
    };
    tiles.extend(cover(whole, left..right, top..bottom));
  }
  tiles
}

pub fn render(doc: &mut Document, key: TileKey) -> Result<Tile> {
  let started = std::time::Instant::now();
  let payload =
    vpdf_terminal::payload::Encoder::live().encode(&raster(doc, key)?)?;
  Ok(Tile { key, payload, render_ms: started.elapsed().as_secs_f64() * 1000.0 })
}

pub use vpdf_pdf::raster::{Capture, capture, cut, draw, raster, raster_area};
