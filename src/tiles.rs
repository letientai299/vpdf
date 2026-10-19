use crate::Result;
use crate::document::Document;
use crate::layout::Layout;
use crate::text::PageId;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;
use std::ops::Range;
use std::sync::Arc;

pub use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH, TileKey};

pub struct Tile {
  pub key: TileKey,
  pub payload: Payload,
  pub render_ms: f64,
}

/// A tile's pixels as the terminal receives them.
#[derive(Clone, Debug)]
pub enum Payload {
  /// zlib-compressed RGB in base64, sent through the terminal; shared
  /// with the kept-tile cache without a copy.
  Encoded(Arc<str>),
  /// RGB in a spool slot the terminal reads itself.
  Spooled(crate::spool::Slot),
}

impl Payload {
  /// Spools `pixels` when the terminal reads the spool, else encodes them.
  pub fn new(pixels: &[u8]) -> Result<Self> {
    Ok(match crate::spool::get() {
      Some(spool) => Self::Spooled(spool.write(pixels)?),
      None => Self::Encoded(encode(pixels)?.into()),
    })
  }
}

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

pub fn encode(bytes: &[u8]) -> Result<String> {
  let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
  encoder.write_all(bytes)?;
  Ok(STANDARD.encode(encoder.finish()?))
}

pub fn render(doc: &mut Document, key: TileKey) -> Result<Tile> {
  let started = std::time::Instant::now();
  let payload = Payload::new(&raster(doc, key)?)?;
  Ok(Tile { key, payload, render_ms: started.elapsed().as_secs_f64() * 1000.0 })
}

pub use vpdf_pdf::raster::{Capture, capture, cut, draw, raster, raster_area};
