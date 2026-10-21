use crate::Result;
use crate::cache::Cache;
use crate::content_cache::{self, Alias, Pack, Records, Version};
use crate::document::{Handle, MIB};
use crate::jobs::{Wake, WakingSender};
use crate::search::{self, Hit, Index};
use crate::text::{PageId, Text};
use anyhow::Context;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Idle time after which the worker releases its document.
static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

const IDLE: Duration = Duration::from_secs(3);
/// Idle time after which new indexes go to disk; typing a query rewrites
/// none.
const SAVE: Duration = Duration::from_millis(500);
/// Scan pages decoded from disk at a time, and their decoded bytes at
/// most, well within the index cache.
const BATCH: usize = 128;
const BATCH_BYTES: usize = 8 * MIB;

pub enum Request {
  Query { generation: u64, query: vpdf_search::query::Query, origin: PageId },
  Seed(PageId, Arc<Text>),
}

pub struct SearchEvent {
  pub generation: u64,
  pub hits: Vec<Hit>,
  pub done: bool,
  pub error: Option<String>,
}

pub struct SearchWorker {
  activity: super::Activity,
  send: mpsc::Sender<Request>,
  generation: Arc<AtomicU64>,
  events: mpsc::Receiver<SearchEvent>,
}

impl SearchWorker {
  pub fn activity(&self) -> super::Activity {
    self.activity.clone()
  }

  /// A worker for `path`, which the reader opened with `pages` pages.
  pub fn new(path: &Path, password: &str, pages: usize, wake: &Wake) -> Self {
    let (send, receive) = mpsc::channel();
    let (events_send, events) = mpsc::sync_channel(32);
    let events_send = wake.sender(events_send);
    let generation = Arc::new(AtomicU64::new(0));
    let current = Arc::clone(&generation);
    let path = path.to_owned();
    let password = password.to_owned();
    let activity = super::Activity::new(true);
    let running = activity.clone();
    thread::spawn(move || {
      let _guard = running.guard();
      match Source::open(path, password, content_cache::root(), pages) {
        Ok(source) => serve(source, &receive, &events_send, &current),
        Err(error) => fail(&receive, &events_send, &error),
      }
    });
    Self { activity, send, generation, events }
  }

  pub fn lookup(
    &self,
    query: &vpdf_search::query::Query,
    origin: PageId,
  ) -> Result<u64> {
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    self.generation.store(generation, Ordering::Relaxed);
    self
      .send
      .send(Request::Query { generation, query: query.clone(), origin })
      .map_err(|_| anyhow::anyhow!("Search worker stopped"))?;
    Ok(generation)
  }

  pub fn cancel(&self) {
    self.generation.fetch_add(1, Ordering::Relaxed);
  }

  pub fn seed(&self, page: PageId, text: Arc<Text>) {
    let _ = self.send.send(Request::Seed(page, text));
  }

  /// Events received so far; a stopped worker reports a failure of
  /// scan `generation`.
  /// The next event, waiting up to `timeout`.
  pub fn wait(
    &self,
    timeout: Duration,
  ) -> std::result::Result<SearchEvent, mpsc::RecvTimeoutError> {
    self.events.recv_timeout(timeout)
  }

  pub fn updates(&self, generation: u64) -> Vec<SearchEvent> {
    let mut events = Vec::new();
    loop {
      match self.events.try_recv() {
        Ok(event) => events.push(event),
        Err(mpsc::TryRecvError::Empty) => return events,
        Err(mpsc::TryRecvError::Disconnected) => {
          events.push(SearchEvent {
            generation,
            hits: Vec::new(),
            done: true,
            error: Some("Search worker stopped".into()),
          });
          return events;
        }
      }
    }
  }
}

/// Answers requests until the handle drops.
fn serve(
  mut source: Source,
  receive: &mpsc::Receiver<Request>,
  send: &WakingSender<mpsc::SyncSender<SearchEvent>>,
  current: &AtomicU64,
) {
  let mut indexes = Cache::new(64 * MIB);
  while let Some(request) = next_request(receive, &mut source) {
    match request {
      Request::Seed(page, text) => {
        keep(&mut indexes, page, Index::new(page, &text));
        source.seed(page, text);
      }
      Request::Query { generation, .. }
        if current.load(Ordering::Relaxed) != generation => {}
      Request::Query { generation, query, origin } => {
        let task = SearchTask {
          generation,
          origin,
          backward: query.backward,
          current,
          send,
        };
        let result = task.run(&mut source, &mut indexes, &query);
        let done = SearchEvent {
          generation,
          hits: Vec::new(),
          done: true,
          error: result.err().map(|e| e.to_string()),
        };
        if send.send(done).is_err() {
          break;
        }
      }
    }
  }
  source.save();
}

fn keep(indexes: &mut Cache<PageId, Index>, page: PageId, index: Index) {
  let weight = index.weight();
  indexes.insert(page, index, weight);
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
  send: &WakingSender<mpsc::SyncSender<SearchEvent>>,
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
    if send.send(event).is_err() {
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
  send: &'a WakingSender<mpsc::SyncSender<SearchEvent>>,
}

impl SearchTask<'_> {
  /// The page `offset` pages along the scan.
  fn page(&self, offset: usize, pages: usize) -> PageId {
    if self.backward {
      PageId((self.origin.0 + pages - offset) % pages)
    } else {
      PageId((self.origin.0 + offset) % pages)
    }
  }

  /// Decodes the next `BATCH` scan pages from `offset` that the disk
  /// holds and `indexes` lacks.
  fn prefetch(
    &self,
    source: &mut Source,
    indexes: &mut Cache<PageId, Index>,
    offset: usize,
  ) {
    if !source.has_pack() {
      return;
    }
    let pages = source.pages;
    let wanted: Vec<PageId> = (offset..pages.min(offset + BATCH))
      .map(|offset| self.page(offset, pages))
      .filter(|page| !indexes.contains(page))
      .collect();
    // Inserted last, the scan's next page is the last to evict.
    for (page, index) in source.saved(&wanted).into_iter().rev() {
      keep(indexes, page, index);
    }
  }

  /// `page`'s index from its text once views finish drawing; none when
  /// a newer query arrives first.
  fn load(&self, source: &mut Source, page: PageId) -> Result<Option<Index>> {
    let stale = || self.current.load(Ordering::Relaxed) != self.generation;
    if source.reads_pdf(page) && !crate::scheduling::yield_to_views(stale) {
      return Ok(None);
    }
    source.index(page).map(Some)
  }

  fn run(
    &self,
    source: &mut Source,
    indexes: &mut Cache<PageId, Index>,
    query: &vpdf_search::query::Query,
  ) -> Result<()> {
    let pattern = search::compile(&query.text, query.whole)?;
    let mut remaining: usize = 10_000;
    let pages = source.pages;
    for offset in 0..pages {
      if self.current.load(Ordering::Relaxed) != self.generation {
        break;
      }
      let page = self.page(offset, pages);
      if indexes.get(&page).is_none() {
        self.prefetch(source, indexes, offset);
      }
      if indexes.get(&page).is_none() {
        let Some(index) = self.load(source, page)? else { break };
        keep(indexes, page, index);
      }
      let Some(index) = indexes.get(&page) else { continue };
      let hits = if let Some(reference) = &query.reference {
        index
          .definitions(&reference.reference, remaining.min(1000))
          .into_iter()
          .filter(|hit| {
            (hit.first.page, hit.first.line)
              != (reference.origin.page, reference.origin.line)
          })
          .collect()
      } else {
        index.matches(&pattern, remaining.min(1000))?
      };
      if query.whole
        && let Some(index) = indexes.remove(&page)
      {
        keep(indexes, page, index);
      }
      remaining -= hits.len();
      if !hits.is_empty() {
        self.send.send(SearchEvent {
          generation: self.generation,
          hits,
          done: false,
          error: None,
        })?;
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

fn same_file(path: &Path, file: Version) -> bool {
  Version::read(path) == Some(file)
}

/// The SHA-256 of `path`, when hashing read the file `file` describes.
fn hash(path: &Path, file: Version) -> Option<Alias> {
  let mut handle = File::open(path).ok()?;
  let alias = content_cache::alias(&mut handle).ok()?;
  (Version::of(&handle.metadata().ok()?) == file).then_some(alias)
}

struct Disk {
  root: PathBuf,
  /// The file as the worker opened it.
  file: Version,
  /// Whether `file` pins its bytes, so memos may stand in for hashing.
  exact: bool,
  /// Hashing runs beside scans, which read the PDF until it ends: a
  /// 150 MB book takes about 50 ms. A remembered alias skips it.
  hashing: Option<JoinHandle<Option<Alias>>>,
  /// The file's alias and its pack, once remembered or hashed.
  found: Option<(Alias, Option<Pack>)>,
  /// Pages indexed from the PDF since the last save.
  fresh: Records,
}

impl Disk {
  /// `alias`'s pack, when it covers `pages` pages, remembered as this
  /// file's where versions are exact.
  fn open_pack(&self, alias: &Alias, pages: usize) -> Option<Pack> {
    let pack =
      Pack::open(&self.root, alias).filter(|pack| pack.pages() == pages);
    if pack.is_some() && self.exact {
      // A memo that fails to write only costs the next open a hash.
      drop(content_cache::remember(&self.root, self.file, alias));
    }
    pack
  }
}

impl Source {
  /// Opens `path`, which the reader opened with `pages` pages, with its
  /// disk records under `root` unless a password protects it: decrypted
  /// text stays off disk. Where versions are exact, a remembered alias
  /// whose pack has `pages` pages skips hashing, and the PDF opens only
  /// when a page misses.
  fn open(
    path: PathBuf,
    password: String,
    root: Option<PathBuf>,
    pages: usize,
  ) -> Result<Self> {
    let file = Version::of(&std::fs::metadata(&path)?);
    let root = root.filter(|_| password.is_empty());
    let exact = content_cache::exact(&path);
    let found = root.as_ref().filter(|_| exact).and_then(|root| {
      let alias = content_cache::recall(root, file)?;
      let pack = Pack::open(root, &alias).filter(|pack| pack.pages() == pages);
      Some((alias, Some(pack?)))
    });
    let doc = match found {
      Some(_) => None,
      None => Some(Handle::open(&path, &password)?),
    };
    let pages = doc.as_ref().map_or(pages, |doc| doc.pages);
    let disk = root.map(|root| {
      let hashed = path.clone();
      Disk {
        root,
        file,
        exact,
        hashing: found
          .is_none()
          .then(|| thread::spawn(move || hash(&hashed, file))),
        found,
        fresh: Records::new(pages),
      }
    });
    Ok(Self {
      path,
      password,
      pages,
      doc,
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
      let doc = Handle::open(&self.path, &self.password)?;
      // Hits must stay within the pages the reader shows.
      anyhow::ensure!(doc.pages == self.pages, "PDF changed; search again");
      self.doc = Some(doc);
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
      Some(alias) => disk.found = Some((alias, disk.open_pack(&alias, pages))),
      None => self.disk = None,
    }
  }

  /// The pack, once hashing found one.
  fn pack(&mut self) -> Option<&Pack> {
    self.settle(false);
    self.disk.as_ref()?.found.as_ref()?.1.as_ref()
  }

  fn has_pack(&mut self) -> bool {
    self.pack().is_some()
  }

  /// Indexes from disk of the leading `pages` OCR left alone.
  fn saved(&mut self, pages: &[PageId]) -> Vec<(PageId, Index)> {
    let pages: Vec<PageId> =
      pages.iter().copied().filter(|&page| self.reads_pdf(page)).collect();
    self.pack().map_or_else(Vec::new, |pack| {
      pack.indexes(&pages, BATCH_BYTES, super::Config::default().helpers)
    })
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
      disk.found = Some((alias, disk.open_pack(&alias, pages)));
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
  use std::os::unix::fs::MetadataExt;

  /// First hits of a forward scan for `query`, and pages extracted.
  fn scan(source: &mut Source, query: &str) -> Result<(Vec<Hit>, usize)> {
    let before = source.extracted;
    let (send, events) = mpsc::sync_channel(4096);
    let send = Wake::none().sender(send);
    let current = AtomicU64::new(1);
    let task = SearchTask {
      generation: 1,
      origin: PageId(0),
      backward: false,
      current: &current,
      send: &send,
    };
    let mut indexes = Cache::new(64 * MIB);
    let query = vpdf_search::query::Query {
      text: query.to_owned(),
      ..vpdf_search::query::Query::default()
    };
    task.run(source, &mut indexes, &query)?;
    drop(send);
    let hits = events.into_iter().flat_map(|event| event.hits).collect();
    Ok((hits, source.extracted - before))
  }

  #[test]
  fn dictionary_growth_respects_cache_budget() -> Result<()> {
    let files = tempfile::tempdir()?;
    let path = files.path().join("one.pdf");
    crate::fixtures::pdf(&path, &["page"], "")?;
    let text = Arc::new(Text {
      lines: vec![crate::text::Line {
        paragraph: true,
        glyphs: "中文排版需求"
          .chars()
          .map(|ch| crate::text::Glyph {
            ch,
            bounds: crate::text::Bounds::default(),
          })
          .collect(),
      }],
      truncated: false,
    });
    let index = Index::new(PageId(0), &text);
    let budget = index.weight() + 1;
    let mut indexes = Cache::new(budget);
    keep(&mut indexes, PageId(0), index);
    let mut source = Source::open(path, String::new(), None, 1)?;
    source.seed(PageId(0), text);
    let (send, events) = mpsc::sync_channel(32);
    let send = Wake::none().sender(send);
    let current = AtomicU64::new(1);
    let task = SearchTask {
      generation: 1,
      origin: PageId(0),
      backward: false,
      current: &current,
      send: &send,
    };
    let query = vpdf_search::query::Query {
      text: "排版".into(),
      whole: true,
      ..vpdf_search::query::Query::default()
    };
    task.run(&mut source, &mut indexes, &query)?;
    assert!(indexes.used() <= budget);
    assert!(!indexes.contains(&PageId(0)));
    assert_eq!(events.try_recv()?.hits.len(), 1);
    assert_eq!(source.extracted, 0);
    Ok(())
  }

  #[test]
  fn stopped_worker_fails_the_scan() {
    let (send, _) = mpsc::channel();
    let (events_send, events) = mpsc::sync_channel(1);
    let worker = SearchWorker {
      send,
      generation: Arc::default(),
      events,
      activity: super::super::Activity::new(false),
    };
    let hits = Vec::new();
    let event = SearchEvent { generation: 3, hits, done: false, error: None };
    events_send.send(event).unwrap();
    drop(events_send);
    let updates = worker.updates(4);
    let seen: Vec<_> = updates.iter().map(|u| (u.generation, u.done)).collect();
    assert_eq!(seen, [(3, false), (4, true)]);
    assert_eq!(updates[1].error.as_deref(), Some("Search worker stopped"));
    assert!(worker.updates(4)[0].error.is_some(), "still stopped");
  }

  fn ends(hits: &[Hit]) -> Vec<(Position, Position, &str)> {
    hits.iter().map(|hit| (hit.first, hit.last, hit.excerpt.as_str())).collect()
  }

  fn corpus_paper() -> Result<PathBuf> {
    Ok(PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf"))
  }

  fn open(path: &Path, root: &Path) -> Result<Source> {
    let pages = Handle::open(path, "")?.pages;
    let mut source =
      Source::open(path.into(), String::new(), Some(root.into()), pages)?;
    source.settle(true);
    Ok(source)
  }

  fn pack_inode(root: &Path) -> Result<u64> {
    let entry = std::fs::read_dir(root)?
      .flatten()
      .find(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
      .context("No pack")?;
    Ok(entry.metadata()?.ino())
  }

  /// A reopened file answers from its pack, without opening the PDF where
  /// versions are exact; pages read again rewrite nothing; OCR text
  /// overrides the pack; a password keeps the pack unwritten.
  #[test]
  fn reopened_files_search_from_disk() -> Result<()> {
    let files = tempfile::tempdir()?;
    let path = files.path().join("two.pdf");
    crate::fixtures::pdf(&path, &["alpha one", "beta two"], "")?;
    let root = tempfile::tempdir()?;
    let mut first = open(&path, root.path())?;
    let (hits, extracted) = scan(&mut first, "beta")?;
    first.save();
    assert_eq!((hits.len(), extracted), (1, 2));
    let mut second = open(&path, root.path())?;
    let (again, extracted) = scan(&mut second, "beta")?;
    assert_eq!(extracted, 0);
    assert_eq!(ends(&again), ends(&hits));
    assert_eq!(second.doc.is_none(), content_cache::exact(&path));
    let written = pack_inode(root.path())?;
    second.index(PageId(0))?;
    second.save();
    assert_eq!(pack_inode(root.path())?, written);
    // OCR text overrides the pack.
    let mut seeded = open(&path, root.path())?;
    seeded.seed(PageId(1), Arc::new(Text::default()));
    let (rest, extracted) = scan(&mut seeded, "beta")?;
    assert_eq!((rest.len(), extracted), (0, 0));
    // A password keeps the disk out.
    let locked = tempfile::tempdir()?;
    let mut source =
      Source::open(path, "unused".into(), Some(locked.path().into()), 2)?;
    scan(&mut source, "beta")?;
    source.save();
    assert_eq!(std::fs::read_dir(locked.path())?.count(), 0);
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn real_multilingual_disk_cache() -> Result<()> {
    let corpus = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    for (name, query, expected_hash) in [
      (
        "vietnamese.pdf",
        "viet nam",
        "5ed2b5f73b6543283c8637e83e0d3ce7ef10b1050fcb0f190b945c8530be6195",
      ),
      (
        "chinese.pdf",
        "经济",
        "387113cf0dc2bc501d5fb77e681666f203f34e35f452fa9bd1f5b00ffe5bbc8e",
      ),
    ] {
      let path = corpus.join(name);
      let alias = content_cache::alias(&mut File::open(&path)?)?;
      assert_eq!(content_cache::hex(&alias), expected_hash);
      let root = tempfile::tempdir()?;
      let mut cold = open(&path, root.path())?;
      let pages = cold.pages;
      let (missing, extracted) = scan(&mut cold, "qzxjvw")?;
      assert!(missing.is_empty());
      assert_eq!(extracted, pages);
      cold.save();
      let (expected, extracted) = scan(&mut cold, query)?;
      assert_eq!(extracted, 0, "{name}");
      assert!(!expected.is_empty(), "{name}");
      cold.save();
      let written = pack_inode(root.path())?;
      drop(cold);
      let mut warm = Source::open(
        path.clone(),
        String::new(),
        Some(root.path().into()),
        pages,
      )?;
      warm.settle(true);
      let (actual, extracted) = scan(&mut warm, query)?;
      assert_eq!(extracted, 0, "{name}");
      assert_eq!(ends(&actual), ends(&expected), "{name}");
      warm.save();
      assert_eq!(pack_inode(root.path())?, written, "{name}");
      assert_eq!(warm.doc.is_none(), content_cache::exact(&path));
      assert_eq!(content_cache::alias(&mut File::open(&path)?)?, alias);
    }
    Ok(())
  }

  /// A memo whose pack has another page count is ignored, and its pack
  /// survives the other file's save.
  #[test]
  fn memos_need_the_readers_page_count() -> Result<()> {
    let files = tempfile::tempdir()?;
    let (two, three) =
      (files.path().join("two.pdf"), files.path().join("three.pdf"));
    crate::fixtures::pdf(&two, &["alpha", "beta"], "")?;
    crate::fixtures::pdf(&three, &["gamma", "delta", "beta"], "")?;
    let root = tempfile::tempdir()?;
    let mut source = open(&two, root.path())?;
    scan(&mut source, "beta")?;
    source.save();
    let alias = content_cache::alias(&mut File::open(&two)?)?;
    let written = pack_inode(root.path())?;
    let version = Version::read(&three).context("version")?;
    content_cache::remember(root.path(), version, &alias)?;
    let mut source = open(&three, root.path())?;
    assert!(source.doc.is_some());
    let (hits, extracted) = scan(&mut source, "beta")?;
    assert_eq!((source.pages, extracted), (3, 3));
    assert_eq!(
      hits.iter().map(|hit| hit.first.page).collect::<Vec<_>>(),
      [PageId(2)]
    );
    source.save();
    let pack = Pack::open(root.path(), &alias).context("two's pack")?;
    assert_eq!(pack.pages(), 2);
    assert!(std::fs::read_dir(root.path())?.flatten().any(|entry| {
      entry.metadata().is_ok_and(|meta| meta.ino() == written)
    }));
    Ok(())
  }

  /// A document that opens with another page count than the reader's
  /// stops the scan instead of returning pages the reader lacks.
  #[test]
  fn lazy_opens_check_the_page_count() -> Result<()> {
    let files = tempfile::tempdir()?;
    let path = files.path().join("two.pdf");
    crate::fixtures::pdf(&path, &["alpha", "beta"], "")?;
    let mut source = Source::open(path, String::new(), None, 2)?;
    source.doc = None;
    source.pages = 3;
    assert!(source.text(PageId(0)).is_err());
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn encrypted_search_stays_private() -> Result<()> {
    let path = corpus_paper()?;
    let original = content_cache::alias(&mut File::open(&path)?)?;
    let cache = tempfile::tempdir()?;
    let mut plain = open(&path, cache.path())?;
    let (expected, _) = scan(&mut plain, "attention")?;
    assert!(!expected.is_empty());
    plain.save();
    pack_inode(cache.path())?;

    let mut doc = Handle::open(&path, "")?;
    let file = crate::encryption::encrypt(&mut doc.native)?;
    let before = content_cache::alias(&mut File::open(file.path())?)?;
    let cache = tempfile::tempdir()?;
    for password in ["", "incorrect"] {
      assert!(
        Source::open(
          file.path().into(),
          password.into(),
          Some(cache.path().into()),
          doc.pages,
        )
        .is_err()
      );
    }
    let mut encrypted = Source::open(
      file.path().into(),
      "test-reader".into(),
      Some(cache.path().into()),
      doc.pages,
    )?;
    for _ in 0..2 {
      let (actual, extracted) = scan(&mut encrypted, "attention")?;
      assert_eq!(ends(&actual), ends(&expected));
      assert_eq!(extracted, doc.pages);
      encrypted.save();
      assert_eq!(std::fs::read_dir(cache.path())?.count(), 0);
      encrypted.doc = None;
    }
    assert_eq!(content_cache::alias(&mut File::open(&path)?)?, original);
    assert_eq!(content_cache::alias(&mut File::open(file.path())?)?, before);
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
    let pages = Handle::open(&path, "")?.pages;
    let mut source = Source::open(
      path.clone(),
      String::new(),
      Some(root.path().into()),
      pages,
    )?;
    scan(&mut source, "attention")?;
    touch()?;
    source.save();
    assert!(source.disk.is_none());
    let mut source = Source::open(
      path.clone(),
      String::new(),
      Some(root.path().into()),
      pages,
    )?;
    source.doc = None;
    touch()?;
    scan(&mut source, "attention")?;
    source.save();
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    Ok(())
  }
}
