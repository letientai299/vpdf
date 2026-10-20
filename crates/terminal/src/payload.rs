use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::{Compression, write::ZlibEncoder};
use std::cell::RefCell;
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
  thread_local! {
    static COMPRESSOR: RefCell<ZlibEncoder<Vec<u8>>> =
      RefCell::new(ZlibEncoder::new(Vec::new(), Compression::fast()));
  }
  COMPRESSOR.with_borrow_mut(|encoder| {
    encoder.write_all(bytes)?;
    Ok(STANDARD.encode(encoder.reset(Vec::new())?))
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reused_streams_match_fresh() -> Result<()> {
    for size in [0, 3, 17, 1 << 18, 1, 1 << 15] {
      let bytes: Vec<_> = (0..size).map(|i| (i * 173 % 251) as u8).collect();
      let mut fresh = ZlibEncoder::new(Vec::new(), Compression::fast());
      fresh.write_all(&bytes)?;
      assert_eq!(encode(&bytes)?, STANDARD.encode(fresh.finish()?));
    }
    Ok(())
  }

  #[test]
  fn compressor_threads_stay_independent() {
    let jobs: Vec<_> = (0..4)
      .map(|byte| {
        std::thread::spawn(move || {
          let input = vec![byte; 8192];
          for _ in 0..5 {
            let encoded = STANDARD.decode(encode(&input).unwrap()).unwrap();
            let mut decoded = Vec::new();
            std::io::Read::read_to_end(
              &mut flate2::read::ZlibDecoder::new(encoded.as_slice()),
              &mut decoded,
            )
            .unwrap();
            assert_eq!(decoded, input);
          }
        })
      })
      .collect();
    for job in jobs {
      job.join().unwrap();
    }
  }
}
