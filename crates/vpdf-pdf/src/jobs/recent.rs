use super::{Wake, WakingSender};
use crate::Result;
use crate::content_cache::{self, Kind, Snapshot};
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
  read_at(ask, password, content_cache::root())
}

fn read_at(
  ask: &FactsRequest,
  password: &str,
  root: Option<PathBuf>,
) -> Result<Facts> {
  let key = format!(
    "{}:{}",
    env!("VPDF_RECENT_CODE"),
    serde_json::to_string(&(ask.cursor, ask.rotation, ask.fit))?
  );
  let cache = root.and_then(|root| {
    Snapshot::open(root, &ask.path, password, Kind::Recent, &key, || true)
  });
  if let Some(facts) = cache.as_ref().and_then(recall) {
    return Ok(facts);
  }
  let facts = extract(ask, password)?;
  if let Some(cache) = cache {
    let size =
      facts.thumb.as_ref().map(|thumb| (thumb.width(), thumb.height()));
    let header = serde_json::to_vec(&(
      facts.pages,
      facts.bytes,
      &facts.metadata,
      &facts.section,
      size,
    ))?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&header);
    if let Some(thumb) = &facts.thumb {
      bytes.extend_from_slice(thumb.pixels());
    }
    let _ = cache.write(&bytes);
  }
  Ok(facts)
}

type Header =
  (usize, u64, Vec<(String, String)>, Option<String>, Option<(i32, i32)>);

fn recall(cache: &Snapshot) -> Option<Facts> {
  let bytes = cache.read()?;
  let length = u32::from_le_bytes(*bytes.first_chunk::<4>()?) as usize;
  let header = bytes.get(4..4usize.checked_add(length)?)?;
  let (pages, size, metadata, section, dimensions): Header =
    serde_json::from_slice(header).ok()?;
  if pages == 0 {
    return None;
  }
  let pixels = bytes.get(4 + length..)?;
  let thumb = match dimensions {
    Some((width, height)) => {
      Some(Thumbnail::new(width, height, pixels.to_vec())?)
    }
    None if pixels.is_empty() => None,
    None => return None,
  };
  Some(Facts { pages, bytes: size, metadata, section, thumb })
}

fn extract(ask: &FactsRequest, password: &str) -> Result<Facts> {
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::Position;

  fn request(path: PathBuf) -> FactsRequest {
    FactsRequest {
      path,
      cursor: Position::default(),
      rotation: 0,
      fit: Some((120, 160)),
    }
  }

  #[test]
  fn previews_roundtrip_and_invalidate() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    crate::fixtures::pdf(&source, &["first page", "second page"], "")?;
    let mut ask = request(source.clone());
    let baseline = read_at(&ask, "", None)?;
    let cold = read_at(&ask, "", Some(root.path().into()))?;
    let warm = read_at(&ask, "", Some(root.path().into()))?;
    assert_eq!(baseline.pages, cold.pages);
    assert_eq!(cold.metadata, warm.metadata);
    assert_eq!(baseline.thumb, cold.thumb);
    assert_eq!(cold.thumb, warm.thumb);
    ask.rotation = 90;
    let rotated = read_at(&ask, "", Some(root.path().into()))?;
    assert_ne!(rotated.thumb, warm.thumb);
    ask.fit = None;
    assert!(read_at(&ask, "", Some(root.path().into()))?.thumb.is_none());
    let files: Vec<_> = std::fs::read_dir(root.path())?
      .flatten()
      .map(|e| e.path())
      .filter(|p| p.extension().is_some_and(|e| e == "recent"))
      .collect();
    assert_eq!(files.len(), 3);
    for path in files {
      std::fs::write(path, b"damaged")?;
    }
    ask.fit = Some((120, 160));
    assert_eq!(
      read_at(&ask, "", Some(root.path().into()))?.thumb,
      rotated.thumb
    );
    let replacement = root.path().join("replacement.pdf");
    crate::fixtures::pdf(&replacement, &["replacement"], "")?;
    std::fs::rename(replacement, source)?;
    assert_eq!(read_at(&ask, "", Some(root.path().into()))?.pages, 1);
    Ok(())
  }

  #[test]
  fn preview_hit_skips_native() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"not a native PDF")?;
    let ask = request(source);
    let key = format!(
      "{}:{}",
      env!("VPDF_RECENT_CODE"),
      serde_json::to_string(&(ask.cursor, ask.rotation, ask.fit))?
    );
    let cache = Snapshot::open(
      root.path().into(),
      &ask.path,
      "",
      Kind::Recent,
      &key,
      || true,
    )
    .unwrap();
    let header = serde_json::to_vec(&(
      1usize,
      16u64,
      Vec::<(String, String)>::new(),
      Some("section"),
      Some((1i32, 1i32)),
    ))?;
    let mut bytes = (header.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(&header);
    bytes.extend_from_slice(&[1, 2, 3]);
    cache.write(&bytes)?;
    let facts = read_at(&ask, "", Some(root.path().into()))?;
    assert_eq!(facts.thumb.unwrap().pixels().as_slice(), &[1, 2, 3]);
    for variant in [
      FactsRequest {
        cursor: Position { page: PageId(1), ..Position::default() },
        ..request(ask.path.clone())
      },
      FactsRequest { fit: None, ..request(ask.path.clone()) },
    ] {
      assert!(read_at(&variant, "", Some(root.path().into())).is_err());
    }
    assert!(read_at(&ask, "secret", Some(root.path().into())).is_err());
    Ok(())
  }
}
