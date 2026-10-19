use crate::Result;
use vpdf_model::tile::TileKey;
use vpdf_pdf::document::Document;
use vpdf_pdf::raster::raster;
use vpdf_terminal::payload::Tile;

pub fn render(doc: &mut Document, key: TileKey) -> Result<Tile> {
  let started = std::time::Instant::now();
  let payload =
    vpdf_terminal::payload::Encoder::live().encode(&raster(doc, key)?)?;
  Ok(Tile { key, payload, render_ms: started.elapsed().as_secs_f64() * 1000.0 })
}
