//! Tile rendering off the input thread.
//!
//! A coordinator per document owns its MuPDF document, builds display
//! lists, and splits each request into bands: runs of adjacent tiles in
//! one tile row. A process-wide pool draws each band in one pass, cuts
//! its tiles, and encodes them. Display lists are immutable and MuPDF
//! renders them from several threads, each with its own context, while
//! only the coordinator touches the document; a list may outlive it, as
//! Type3 glyphs draw from lists compiled when their font loads. Pages
//! with large images render whole on the coordinator, which keeps the
//! last such page.
use crate::Result;
use crate::document::{Document, Lists};
use crate::layout::{Crop, page_matrix, transform};
use crate::text::PageId;
use crate::tiles::{self, TILE_WIDTH, Tile, TileKey};
use mupdf::{DisplayList, IRect, Matrix, Pixmap};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Instant;

/// Largest image page rendered whole, in pixels; about 24 MiB of RGB.
const WHOLE: i64 = 8 << 20;

type Sender = mpsc::Sender<std::result::Result<Tile, String>>;

#[derive(Default)]
struct Queue {
  /// Latest request in priority order; the coordinator drains it.
  tiles: Vec<TileKey>,
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
}

pub struct RenderWorker {
  shared: Arc<Shared>,
  pub results: mpsc::Receiver<std::result::Result<Tile, String>>,
}

impl RenderWorker {
  /// Opens its own handle of `path`, sharing display lists through
  /// `lists`.
  pub fn new(path: &Path, password: &str, lists: Arc<Lists>) -> Self {
    let shared = Arc::new(Shared::default());
    let work = Arc::clone(&shared);
    let (send, results) = mpsc::channel();
    let path = path.to_owned();
    let password = password.to_owned();
    thread::spawn(move || match Document::open(&path, &password) {
      Ok(mut doc) => {
        doc.share(lists);
        Coordinator::new(doc, work, send).run();
      }
      Err(error) => drop(send.send(Err(error.to_string()))),
    });
    Self { shared, results }
  }

  /// Replaces queued work with `tiles`, most urgent first. Tiles after
  /// the first `urgent` prefetch: they draw one band at a time while the
  /// pool is otherwise idle, so a new viewport waits for one band at most.
  pub fn request(&self, tiles: &[TileKey], urgent: usize) {
    if let Ok(mut queue) = self.shared.queue.lock() {
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

/// How a page renders at one scale.
#[derive(Clone, Copy)]
enum Kind {
  Bands,
  /// Large images scale differently by clip origin; draw whole.
  Whole,
}

type PageKey = (PageId, u32, u16, Crop);

struct Coordinator {
  doc: Document,
  shared: Arc<Shared>,
  send: Sender,
  kinds: HashMap<PageKey, Kind>,
  /// The last whole page and milliseconds it took to draw.
  whole: Option<(PageKey, Pixmap, f64)>,
}

impl Coordinator {
  fn new(doc: Document, shared: Arc<Shared>, send: Sender) -> Self {
    Self { doc, shared, send, kinds: HashMap::new(), whole: None }
  }

  fn run(mut self) {
    while let Some((urgent, ahead)) = self.next() {
      for band in bands(&urgent) {
        if !self.current() {
          break;
        }
        self.start(band);
      }
      for band in bands(&ahead) {
        if !self.idle() {
          break;
        }
        self.start(band);
      }
    }
  }

  fn start(&mut self, band: Vec<TileKey>) {
    match self.job(band) {
      Ok(job) => self.dispatch(job),
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
  fn next(&self) -> Option<(Vec<TileKey>, Vec<TileKey>)> {
    let mut queue = self.shared.queue.lock().ok()?;
    while !queue.fresh && !queue.stop {
      queue = self.shared.wake.wait(queue).ok()?;
    }
    queue.fresh = false;
    let mut dropped = Vec::new();
    if let Ok(mut tasks) = pool().tasks.lock() {
      for task in std::mem::take(&mut *tasks) {
        let stale = Arc::ptr_eq(&task.shared, &self.shared)
          && !task.job.tiles.iter().any(|key| queue.wanted.contains(key));
        if stale {
          for key in &task.job.tiles {
            queue.dispatched.remove(key);
          }
          dropped.push(task);
        } else {
          tasks.push_back(task);
        }
      }
    }
    if queue.stop {
      return None;
    }
    let mut urgent = std::mem::take(&mut queue.tiles);
    let mut ahead = urgent.split_off(queue.urgent);
    urgent.retain(|key| !queue.dispatched.contains(key));
    ahead.retain(|key| !queue.dispatched.contains(key));
    drop(queue);
    drop(dropped);
    Some((urgent, ahead))
  }

  fn job(&mut self, tiles: Vec<TileKey>) -> Result<Job> {
    let key = tiles[0];
    self.doc.validate(key.page)?;
    let page = (key.page, key.scale, key.rotation, key.crop);
    let bounds = key.crop.apply(self.doc.pages[key.page.0].bounds);
    let matrix = page_matrix(bounds, key.scale, key.rotation);
    let rect = transform(bounds, &matrix);
    let area = IRect::new(0, 0, rect.x1.ceil() as i32, rect.y1.ceil() as i32);
    let list = self.doc.display(key.page)?;
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
    let work = match kind {
      Kind::Bands => Work::Band { list, matrix },
      Kind::Whole => self.cut(page, &list, &matrix, area, &tiles)?,
    };
    Ok(Job { tiles, work })
  }

  /// Cuts `tiles` from the whole page, drawing it on first use.
  fn cut(
    &mut self,
    page: PageKey,
    list: &DisplayList,
    matrix: &Matrix,
    area: IRect,
    tiles: &[TileKey],
  ) -> Result<Work> {
    if self.whole.as_ref().is_none_or(|(key, ..)| *key != page) {
      // Frees the previous page before drawing the next.
      self.whole = None;
      let started = Instant::now();
      let pixmap = tiles::draw(list, matrix, area)?;
      self.whole = Some((page, pixmap, ms(started)));
    }
    let Some((_, pixmap, drawn)) = &self.whole else {
      anyhow::bail!("Whole page missing");
    };
    let pixels =
      tiles.iter().map(|key| tiles::cut(pixmap, key)).collect::<Result<_>>()?;
    Ok(Work::Cut { pixels, drawn: *drawn })
  }

  fn dispatch(&self, job: Job) {
    if let Ok(mut queue) = self.shared.queue.lock() {
      queue.dispatched.extend(job.tiles.iter().copied());
    }
    let task =
      Task { shared: Arc::clone(&self.shared), send: self.send.clone(), job };
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

fn ms(started: Instant) -> f64 {
  started.elapsed().as_secs_f64() * 1000.0
}

/// Runs of horizontally adjacent tiles in one tile row, in the order of
/// their most urgent tile.
fn bands(tiles: &[TileKey]) -> Vec<Vec<TileKey>> {
  let mut rows: Vec<Vec<TileKey>> = Vec::new();
  for key in tiles {
    let row = rows.iter_mut().find(|row| {
      let first = &row[0];
      (first.page, first.scale, first.rotation, first.crop, first.y)
        == (key.page, key.scale, key.rotation, key.crop, key.y)
    });
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
  /// Pixels already cut from a whole page.
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
}

struct Pool {
  tasks: Mutex<VecDeque<Task>>,
  ready: Condvar,
}

/// Raster threads shared by every document, so panes and tabs do not
/// multiply them.
fn pool() -> &'static Pool {
  static POOL: OnceLock<Pool> = OnceLock::new();
  POOL.get_or_init(|| {
    let threads = thread::available_parallelism()
      .map_or(2, |count| count.get() / 2)
      .clamp(1, 4);
    for _ in 0..threads {
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
    let Task { shared, send, job } = task;
    let Job { tiles, work } = job;
    render(&tiles, work, &send);
    if let Ok(mut queue) = shared.queue.lock() {
      for key in &tiles {
        queue.dispatched.remove(key);
      }
      shared.wake.notify_all();
    }
  }
}

/// Draws or takes the job's pixels and sends each tile encoded.
fn render(tiles: &[TileKey], work: Work, send: &Sender) {
  let started = Instant::now();
  let (pixels, drawn): (Vec<Result<Vec<u8>>>, f64) = match work {
    Work::Band { list, matrix } => {
      let (first, last) = (tiles[0], tiles[tiles.len() - 1]);
      let area = IRect::new(
        first.x,
        first.y,
        last.x + last.width,
        first.y + first.height,
      );
      let band = match tiles::draw(&list, &matrix, area) {
        Ok(band) => band,
        Err(error) => {
          drop(send.send(Err(error.to_string())));
          return;
        }
      };
      let cut = tiles.iter().map(|key| tiles::cut(&band, key)).collect();
      (cut, ms(started))
    }
    Work::Cut { pixels, drawn } => {
      (pixels.into_iter().map(Ok).collect(), drawn)
    }
  };
  for (key, pixels) in tiles.iter().zip(pixels) {
    let started = Instant::now();
    let tile = pixels
      .and_then(|pixels| tiles::encode(&pixels))
      .map(|encoded| Tile {
        key: *key,
        encoded,
        render_ms: drawn + ms(started),
      })
      .map_err(|error| error.to_string());
    if send.send(tile).is_err() {
      return;
    }
  }
}
