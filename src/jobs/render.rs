//! Tile rendering off the input thread.
//!
//! A coordinator per document owns its `MuPDF` document, builds display
//! lists, and splits each request into bands: runs of adjacent tiles in
//! one tile row. A process-wide pool draws each band in one pass, cuts
//! its tiles, and encodes them. Display lists are immutable and `MuPDF`
//! renders them from several threads, each with its own context, while
//! only the coordinator touches the document; a list may outlive it, as
//! Type3 glyphs draw from lists compiled when their font loads. Pages
//! with large images render whole on the pool, which sends every tile of
//! the page, so later bands and scrolls reuse the one draw.
use crate::Result;
use crate::document::{Handle, Lists};
use crate::jobs::{Wake, WakingSender};
use crate::layout::{Crop, page_matrix, transform};
use crate::text::PageId;
use crate::tiles::{self, TILE_WIDTH, Tile, TileKey};
use mupdf::{DisplayList, IRect, Matrix};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

/// Largest image page rendered whole, in pixels; about 24 MiB of RGB.
const WHOLE: i64 = 8 << 20;
/// Pages drawn after which an idle coordinator reopens its document.
const RELOAD: usize = 64;
/// Idle time before that reopen.
const IDLE: Duration = Duration::from_secs(2);

type Sender = WakingSender<mpsc::Sender<std::result::Result<Tile, String>>>;

use vpdf_pdf::scheduling::Urgent;
pub use vpdf_pdf::scheduling::yield_to_views;

#[derive(Default)]
struct Queue {
  /// Latest request in priority order; the coordinator drains it.
  tiles: Vec<TileKey>,
  pages: Vec<PageId>,
  /// Leading tiles of `tiles` that are on screen; the rest prefetch.
  urgent: usize,
  /// Every tile of the latest request.
  wanted: HashSet<TileKey>,
  /// Tiles handed to the pool and not yet sent.
  dispatched: HashSet<TileKey>,
  fresh: bool,
  stop: bool,
}

#[derive(Default)]
struct Shared {
  queue: Mutex<Queue>,
  wake: Condvar,
  /// Whole pages drawn, which tests count.
  #[cfg(test)]
  wholes: AtomicUsize,
}

pub struct RenderWorker {
  shared: Arc<Shared>,
  pub results: mpsc::Receiver<std::result::Result<Tile, String>>,
}

impl RenderWorker {
  /// Opens its own handle of `path`, sharing display lists through
  /// `lists`.
  pub fn new(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
  ) -> Self {
    Self::start(path, password, lists, wake, false)
  }

  /// A worker for a hidden pane: opens its document on the first request.
  pub fn idle(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
  ) -> Self {
    Self::start(path, password, lists, wake, true)
  }

  fn start(
    path: &Path,
    password: &str,
    lists: Arc<Lists>,
    wake: &Wake,
    lazy: bool,
  ) -> Self {
    let shared = Arc::new(Shared::default());
    let work = Arc::clone(&shared);
    let (send, results) = mpsc::channel();
    let send = wake.sender(send);
    let path = path.to_owned();
    let password = password.to_owned();
    thread::spawn(move || {
      if lazy && !requested(&work) {
        return;
      }
      match Handle::open(&path, &password) {
        Ok(doc) => {
          Coordinator::new(doc, lists, (path, password), work, send).run();
        }
        Err(error) => drop(send.send(Err(error.to_string()))),
      }
    });
    Self { shared, results }
  }

  /// Replaces queued work with `tiles`, most urgent first. Tiles after
  /// the first `urgent` prefetch: they draw one band at a time while the
  /// pool is otherwise idle, so a new viewport waits for one band at most.
  pub fn request(&self, tiles: &[TileKey], urgent: usize) {
    self.prepare(tiles, urgent, &[]);
  }

  pub(crate) fn prepare(
    &self,
    tiles: &[TileKey],
    urgent: usize,
    pages: &[PageId],
  ) {
    if let Ok(mut queue) = self.shared.queue.lock() {
      queue.pages =
        pages.iter().take(crate::prefetch::PAGES).copied().collect();
      queue.tiles = tiles.to_vec();
      queue.urgent = urgent.min(tiles.len());
      queue.wanted = tiles.iter().copied().collect();
      queue.fresh = true;
      self.shared.wake.notify_all();
    }
  }
}

impl Drop for RenderWorker {
  fn drop(&mut self) {
    if let Ok(mut queue) = self.shared.queue.lock() {
      queue.stop = true;
      queue.wanted.clear();
      self.shared.wake.notify_all();
    }
  }
}

/// Waits for a request; false once the worker stops first.
fn requested(shared: &Shared) -> bool {
  let Ok(queue) = shared.queue.lock() else { return false };
  shared
    .wake
    .wait_while(queue, |queue| !queue.fresh && !queue.stop)
    .is_ok_and(|queue| !queue.stop)
}

/// How a page renders at one scale.
#[derive(Clone, Copy)]
enum Kind {
  Bands,
  /// Large images scale differently by clip origin; draw whole.
  Whole,
}

type PageKey = (PageId, u32, u16, Crop);

struct Pending {
  urgent: Vec<TileKey>,
  ahead: Vec<TileKey>,
  pages: Vec<PageId>,
}

struct Coordinator {
  doc: Handle,
  /// Display lists shared with the pane's document.
  lists: Arc<Lists>,
  /// Path and password to reopen `doc`.
  source: (PathBuf, String),
  shared: Arc<Shared>,
  send: Sender,
  kinds: HashMap<PageKey, Kind>,
  /// Whole pages the current request has drawn; their tiles are sent.
  wholes: HashSet<PageKey>,
  /// Pages drawn since `doc` opened.
  touched: HashSet<PageId>,
}

impl Coordinator {
  fn new(
    doc: Handle,
    lists: Arc<Lists>,
    source: (PathBuf, String),
    shared: Arc<Shared>,
    send: Sender,
  ) -> Self {
    Self {
      doc,
      lists,
      source,
      shared,
      send,
      kinds: HashMap::new(),
      wholes: HashSet::new(),
      touched: HashSet::new(),
    }
  }

  /// Reopens the document, so `MuPDF` releases the objects and resources
  /// of every page drawn since: browsing 320 thumbnails of a 1370-page
  /// book held about 200 MB. Cached display lists keep their own.
  fn refresh(&mut self) {
    if let Ok(doc) = Handle::open(&self.source.0, &self.source.1) {
      self.doc = doc;
    }
    self.touched.clear();
  }

  fn run(mut self) {
    while let Some(Pending { urgent, ahead, pages }) = self.next() {
      self.wholes.clear();
      let drawing = Urgent::new();
      for band in bands(&urgent) {
        if !self.current() {
          break;
        }
        self.start(band, true);
      }
      drop(drawing);
      for band in bands(&ahead) {
        if !self.idle() {
          break;
        }
        self.start(band, false);
      }
      for page in pages {
        if !self.idle() || !yield_to_views(|| !self.current()) {
          break;
        }
        self.touched.insert(page);
        // Speculative failures must not interrupt reading.
        let _ = self.doc.display(page, &self.lists);
      }
    }
  }

  fn start(&mut self, mut band: Vec<TileKey>, urgent: bool) {
    // A whole page drawn for this request already sent its other bands.
    band.retain(|key| !self.wholes.contains(&page_of(key)));
    if let Ok(queue) = self.shared.queue.lock() {
      band.retain(|key| !queue.dispatched.contains(key));
    }
    if band.is_empty() {
      return;
    }
    match self.job(band) {
      Ok(job) => self.dispatch(job, urgent),
      Err(error) => drop(self.send.send(Err(error.to_string()))),
    }
  }

  /// Whether no newer request has arrived.
  fn current(&self) -> bool {
    self.shared.queue.lock().is_ok_and(|q| !q.fresh && !q.stop)
  }

  /// Waits until this document has no band in flight; false when a newer
  /// request arrives first.
  fn idle(&self) -> bool {
    let Ok(mut queue) = self.shared.queue.lock() else { return false };
    while !queue.dispatched.is_empty() && !queue.fresh && !queue.stop {
      queue = match self.shared.wake.wait(queue) {
        Ok(queue) => queue,
        Err(_) => return false,
      };
    }
    !queue.fresh && !queue.stop
  }

  /// Waits for a request; drops queued bands it no longer wants and
  /// returns its on-screen and prefetch tiles not already in flight.
  fn next(&mut self) -> Option<Pending> {
    let waiting = |queue: &mut Queue| !queue.fresh && !queue.stop;
    let mut queue = loop {
      let queue = self.shared.queue.lock().ok()?;
      if self.touched.len() < RELOAD {
        break self.shared.wake.wait_while(queue, waiting).ok()?;
      }
      let (queue, wait) =
        self.shared.wake.wait_timeout_while(queue, IDLE, waiting).ok()?;
      if !wait.timed_out() {
        break queue;
      }
      drop(queue);
      self.refresh();
    };
    queue.fresh = false;
    let dropped = self.unqueue(&mut queue);
    if queue.stop {
      return None;
    }
    let mut urgent = std::mem::take(&mut queue.tiles);
    let mut ahead = urgent.split_off(queue.urgent);
    urgent.retain(|key| !queue.dispatched.contains(key));
    ahead.retain(|key| !queue.dispatched.contains(key));
    let pages = std::mem::take(&mut queue.pages);
    drop(queue);
    drop(dropped);
    Some(Pending { urgent, ahead, pages })
  }

  /// Takes this coordinator's queued bands with no tile `queue` still
  /// wants, for the caller to drop outside the locks.
  fn unqueue(&self, queue: &mut Queue) -> Vec<Task> {
    let Ok(mut tasks) = pool().tasks.lock() else { return Vec::new() };
    let (stale, kept): (Vec<_>, Vec<_>) =
      std::mem::take(&mut *tasks).into_iter().partition(|task| {
        Arc::ptr_eq(&task.shared, &self.shared)
          && !task.job.tiles.iter().any(|key| queue.wanted.contains(key))
      });
    *tasks = kept.into();
    for key in stale.iter().flat_map(|task| &task.job.tiles) {
      queue.dispatched.remove(key);
    }
    stale
  }

  fn job(&mut self, tiles: Vec<TileKey>) -> Result<Job> {
    let key = tiles[0];
    let bounds = key.crop.apply(self.doc.bounds(key.page)?);
    self.touched.insert(key.page);
    let page = page_of(&key);
    let matrix = page_matrix(bounds, key.scale, key.rotation);
    let rect = transform(bounds, &matrix);
    let area = IRect::new(0, 0, rect.x1.ceil() as i32, rect.y1.ceil() as i32);
    let list = self.doc.display(key.page, &self.lists)?;
    let kind = match self.kinds.get(&page) {
      Some(kind) => *kind,
      None => {
        let kind = classify(&list, &matrix, area)?;
        if self.kinds.len() >= 4096 {
          self.kinds.clear();
        }
        self.kinds.insert(page, kind);
        kind
      }
    };
    Ok(match kind {
      Kind::Bands => Job { tiles, work: Work::Band { list, matrix } },
      Kind::Whole => {
        self.wholes.insert(page);
        let whole = TileKey {
          x: 0,
          y: 0,
          width: rect.width().ceil() as i32,
          height: rect.height().ceil() as i32,
          ..key
        };
        // Requested tiles first, so their rows encode first.
        let others: Vec<TileKey> =
          tiles::cover(whole, 0..whole.width, 0..whole.height)
            .filter(|key| !tiles.contains(key))
            .collect();
        let tiles = [tiles, others].concat();
        Job { tiles, work: Work::Whole { list, matrix, area } }
      }
    })
  }

  fn dispatch(&self, job: Job, urgent: bool) {
    if let Ok(mut queue) = self.shared.queue.lock() {
      queue.dispatched.extend(job.tiles.iter().copied());
    }
    let task = Task {
      shared: Arc::clone(&self.shared),
      send: self.send.clone(),
      job,
      urgent: urgent.then(Urgent::new),
    };
    let pool = pool();
    if let Ok(mut tasks) = pool.tasks.lock() {
      tasks.push_back(task);
      pool.ready.notify_one();
    }
  }
}

fn classify(list: &DisplayList, matrix: &Matrix, area: IRect) -> Result<Kind> {
  if !crate::prime::large(list, matrix, area)? {
    return Ok(Kind::Bands);
  }
  if i64::from(area.x1) * i64::from(area.y1) <= WHOLE {
    return Ok(Kind::Whole);
  }
  crate::prime::decode(list, matrix, area)?;
  Ok(Kind::Bands)
}

fn page_of(tile: &TileKey) -> PageKey {
  (tile.page, tile.scale, tile.rotation, tile.crop)
}

fn ms(started: Instant) -> f64 {
  started.elapsed().as_secs_f64() * 1000.0
}

/// Runs of horizontally adjacent tiles in one tile row, in the order of
/// their most urgent tile.
fn bands(tiles: &[TileKey]) -> Vec<Vec<TileKey>> {
  let mut rows: Vec<Vec<TileKey>> = Vec::new();
  for key in tiles {
    let row = rows
      .iter_mut()
      .find(|row| (page_of(&row[0]), row[0].y) == (page_of(key), key.y));
    match row {
      Some(row) => row.push(*key),
      None => rows.push(vec![*key]),
    }
  }
  let mut bands = Vec::new();
  for mut row in rows {
    row.sort_by_key(|key| key.x);
    let mut band: Vec<TileKey> = Vec::new();
    for key in row {
      if band.last().is_some_and(|last| last.x + TILE_WIDTH != key.x) {
        bands.push(std::mem::take(&mut band));
      }
      band.push(key);
    }
    bands.push(band);
  }
  bands
}

enum Work {
  /// Draw the band in one pass, then cut its tiles.
  Band { list: Arc<DisplayList>, matrix: Matrix },
  /// Draw the whole page `area`, then hand its tiles back to the pool.
  Whole { list: Arc<DisplayList>, matrix: Matrix, area: IRect },
  /// Encode tiles cut from a whole page.
  Cut { pixels: Vec<Vec<u8>>, drawn: f64 },
}

struct Job {
  tiles: Vec<TileKey>,
  work: Work,
}

struct Task {
  shared: Arc<Shared>,
  send: Sender,
  job: Job,
  urgent: Option<Urgent>,
}

struct Pool {
  tasks: Mutex<VecDeque<Task>>,
  ready: Condvar,
}

/// Threads per pool of CPU-bound helpers: half the cores, 1 to 4.
pub(crate) fn pool_size() -> usize {
  static SIZE: OnceLock<usize> = OnceLock::new();
  *SIZE.get_or_init(|| {
    thread::available_parallelism()
      .map_or(2, |count| count.get() / 2)
      .clamp(1, 4)
  })
}

/// Raster threads shared by every document, so panes and tabs do not
/// multiply them.
fn pool() -> &'static Pool {
  static POOL: OnceLock<Pool> = OnceLock::new();
  POOL.get_or_init(|| {
    for _ in 0..pool_size() {
      thread::spawn(|| serve(pool()));
    }
    Pool { tasks: Mutex::new(VecDeque::new()), ready: Condvar::new() }
  })
}

fn serve(pool: &Pool) {
  loop {
    let Ok(mut tasks) = pool.tasks.lock() else { return };
    let task = loop {
      if let Some(task) = tasks.pop_front() {
        break task;
      }
      tasks = match pool.ready.wait(tasks) {
        Ok(tasks) => tasks,
        Err(_) => return,
      };
    };
    drop(tasks);
    let Task { shared, send, job, urgent } = task;
    let Job { tiles, work } = job;
    #[cfg(test)]
    if matches!(work, Work::Whole { .. }) {
      shared.wholes.fetch_add(1, Ordering::Relaxed);
    }
    let handed = render(&tiles, work, &send);
    // Held through delivery: a scan started between a view's bands would
    // hold `MuPDF`'s locks while the next band draws.
    drop(urgent);
    if let Ok(mut queue) = shared.queue.lock() {
      let kept: HashSet<&TileKey> =
        handed.iter().flat_map(|job| &job.tiles).collect();
      for key in tiles.iter().filter(|key| !kept.contains(key)) {
        queue.dispatched.remove(key);
      }
      shared.wake.notify_all();
    }
    // Rows of a whole page encode on every free thread, before other work.
    if let Ok(mut tasks) = pool.tasks.lock() {
      for job in handed.into_iter().rev() {
        let (shared, send) = (Arc::clone(&shared), send.clone());
        tasks.push_front(Task { shared, send, job, urgent: None });
      }
      pool.ready.notify_all();
    }
  }
}

/// Draws the job's band and sends each tile encoded; a whole page
/// returns its rows as jobs for the pool to encode.
fn render(tiles: &[TileKey], work: Work, send: &Sender) -> Vec<Job> {
  let started = Instant::now();
  let (list, matrix, area, whole) = match work {
    Work::Band { list, matrix } => {
      let (first, last) = (tiles[0], tiles[tiles.len() - 1]);
      let area = IRect::new(
        first.x,
        first.y,
        last.x + last.width,
        first.y + first.height,
      );
      (list, matrix, area, false)
    }
    Work::Whole { list, matrix, area } => (list, matrix, area, true),
    Work::Cut { pixels, drawn } => {
      let pixels = pixels.into_iter().map(Ok);
      deliver(tiles, pixels, drawn, send);
      return Vec::new();
    }
  };
  let drawn = match tiles::draw(&list, &matrix, area) {
    Ok(drawn) => drawn,
    Err(error) => {
      drop(send.send(Err(error.to_string())));
      return Vec::new();
    }
  };
  let drawn_ms = ms(started);
  if !whole {
    let pixels = tiles.iter().map(|key| tiles::cut(&drawn, key));
    deliver(tiles, pixels, drawn_ms, send);
    return Vec::new();
  }
  let mut rows = Vec::new();
  for row in bands(tiles) {
    let pixels: Result<Vec<_>> =
      row.iter().map(|key| tiles::cut(&drawn, key)).collect();
    match pixels {
      Ok(pixels) => {
        rows.push(Job {
          tiles: row,
          work: Work::Cut { pixels, drawn: drawn_ms },
        });
      }
      Err(error) => drop(send.send(Err(error.to_string()))),
    }
  }
  rows
}

/// Sends `tiles` to the reader; `drawn` is the draw's milliseconds.
fn deliver(
  tiles: &[TileKey],
  pixels: impl Iterator<Item = Result<Vec<u8>>>,
  drawn: f64,
  send: &Sender,
) {
  for (key, pixels) in tiles.iter().zip(pixels) {
    let started = Instant::now();
    let tile = pixels
      .and_then(|pixels| tiles::Payload::new(&pixels))
      .map(|payload| Tile {
        key: *key,
        payload,
        render_ms: drawn + ms(started),
      })
      .map_err(|error| error.to_string());
    if send.send(tile).is_err() {
      return;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::document::Document;
  use std::sync::atomic::Ordering;

  /// A PDF of `count` letter pages, each one large gray image: scans.
  fn scans(count: usize) -> Result<tempfile::NamedTempFile> {
    use mupdf::{ColorParams, Colorspace, DocumentWriter, Image, Pixmap, Rect};
    let file = tempfile::Builder::new().suffix(".pdf").tempfile()?;
    let gray = Colorspace::device_gray();
    let mut pixels = Pixmap::new_with_w_h(&gray, 1024, 1024, false)?;
    for (index, value) in pixels.samples_mut().iter_mut().enumerate() {
      *value = (index % 251) as u8;
    }
    let scan = Image::from_pixmap(&pixels)?;
    let path = file.path().to_string_lossy();
    let mut writer = DocumentWriter::new(path.as_ref(), "pdf", "")?;
    let page = Matrix::new(612.0, 0.0, 0.0, 792.0, 0.0, 0.0);
    for _ in 0..count {
      let device = writer.begin_page(Rect::new(0.0, 0.0, 612.0, 792.0))?;
      device.fill_image(&scan, &page, 1.0, ColorParams::default())?;
      device.end()?;
    }
    drop(writer);
    Ok(file)
  }

  #[test]
  fn tiles_wake_the_loop() -> Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["body"], "")?;
    let doc = Document::open(file.path(), "")?;
    let (wake, woken) = crate::fixtures::wake();
    let worker = RenderWorker::new(file.path(), "", doc.lists(), &wake);
    let key = TileKey {
      page: PageId(0),
      scale: 1024,
      rotation: 0,
      x: 0,
      y: 0,
      width: 64,
      height: 64,
      crop: Crop::default(),
    };
    worker.request(&[key], 1);
    woken.recv_timeout(Duration::from_secs(10))?;
    let tile = worker.results.try_recv()?.map_err(anyhow::Error::msg)?;
    assert_eq!(tile.key, key);
    assert_eq!(woken.try_iter().count(), 0);
    Ok(())
  }

  #[test]
  fn prepares_latest_neighborhood() -> Result<()> {
    let file = tempfile::Builder::new().suffix(".pdf").tempfile()?;
    crate::fixtures::pdf(file.path(), &["body"; 30], "")?;
    let doc = Document::open(file.path(), "")?;
    let worker =
      RenderWorker::idle(file.path(), "", doc.lists(), &Wake::none());
    let drawing = Urgent::new();
    worker.prepare(&[], 0, &crate::prefetch::nearby(30, PageId(0), true));
    let latest = crate::prefetch::nearby(30, PageId(29), false);
    worker.prepare(&[], 0, &latest);
    drop(drawing);
    let started = Instant::now();
    loop {
      let lists = doc.lists();
      let lists = lists.lock().expect("display lists");
      if latest.iter().all(|page| lists.contains(page)) {
        break;
      }
      drop(lists);
      anyhow::ensure!(
        started.elapsed() < Duration::from_secs(10),
        "Prefetch timeout"
      );
      thread::sleep(Duration::from_millis(5));
    }
    assert!(
      worker.results.try_recv().is_err(),
      "preprocessing does not rasterize"
    );
    Ok(())
  }

  #[test]
  fn scans_yield_to_views() {
    assert!(yield_to_views(|| false));
    let drawing = Urgent::new();
    let stop = std::sync::atomic::AtomicBool::new(false);
    thread::scope(|scope| {
      let waiting =
        scope.spawn(|| yield_to_views(|| stop.load(Ordering::Relaxed)));
      thread::sleep(Duration::from_millis(20));
      assert!(!waiting.is_finished(), "returned while a view draws");
      stop.store(true, Ordering::Relaxed);
      assert!(!waiting.join().expect("waiting thread"), "stopped");
    });
    let waiting = thread::spawn(|| yield_to_views(|| false));
    drop(drawing);
    assert!(waiting.join().expect("waiting thread"), "views done");
  }

  /// Two scans in one view: rows alternate when a scroll up adds the
  /// target's rows above to those below, and prefetch returns to the
  /// first page after the visible tiles end on the second.
  #[test]
  fn two_scans_draw_once() -> Result<()> {
    let file = scans(2)?;
    let doc = Document::open(file.path(), "")?;
    let key = |page, y| TileKey {
      page: PageId(page),
      scale: 1024,
      rotation: 0,
      x: 0,
      y,
      width: TILE_WIDTH,
      height: 256,
      crop: Crop::default(),
    };
    let tiles = [key(0, 512), key(1, 0), key(0, 256), key(1, 256)];
    for urgent in [4, 2] {
      let worker =
        RenderWorker::new(file.path(), "", doc.lists(), &Wake::none());
      worker.request(&tiles, urgent);
      // Whole pages send every tile, not only the requested ones.
      let mut pending: HashSet<TileKey> = tiles.into_iter().collect();
      while !pending.is_empty() {
        let tile = worker
          .results
          .recv_timeout(Duration::from_secs(20))?
          .map_err(anyhow::Error::msg)?;
        pending.remove(&tile.key);
      }
      while worker.results.recv_timeout(Duration::from_millis(200)).is_ok() {}
      assert_eq!(worker.shared.wholes.load(Ordering::Relaxed), 2, "{urgent}");
    }
    Ok(())
  }
}
