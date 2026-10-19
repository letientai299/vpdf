use crate::Result;
use crate::document::Document;

pub use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH, TileKey, cover};

pub use vpdf_terminal::payload::{Payload, Tile, encode};

pub fn render(doc: &mut Document, key: TileKey) -> Result<Tile> {
  let started = std::time::Instant::now();
  let payload =
    vpdf_terminal::payload::Encoder::live().encode(&raster(doc, key)?)?;
  Ok(Tile { key, payload, render_ms: started.elapsed().as_secs_f64() * 1000.0 })
}

pub use vpdf_pdf::raster::{Capture, capture, cut, draw, raster, raster_area};
