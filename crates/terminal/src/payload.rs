use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::ZlibEncoder};
use std::io::Write;
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
  Spooled { spool: Arc<crate::spool::Spool>, slot: crate::spool::Slot },
}

#[derive(Clone, Default)]
pub struct Encoder {
  spool: Option<Arc<crate::spool::Spool>>,
}

impl Encoder {
  pub fn new(spool: Option<Arc<crate::spool::Spool>>) -> Self {
    Self { spool }
  }
  pub fn live() -> Self {
    Self::new(crate::spool::get())
  }
  pub fn spooled(&self) -> bool {
    self.spool.is_some()
  }

  pub fn encode(&self, pixels: &[u8]) -> Result<Payload> {
    Ok(match &self.spool {
      Some(spool) => Payload::Spooled {
        spool: Arc::clone(spool),
        slot: spool.write(pixels)?,
      },
      None => Payload::Encoded(encode(pixels)?.into()),
    })
  }
}

pub fn encode(bytes: &[u8]) -> Result<String> {
  let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
  encoder.write_all(bytes)?;
  Ok(STANDARD.encode(encoder.finish()?))
}
