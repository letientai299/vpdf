use crate::Result;
use crate::cache::Cache;
use crate::document::{Handle, MIB};
use crate::search::{self, Hit, Index};
use crate::text::{PageId, Text};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

/// Idle time after which the worker releases its document.
const IDLE: Duration = Duration::from_secs(3);

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
    thread::spawn(move || match Source::open(path, password) {
      Ok(source) => serve(source, &receive, &events_send, &current),
      Err(error) => fail(&receive, &events_send, &error),
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
  let mut indexes = Cache::new(32 * MIB);
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
        if send.send(done).is_err() {
          return;
        }
      }
    }
  }
}

/// The next request; the document drops while none arrives.
fn next_request(
  receive: &mpsc::Receiver<Request>,
  source: &mut Source,
) -> Option<Request> {
  match receive.recv_timeout(IDLE) {
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
  send: &'a mpsc::SyncSender<SearchEvent>,
}

impl SearchTask<'_> {
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
        let text = source.text(page)?;
        let index = Index::new(page, &text);
        let weight = index.weight();
        indexes.insert(page, index, weight);
      }
      let Some(index) = indexes.get(&page) else { continue };
      let hits = index.matches(&pattern, remaining.min(1000))?;
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
}

impl Source {
  fn open(path: PathBuf, password: String) -> Result<Self> {
    let doc = Handle::open(&path, &password)?;
    Ok(Self {
      path,
      password,
      pages: doc.pages,
      doc: Some(doc),
      seeds: Vec::new(),
    })
  }

  /// Page text read straight from the page: the index keeps what a
  /// scan needs, so text and display list caches would hold it twice.
  fn text(&mut self, page: PageId) -> Result<Arc<Text>> {
    if let Some((_, text)) = self.seeds.iter().find(|(seed, _)| *seed == page) {
      return Ok(Arc::clone(text));
    }
    let doc = match &mut self.doc {
      Some(doc) => doc,
      None => self.doc.insert(Handle::open(&self.path, &self.password)?),
    };
    Ok(Arc::new(doc.text(page)?))
  }

  fn seed(&mut self, page: PageId, text: Arc<Text>) {
    self.seeds.retain(|(old, _)| *old != page);
    self.seeds.push((page, text));
  }
}
