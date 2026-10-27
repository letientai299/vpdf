//! Frontend raster delivery.

use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::Instant;
use vpdf_model::tile::TileKey;
use vpdf_pdf::document::{Document, Lists};
use vpdf_pdf::jobs::Wake;
use vpdf_pdf::jobs::render::{RenderWorker, Sink};
use vpdf_pdf::raster::raster;
use vpdf_terminal::payload::{Encoder, Tile};

#[derive(Clone, Default)]
pub(crate) enum Frontend {
  #[default]
  Terminal,
  Native(Wake),
}

impl Frontend {
  pub fn wake(&self) -> Wake {
    match self {
      Self::Terminal => Wake::new(vpdf_terminal::input::wake),
      Self::Native(wake) => wake.clone(),
    }
  }

  pub fn raster(
    &self,
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
    lazy: bool,
  ) -> Raster {
    match self {
      Self::Terminal => {
        Raster::start(path, password, lists, wake, Encoder::live(), lazy)
      }
      Self::Native(_) => Raster::raw(path, password, lists, wake, lazy),
    }
  }
}

pub struct Raster {
  pub service: RenderWorker,
  pub results: mpsc::Receiver<Result<Tile, String>>,
  pub(crate) raw: Option<Raw>,
}

pub(crate) struct Raw {
  pub results: mpsc::Receiver<vpdf_pdf::jobs::render::Completion>,
  pub overflow: Arc<std::sync::atomic::AtomicBool>,
}

struct Delivery {
  send: mpsc::SyncSender<vpdf_pdf::jobs::render::Completion>,
  overflow: Arc<std::sync::atomic::AtomicBool>,
  wake: Wake,
}

impl Delivery {
  fn send(&self, result: vpdf_pdf::jobs::render::Completion) -> bool {
    use std::sync::atomic::Ordering;
    match self.send.try_send(result) {
      Ok(()) => {}
      Err(mpsc::TrySendError::Full(_)) => {
        self.overflow.store(true, Ordering::Release);
      }
      Err(mpsc::TrySendError::Disconnected(_)) => return false,
    }
    self.wake.wake();
    true
  }
}

impl Raster {
  pub(crate) fn raw(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
    lazy: bool,
  ) -> Self {
    let (send, receive) = mpsc::sync_channel(128);
    let overflow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let delivery =
      Delivery { send, overflow: Arc::clone(&overflow), wake: wake.clone() };
    let sink = Sink::new(move |result| delivery.send(result));
    let (_, results) = mpsc::channel();
    Self {
      service: (if lazy { RenderWorker::idle } else { RenderWorker::new })(
        path, password, lists, sink,
      ),
      results,
      raw: Some(Raw { results: receive, overflow }),
    }
  }

  pub fn start(
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
        encode(&encoder, tile.key, &tile.pixels, tile.render_ms, started)
          .map_err(|error| error.to_string())
      });
      send.send(result).is_ok()
    });
    let start = if lazy { RenderWorker::idle } else { RenderWorker::new };
    let service = start(path, password, lists, sink);
    Self { service, results, raw: None }
  }
}

/// A terminal tile of raster pixels; its time adds encoding since
/// `started` to `render_ms`.
pub fn encode(
  encoder: &Encoder,
  key: TileKey,
  pixels: &[u8],
  render_ms: f64,
  started: Instant,
) -> anyhow::Result<Tile> {
  let payload = encoder.encode(pixels)?;
  let render_ms = render_ms + started.elapsed().as_secs_f64() * 1000.0;
  Ok(Tile { key, payload, render_ms })
}

pub fn render(doc: &mut Document, key: TileKey) -> anyhow::Result<Tile> {
  let started = Instant::now();
  let mut pixels = raster(doc, key)?;
  key.recolor.apply(&mut pixels);
  encode(&Encoder::live(), key, &pixels, 0.0, started)
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_terminal::spool::Spool;

  #[test]
  fn raw_delivery_bounds_and_wakes() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (send, receive) =
      mpsc::sync_channel::<vpdf_pdf::jobs::render::Completion>(1);
    let receive = Arc::new(std::sync::Mutex::new(receive));
    let observed = Arc::clone(&receive);
    let wake = Wake::new(move || {
      assert_eq!(
        observed.lock().unwrap().try_recv().unwrap().err().as_deref(),
        Some("queued")
      );
    });
    let overflow = Arc::new(AtomicBool::new(false));
    let mut delivery = Delivery { send, overflow: Arc::clone(&overflow), wake };
    assert!(delivery.send(Err("queued".into())));
    let (wake, woken) = crate::fixtures::wake();
    delivery.wake = wake;
    assert!(delivery.send(Err("kept".into())));
    assert!(delivery.send(Err("dropped".into())));
    assert!(overflow.swap(false, Ordering::AcqRel));
    assert_eq!(woken.try_iter().count(), 2);
    assert_eq!(receive.lock().unwrap().try_iter().count(), 1);
    drop(receive);
    assert!(!delivery.send(Err("closed".into())));
  }

  #[test]
  fn encoding_failure_wakes_receiver() -> anyhow::Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["body"], "")?;
    let doc = Document::open(file.path(), "")?;
    let directory = tempfile::tempdir()?;
    let spool = Arc::new(Spool::create(directory.path())?);
    let encoder = Encoder::new(Some(Arc::clone(&spool)));
    spool.close();
    let (wake, woken) = crate::fixtures::wake();
    let raster =
      Raster::start(file.path(), "", doc.lists(), &wake, encoder, false);
    let key =
      TileKey { scale: 1024, width: 1, height: 1, ..TileKey::default() };
    raster.service.request(&[key], 1);
    woken.recv_timeout(std::time::Duration::from_secs(10))?;
    let result = raster.results.try_recv()?;
    assert_eq!(result.err().as_deref(), Some("Spool closed"));
    Ok(())
  }
}
