use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Instant;
use vpdf_pdf::document::Lists;
use vpdf_pdf::jobs::Wake;
use vpdf_pdf::jobs::render::{RenderWorker, Sink};
use vpdf_terminal::payload::{Encoder, Tile};

pub struct Raster {
  pub service: RenderWorker,
  pub results: mpsc::Receiver<Result<Tile, String>>,
}

impl Raster {
  pub fn new(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
    encoder: Encoder,
  ) -> Self {
    Self::start(path, password, lists, wake, encoder, false)
  }

  pub fn idle(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
    encoder: Encoder,
  ) -> Self {
    Self::start(path, password, lists, wake, encoder, true)
  }

  fn start(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
    encoder: Encoder,
    lazy: bool,
  ) -> Self {
    let (send, results) = mpsc::channel();
    let send = wake.sender(send);
    let sink = Sink::new(move |result| {
      let started = Instant::now();
      let result = result.and_then(|tile| {
        encoder
          .encode(&tile.pixels)
          .map(|payload| Tile {
            key: tile.key,
            payload,
            render_ms: tile.render_ms
              + started.elapsed().as_secs_f64() * 1000.0,
          })
          .map_err(|error| error.to_string())
      });
      send.send(result).is_ok()
    });
    let start = if lazy { RenderWorker::idle } else { RenderWorker::new };
    let service = start(path, password, lists, sink);
    Self { service, results }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::geometry::Crop;
  use vpdf_model::text::PageId;
  use vpdf_model::tile::TileKey;
  use vpdf_terminal::spool::Spool;

  #[test]
  fn encoding_failure_wakes_receiver() -> anyhow::Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["body"], "")?;
    let doc = vpdf_pdf::document::Document::open(file.path(), "")?;
    let directory = tempfile::tempdir()?;
    let spool = Arc::new(Spool::create(directory.path())?);
    let encoder = Encoder::new(Some(Arc::clone(&spool)));
    spool.close();
    let (wake, woken) = crate::fixtures::wake();
    let raster = Raster::new(file.path(), "", doc.lists(), &wake, encoder);
    let key = TileKey {
      page: PageId(0),
      scale: 1024,
      rotation: 0,
      x: 0,
      y: 0,
      width: 1,
      height: 1,
      crop: Crop::default(),
    };
    raster.service.request(&[key], 1);
    woken.recv_timeout(std::time::Duration::from_secs(10))?;
    let result = raster.results.try_recv()?;
    assert_eq!(result.err().as_deref(), Some("Spool closed"));
    Ok(())
  }
}
