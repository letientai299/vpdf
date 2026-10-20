use super::{Wake, WakingSender};
use crate::Result;
use crate::document::Handle;
use mupdf::IRect;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};
use vpdf_model::cache::Cache;
use vpdf_model::document::{Facts, FactsRequest, LoadedFacts, Thumbnail};
use vpdf_model::geometry::page_matrix;
use vpdf_model::text::PageId;

pub struct Worker {
  requests: mpsc::Sender<FactsRequest>,
  results: mpsc::Receiver<(PathBuf, LoadedFacts)>,
}

impl Worker {
  pub fn new(password: &str, wake: &Wake) -> Self {
    let (requests, inbox) = mpsc::channel();
    let (send, results) = mpsc::channel();
    let send = wake.sender(send);
    let password = password.to_owned();
    std::thread::spawn(move || work(&password, &inbox, &send));
    Self { requests, results }
  }

  /// Queues a load; the worker reads only the newest.
  pub fn request(&self, request: FactsRequest) {
    // A stopped worker drops the request, as its results stop too.
    let _ = self.requests.send(request);
  }

  /// Loads finished since the last call.
  pub fn updates(&self) -> mpsc::TryIter<'_, (PathBuf, LoadedFacts)> {
    self.results.try_iter()
  }
}

/// Loads each newest ask until the shelf drops.
fn work(
  password: &str,
  inbox: &mpsc::Receiver<FactsRequest>,
  send: &WakingSender<mpsc::Sender<(PathBuf, LoadedFacts)>>,
) {
  while let Ok(mut ask) = inbox.recv() {
    // A selection moved past is not worth reading.
    while let Ok(newer) = inbox.try_recv() {
      ask = newer;
    }
    let facts = read(&ask, password).map_err(|error| error.to_string());
    let card = LoadedFacts { fit: ask.fit, facts };
    if send.send((ask.path, card)).is_err() {
      return;
    }
  }
}

fn read(ask: &FactsRequest, password: &str) -> Result<Facts> {
  let handle = Handle::open(&ask.path, password)?;
  let thumb =
    ask.fit.map(|fit| thumbnail(&handle, fit, ask.rotation)).transpose()?;
  let outline = handle.outline().unwrap_or_default();
  let cursor = ask.cursor;
  let section = if outline.is_empty() {
    None
  } else {
    let y = handle
      .text(cursor.page)
      .ok()
      .and_then(|text| text.glyph(cursor).map(|g| g.bounds.y1))
      .unwrap_or(f32::MAX);
    vpdf_model::document::section(&outline, cursor.page, y)
      .map(|index| outline[index].title.clone())
  };
  Ok(Facts {
    pages: handle.pages,
    bytes: std::fs::metadata(&ask.path)?.len(),
    metadata: handle.metadata(),
    section,
    thumb,
  })
}

/// The first page at the largest scale that fits `fit` pixels.
fn thumbnail(
  handle: &Handle,
  (width, height): (u32, u32),
  rotation: u16,
) -> Result<Thumbnail> {
  let page = PageId(0);
  let bounds = handle.bounds(page)?;
  let (w, h) = if rotation % 180 == 0 {
    (bounds.width(), bounds.height())
  } else {
    (bounds.height(), bounds.width())
  };
  anyhow::ensure!(w >= 1.0 && h >= 1.0, "Empty first page");
  let fit = (width as f32 / w).min(height as f32 / h);
  // `page_matrix` takes 1024ths.
  let scale = (fit * 1024.0) as u32;
  let size = |side: f32| (side * scale as f32 / 1024.0) as i32;
  let (pixels_wide, pixels_high) = (size(w), size(h));
  anyhow::ensure!(pixels_wide > 0 && pixels_high > 0, "Thumbnail too small");
  // A list read once needs no cache.
  let lists = Mutex::new(Cache::new(0));
  let list = handle.display(page, &lists)?;
  let matrix = page_matrix(bounds, scale, rotation);
  let area = IRect::new(0, 0, pixels_wide, pixels_high);
  let pixmap = crate::raster::draw(&list, &matrix, area)?;
  Thumbnail::new(pixels_wide, pixels_high, pixmap.samples().to_vec())
    .ok_or_else(|| anyhow::anyhow!("Invalid thumbnail pixels"))
}
