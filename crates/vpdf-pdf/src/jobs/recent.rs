use super::{Wake, WakingSender};
use crate::Result;
use crate::content_cache::{self, Kind, Record};
use crate::document::Handle;
use mupdf::IRect;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use vpdf_model::cache::Cache;
use vpdf_model::document::{
  CountsRequest, Facts, FactsRequest, LoadedFacts, RECENT_COUNTS, Thumbnail,
};
use vpdf_model::geometry::page_matrix;
use vpdf_model::text::PageId;

type CountResult = std::result::Result<usize, String>;
type CountReply = (u64, PathBuf, CountResult, bool);

#[derive(Default)]
struct Requests {
  preview: Option<FactsRequest>,
  counts: VecDeque<(u64, PathBuf, Option<usize>)>,
  visible: Vec<(u64, PathBuf, Option<usize>)>,
  stopped: bool,
}

pub struct Worker {
  requests: Arc<(Mutex<Requests>, Condvar)>,
  results: mpsc::Receiver<(PathBuf, LoadedFacts)>,
  counts: mpsc::Receiver<CountReply>,
}

impl Worker {
  pub fn new(password: &str, wake: &Wake) -> Self {
    let requests = Arc::new((Mutex::new(Requests::default()), Condvar::new()));
    let (send, results) = mpsc::channel();
    let send = wake.sender(send);
    let (count_send, counts) = mpsc::sync_channel(RECENT_COUNTS);
    let count_send = wake.sender(count_send);
    let password = password.to_owned();
    let pending = requests.clone();
    std::thread::spawn(move || work(&password, &pending, &send, &count_send));
    Self { requests, results, counts }
  }

  pub fn request(&self, request: FactsRequest) {
    let (lock, ready) = &*self.requests;
    lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner).preview =
      Some(request);
    ready.notify_one();
  }

  /// Replaces obsolete visible work; previews retain priority.
  pub fn visible(&self, request: CountsRequest) {
    let CountsRequest { generation, files: paths } = request;
    let (lock, ready) = &*self.requests;
    let mut requests =
      lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    requests.counts.clear();
    for (path, known) in paths {
      if requests.counts.len() == RECENT_COUNTS {
        break;
      }
      if !requests.counts.iter().any(|(_, queued, _)| *queued == path) {
        requests.counts.push_back((generation, path, known));
      }
    }
    requests.visible = requests.counts.iter().cloned().collect();
    ready.notify_one();
  }

  pub fn updates(&self) -> mpsc::TryIter<'_, (PathBuf, LoadedFacts)> {
    self.results.try_iter()
  }

  pub fn counts(&self) -> mpsc::TryIter<'_, CountReply> {
    self.counts.try_iter()
  }
}

impl Drop for Worker {
  fn drop(&mut self) {
    let (lock, ready) = &*self.requests;
    lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner).stopped =
      true;
    ready.notify_one();
  }
}

fn work(
  password: &str,
  requests: &(Mutex<Requests>, Condvar),
  send: &WakingSender<mpsc::Sender<(PathBuf, LoadedFacts)>>,
  counts: &WakingSender<mpsc::SyncSender<CountReply>>,
) {
  let mut seen = Cache::new(RECENT_COUNTS * 2);
  loop {
    let (lock, ready) = requests;
    let mut pending =
      lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    while !pending.stopped
      && pending.preview.is_none()
      && pending.counts.is_empty()
    {
      let (next, timeout) = ready
        .wait_timeout(pending, std::time::Duration::from_secs(1))
        .unwrap_or_else(std::sync::PoisonError::into_inner);
      pending = next;
      if timeout.timed_out() {
        pending.counts = pending.visible.iter().cloned().collect();
      }
    }
    if pending.stopped {
      return;
    }
    if let Some(ask) = pending.preview.take() {
      drop(pending);
      let facts = read(&ask, password).map_err(|error| error.to_string());
      let card =
        LoadedFacts { generation: ask.generation, fit: ask.fit, facts };
      if send.send((ask.path, card)).is_err() {
        return;
      }
      continue;
    }
    let Some((generation, path, known)) = pending.counts.pop_front() else {
      continue;
    };
    drop(pending);
    let version = content_cache::Version::read(&path);
    let previous = seen.get(&path).copied();
    if previous == Some((generation, version)) {
      continue;
    }
    seen.insert(path.clone(), (generation, version), 1);
    let changed = previous.is_some_and(|(_, before)| before != version);
    if previous.is_none() && known.is_some() {
      continue;
    }
    let pages = page_count(&path, password, content_cache::root())
      .map_err(|error| error.to_string());
    if content_cache::Version::read(&path) != version {
      seen.remove(&path);
      continue;
    }
    if counts.send((generation, path, pages, changed)).is_err() {
      return;
    }
  }
}

fn page_count(
  path: &std::path::Path,
  password: &str,
  root: Option<PathBuf>,
) -> Result<usize> {
  let cache = root.and_then(|root| {
    Record::open(root, path, password, Kind::Recent, "page-count-v1", || true)
  });
  if let Some(pages) = cache
    .as_ref()
    .and_then(Record::read)
    .and_then(|bytes| serde_json::from_slice::<usize>(&bytes).ok())
  {
    return Ok(pages);
  }
  let pages = Handle::open(path, password)?.pages;
  if let Some(cache) = cache {
    let _ = cache.write(&serde_json::to_vec(&pages)?);
  }
  Ok(pages)
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
    Record::open(root, &ask.path, password, Kind::Recent, &key, || true)
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

fn recall(cache: &Record) -> Option<Facts> {
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
      generation: 0,
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
    let cache = Record::open(
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

#[cfg(test)]
mod count_tests {
  use super::*;

  #[test]
  fn count_cache_tracks_source_changes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    crate::fixtures::pdf(&source, &["first", "second"], "")?;
    assert_eq!(page_count(&source, "", Some(root.path().into()))?, 2);
    assert_eq!(page_count(&source, "", Some(root.path().into()))?, 2);
    let replacement = root.path().join("replacement.pdf");
    crate::fixtures::pdf(&replacement, &["replacement"], "")?;
    std::fs::rename(replacement, &source)?;
    assert_eq!(page_count(&source, "", Some(root.path().into()))?, 1);
    Ok(())
  }

  #[test]
  fn visible_queue_bounds_deduplicates_and_replaces() {
    let requests = Arc::new((Mutex::new(Requests::default()), Condvar::new()));
    let (_, results) = mpsc::channel();
    let (_, counts) = mpsc::channel();
    let worker = Worker { requests: requests.clone(), results, counts };
    worker.visible(CountsRequest {
      generation: 1,
      files: (0..100)
        .flat_map(|index| {
          vec![(PathBuf::from(format!("{index}.pdf")), None); 2]
        })
        .collect(),
    });
    {
      let pending = requests.0.lock().unwrap();
      assert_eq!(pending.counts.len(), RECENT_COUNTS);
      assert_eq!(pending.counts.front().unwrap().0, 1);
    }
    worker.visible(CountsRequest {
      generation: 2,
      files: vec![("current.pdf".into(), None)],
    });
    assert_eq!(
      requests.0.lock().unwrap().counts.iter().cloned().collect::<Vec<_>>(),
      vec![(2, "current.pdf".into(), None)]
    );
  }
}

#[cfg(test)]
mod watcher_tests {
  use super::*;
  use std::time::{Duration, Instant};

  #[test]
  fn visible_identity_changes_refresh_counts() -> Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    crate::fixtures::pdf(&source, &["first", "second"], "")?;
    let worker = Worker::new("", &Wake::none());
    worker.visible(CountsRequest {
      generation: 3,
      files: vec![(source.clone(), None)],
    });
    let receive = || -> Result<CountReply> {
      let deadline = Instant::now() + Duration::from_secs(5);
      loop {
        if let Some(reply) = worker.counts().next() {
          return Ok(reply);
        }
        anyhow::ensure!(Instant::now() < deadline, "Count never arrived");
        std::thread::sleep(Duration::from_millis(5));
      }
    };
    let (generation, path, pages, changed) = receive()?;
    assert_eq!(
      (generation, path, pages, changed),
      (3, source.clone(), Ok(2), false)
    );
    let replacement = root.path().join("replacement.pdf");
    crate::fixtures::pdf(&replacement, &["replacement"], "")?;
    std::fs::rename(replacement, &source)?;
    let (_, path, pages, changed) = receive()?;
    assert_eq!((path, pages, changed), (source, Ok(1), true));
    Ok(())
  }
}
