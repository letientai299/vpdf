use crate::Result;
use crate::cache::Cache;
use crate::content_cache::{self, Alias, Pack, Records};
use crate::document::{Handle, MIB};
use crate::input::notify;
use crate::search::{self, Hit, Index};
use crate::text::{PageId, Text};
use anyhow::Context;
use std::fs::{File, Metadata};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

/// Idle time after which the worker releases its document.
const IDLE: Duration = Duration::from_secs(3);
/// Idle time after which new indexes go to disk; typing a query rewrites
/// none.
const SAVE: Duration = Duration::from_millis(500);

pub enum Request {
  Query {
    generation: u64,
    query: String,
    whole_word: bool,
    origin: PageId,
    backward: bool,
  },
  Seed(PageId, Arc<Text>),
}

pub struct SearchEvent {
  pub generation: u64,
  pub hits: Vec<Hit>,
  pub done: bool,
  pub error: Option<String>,
}

pub struct SearchWorker {
  send: mpsc::Sender<Request>,
  generation: Arc<AtomicU64>,
  pub events: mpsc::Receiver<SearchEvent>,
}

impl SearchWorker {
  pub fn new(path: &Path, password: &str) -> Self {
    let (send, receive) = mpsc::channel();
    let (events_send, events) = mpsc::sync_channel(32);
    let generation = Arc::new(AtomicU64::new(0));
    let current = Arc::clone(&generation);
    let path = path.to_owned();
    let password = password.to_owned();
    thread::spawn(move || {
      match Source::open(path, password, content_cache::root()) {
        Ok(source) => serve(source, &receive, &events_send, &current),
        Err(error) => fail(&receive, &events_send, &error),
      }
    });
    Self { send, generation, events }
  }

  pub fn query(
    &self,
    query: &str,
    whole_word: bool,
    origin: PageId,
    backward: bool,
  ) -> Result<u64> {
    let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
    self
      .send
      .send(Request::Query {
        generation,
        query: query.to_owned(),
        whole_word,
        origin,
        backward,
      })
      .map_err(|_| anyhow::anyhow!("Search worker stopped"))?;
    Ok(generation)
  }

  pub fn cancel(&self) {
    self.generation.fetch_add(1, Ordering::Relaxed);
  }

  pub fn seed(&self, page: PageId, text: Arc<Text>) {
    let _ = self.send.send(Request::Seed(page, text));
  }
}

/// Answers requests until the handle drops.
fn serve(
  mut source: Source,
  receive: &mpsc::Receiver<Request>,
  send: &mpsc::SyncSender<SearchEvent>,
  current: &AtomicU64,
) {
  let mut indexes = Cache::new(64 * MIB);
  while let Some(request) = next_request(receive, &mut source) {
    match request {
      Request::Seed(page, text) => {
        let index = Index::new(page, &text);
        let weight = index.weight();
        indexes.insert(page, index, weight);
        source.seed(page, text);
      }
      Request::Query { generation, .. }
        if current.load(Ordering::Relaxed) != generation => {}
      Request::Query { generation, query, whole_word, origin, backward } => {
        let task = SearchTask { generation, origin, backward, current, send };
        let result = task.run(&mut source, &mut indexes, &query, whole_word);
        let done = SearchEvent {
          generation,
          hits: Vec::new(),
          done: true,
          error: result.err().map(|e| e.to_string()),
        };
        if notify(send.send(done)).is_err() {
          break;
        }
      }
    }
  }
  source.save();
}

/// The next request; new indexes save and the document drops while none
/// arrives.
fn next_request(
  receive: &mpsc::Receiver<Request>,
  source: &mut Source,
) -> Option<Request> {
  match receive.recv_timeout(SAVE) {
    Ok(request) => return Some(request),
    Err(mpsc::RecvTimeoutError::Timeout) => source.save(),
    Err(mpsc::RecvTimeoutError::Disconnected) => return None,
  }
  match receive.recv_timeout(IDLE.checked_sub(SAVE).unwrap()) {
    Ok(request) => Some(request),
    Err(mpsc::RecvTimeoutError::Timeout) => {
      source.doc = None;
      receive.recv().ok()
    }
    Err(mpsc::RecvTimeoutError::Disconnected) => None,
  }
}

/// Answers every query with `error`, as the document did not open.
fn fail(
  receive: &mpsc::Receiver<Request>,
  send: &mpsc::SyncSender<SearchEvent>,
  error: &anyhow::Error,
) {
  for request in receive {
    let Request::Query { generation, .. } = request else { continue };
    let event = SearchEvent {
      generation,
      hits: Vec::new(),
      done: true,
      error: Some(error.to_string()),
    };
    if notify(send.send(event)).is_err() {
      return;
    }
  }
}

impl Drop for SearchWorker {
  fn drop(&mut self) {
    self.cancel();
  }
}

struct SearchTask<'a> {
  generation: u64,
  origin: PageId,
  backward: bool,
  current: &'a AtomicU64,
  send: &'a mpsc::SyncSender<SearchEvent>,
}

impl SearchTask<'_> {
  /// `page`'s index from disk, else from its text once views finish
  /// drawing; none when a newer query arrives first.
  fn load(&self, source: &mut Source, page: PageId) -> Result<Option<Index>> {
    if let Some(index) = source.saved(page) {
      return Ok(Some(index));
    }
    let stale = || self.current.load(Ordering::Relaxed) != self.generation;
    if source.reads_pdf(page) && !crate::render_worker::yield_to_views(stale) {
      return Ok(None);
    }
    source.index(page).map(Some)
  }

  fn run(
    &self,
    source: &mut Source,
    indexes: &mut Cache<PageId, Index>,
    query: &str,
    whole_word: bool,
  ) -> Result<()> {
    let pattern = search::compile(query, whole_word)?;
    let mut remaining: usize = 10_000;
    let pages = source.pages;
    for offset in 0..pages {
      if self.current.load(Ordering::Relaxed) != self.generation {
        break;
      }
      let page = if self.backward {
        PageId((self.origin.0 + pages - offset) % pages)
      } else {
        PageId((self.origin.0 + offset) % pages)
      };
      if indexes.get(&page).is_none() {
        let Some(index) = self.load(source, page)? else { break };
        let weight = index.weight();
        indexes.insert(page, index, weight);
      }
      let Some(index) = indexes.get(&page) else { continue };
      let hits = index.matches(&pattern, remaining.min(1000))?;
      remaining -= hits.len();
      if !hits.is_empty() {
        notify(self.send.send(SearchEvent {
          generation: self.generation,
          hits,
          done: false,
          error: None,
        }))?;
      }
      if remaining == 0 {
        break;
      }
    }
    Ok(())
  }
}

/// Page text for scans. The document drops after the worker idles,
/// which releases `MuPDF`'s objects and cached resources for every page a
/// scan read: about 110 MB after scanning a 1370-page book. Indexes stay,
/// so later queries reopen it only for pages they evicted.
struct Source {
  path: PathBuf,
  password: String,
  pages: usize,
  doc: Option<Handle>,
  /// OCR text, which replaces a page's extracted text.
  seeds: Vec<(PageId, Arc<Text>)>,
  /// This file's indexes on disk; none when the cache is off, a password
  /// protects the file, or the file changed.
  disk: Option<Disk>,
  /// Pages read from the PDF, which tests count.
  #[cfg(test)]
  extracted: usize,
}

/// Inode, size, modification, and change times: a replaced or rewritten
/// file differs in one.
type Identity = (u64, u64, Option<SystemTime>, i64, i64);

fn identity(meta: &Metadata) -> Identity {
  (
    meta.ino(),
    meta.len(),
    meta.modified().ok(),
    meta.ctime(),
    meta.ctime_nsec(),
  )
}

fn same_file(path: &Path, file: Identity) -> bool {
  std::fs::metadata(path).is_ok_and(|meta| identity(&meta) == file)
}

/// The SHA-256 of `path`, when hashing read the file `file` describes.
fn hash(path: &Path, file: Identity) -> Option<Alias> {
  let mut handle = File::open(path).ok()?;
  let alias = content_cache::alias(&mut handle).ok()?;
  (identity(&handle.metadata().ok()?) == file).then_some(alias)
}

struct Disk {
  root: PathBuf,
  /// The file as the document opened it.
  file: Identity,
  /// Hashing runs beside the first scan, which reads the PDF until the
  /// pack is known: a 150 MB book takes 270 ms to hash.
  hashing: Option<JoinHandle<Option<Alias>>>,
  /// The file's alias and its pack, once hashing ends.
  found: Option<(Alias, Option<Pack>)>,
  /// Pages indexed from the PDF since the last save.
  fresh: Records,
}

impl Source {
  /// Opens `path`, with its disk records under `root` unless a password
  /// protects it: decrypted text stays off disk.
  fn open(
    path: PathBuf,
    password: String,
    root: Option<PathBuf>,
  ) -> Result<Self> {
    let file = identity(&std::fs::metadata(&path)?);
    let doc = Handle::open(&path, &password)?;
    let disk = root.filter(|_| password.is_empty()).map(|root| {
      let hashed = path.clone();
      Disk {
        root,
        file,
        hashing: Some(thread::spawn(move || hash(&hashed, file))),
        found: None,
        fresh: Records::new(doc.pages),
      }
    });
    Ok(Self {
      path,
      password,
      pages: doc.pages,
      doc: Some(doc),
      seeds: Vec::new(),
      disk,
      #[cfg(test)]
      extracted: 0,
    })
  }

  /// Page text read straight from the page: the index keeps what a
  /// scan needs, so text and display list caches would hold it twice.
  fn text(&mut self, page: PageId) -> Result<Arc<Text>> {
    if let Some((_, text)) = self.seeds.iter().find(|(seed, _)| *seed == page) {
      return Ok(Arc::clone(text));
    }
    if self.doc.is_none() {
      self.doc = Some(Handle::open(&self.path, &self.password)?);
      // A replaced file must not add records under the old alias.
      if self
        .disk
        .as_ref()
        .is_some_and(|disk| !same_file(&self.path, disk.file))
      {
        self.disk = None;
      }
    }
    let doc = self.doc.as_ref().context("Search document closed")?;
    #[cfg(test)]
    {
      self.extracted += 1;
    }
    Ok(Arc::new(doc.text(page)?))
  }

  /// Takes the alias and pack once hashing ends, waiting for it when
  /// `wait`; a failed hash turns the disk off.
  fn settle(&mut self, wait: bool) {
    let pages = self.pages;
    let Some(disk) = &mut self.disk else { return };
    let Some(hashing) = disk.hashing.take_if(|job| wait || job.is_finished())
    else {
      return;
    };
    match hashing.join().ok().flatten() {
      Some(alias) => {
        let pack = Pack::open(&disk.root, &alias, pages);
        disk.found = Some((alias, pack));
      }
      None => self.disk = None,
    }
  }

  /// `page`'s index from disk, unless OCR replaced its text.
  fn saved(&mut self, page: PageId) -> Option<Index> {
    if !self.reads_pdf(page) {
      return None;
    }
    self.settle(false);
    let (_, pack) = self.disk.as_ref()?.found.as_ref()?;
    pack.as_ref()?.index(page)
  }

  /// Indexes `page`'s text, keeping PDF text for the disk.
  fn index(&mut self, page: PageId) -> Result<Index> {
    let text = self.text(page)?;
    let index = Index::new(page, &text);
    if self.reads_pdf(page)
      && let Some(disk) = &mut self.disk
    {
      // A record that fails to encode leaves the page uncached.
      drop(disk.fresh.add(page, &index));
    }
    Ok(index)
  }

  /// Writes the indexes extracted since the last save, unless the file
  /// changed since it opened.
  fn save(&mut self) {
    if self.disk.as_ref().is_none_or(|disk| disk.fresh.is_empty()) {
      return;
    }
    self.settle(true);
    let pages = self.pages;
    let same =
      self.disk.as_ref().is_some_and(|disk| same_file(&self.path, disk.file));
    let Some(disk) = self.disk.as_mut().filter(|_| same) else {
      self.disk = None;
      return;
    };
    let Some((alias, _)) = disk.found else { return };
    let fresh = std::mem::replace(&mut disk.fresh, Records::new(pages));
    let old = disk.found.as_ref().and_then(|(_, pack)| pack.as_ref());
    // Pages read while hashing ran are often on disk already.
    if !fresh.adds_to(old) {
      return;
    }
    if content_cache::save(&disk.root, &alias, &fresh, old).is_ok() {
      disk.found = Some((alias, Pack::open(&disk.root, &alias, pages)));
    }
  }

  /// Whether `page`'s text comes from the PDF rather than OCR.
  fn reads_pdf(&self, page: PageId) -> bool {
    self.seeds.iter().all(|(seed, _)| *seed != page)
  }

  fn seed(&mut self, page: PageId, text: Arc<Text>) {
    self.seeds.retain(|(old, _)| *old != page);
    self.seeds.push((page, text));
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::Position;

  /// First hits of a forward scan for `query`, and pages extracted.
  fn scan(source: &mut Source, query: &str) -> Result<(Vec<Hit>, usize)> {
    let before = source.extracted;
    let (send, events) = mpsc::sync_channel(4096);
    let current = AtomicU64::new(1);
    let task = SearchTask {
      generation: 1,
      origin: PageId(0),
      backward: false,
      current: &current,
      send: &send,
    };
    let mut indexes = Cache::new(64 * MIB);
    task.run(source, &mut indexes, query, false)?;
    drop(send);
    let hits = events.into_iter().flat_map(|event| event.hits).collect();
    Ok((hits, source.extracted - before))
  }

  fn ends(hits: &[Hit]) -> Vec<(Position, Position, &str)> {
    hits.iter().map(|hit| (hit.first, hit.last, hit.excerpt.as_str())).collect()
  }

  fn corpus_paper() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf"))
  }

  /// A reopened file answers from its pack without reading the PDF, with
  /// the hits its first scan found; OCR text still overrides the pack;
  /// a password keeps the pack unwritten.
  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn reopened_files_search_from_disk() -> Result<()> {
    let path = corpus_paper()?;
    let root = tempfile::tempdir()?;
    let open = || -> Result<Source> {
      let mut source =
        Source::open(path.clone(), String::new(), Some(root.path().into()))?;
      source.settle(true);
      Ok(source)
    };
    let mut first = open()?;
    let (hits, extracted) = scan(&mut first, "attention")?;
    first.save();
    assert_eq!(extracted, first.pages);
    assert!(!hits.is_empty());
    let mut second = open()?;
    let (again, extracted) = scan(&mut second, "attention")?;
    assert_eq!(extracted, 0);
    assert_eq!(ends(&again), ends(&hits));
    // Pages the pack holds, read again from the PDF, rewrite nothing.
    let pack = |root: &Path| -> Result<u64> {
      let entry = std::fs::read_dir(root)?.next().context("No pack")??;
      Ok(entry.metadata()?.ino())
    };
    let written = pack(root.path())?;
    second.index(PageId(0))?;
    second.save();
    assert_eq!(pack(root.path())?, written);
    let mut seeded = open()?;
    seeded.seed(PageId(0), Arc::new(Text::default()));
    let (rest, extracted) = scan(&mut seeded, "attention")?;
    assert_eq!(extracted, 0);
    assert!(rest.iter().all(|hit| hit.first.page != PageId(0)));
    let locked = tempfile::tempdir()?;
    let mut source =
      Source::open(path.clone(), "unused".into(), Some(locked.path().into()))?;
    scan(&mut source, "attention")?;
    source.save();
    assert_eq!(std::fs::read_dir(locked.path())?.count(), 0);
    Ok(())
  }

  /// Pages read from a file that changed after it opened, before saving
  /// or before reopening its document, never reach the disk.
  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn changed_files_store_nothing() -> Result<()> {
    let files = tempfile::tempdir()?;
    let path = files.path().join("paper.pdf");
    std::fs::copy(corpus_paper()?, &path)?;
    let touch = || -> Result<()> {
      let mut file = File::options().append(true).open(&path)?;
      std::io::Write::write_all(&mut file, b"\n%\n")?;
      Ok(())
    };
    let root = tempfile::tempdir()?;
    let mut source =
      Source::open(path.clone(), String::new(), Some(root.path().into()))?;
    scan(&mut source, "attention")?;
    touch()?;
    source.save();
    assert!(source.disk.is_none());
    let mut source =
      Source::open(path.clone(), String::new(), Some(root.path().into()))?;
    source.doc = None;
    touch()?;
    scan(&mut source, "attention")?;
    source.save();
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    Ok(())
  }
}
