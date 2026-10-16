use crate::Result;
use crate::cache::Cache;
use crate::document::{Document, MIB};
use crate::search::{self, Hit, Index};
use crate::text::{PageId, Text};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

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
      let mut doc = match Document::open(&path, &password) {
        Ok(doc) => doc,
        Err(error) => {
          let _ = events_send.send(SearchEvent {
            generation: current.load(Ordering::Relaxed),
            hits: Vec::new(),
            done: true,
            error: Some(error.to_string()),
          });
          return;
        }
      };
      let mut indexes = Cache::new(32 * MIB);
      while let Ok(request) = receive.recv() {
        match request {
          Request::Seed(page, text) => {
            let index = Index::new(page, &text);
            let weight = index.weight();
            indexes.insert(page, index, weight);
            doc.seed(page, text);
          }
          Request::Query {
            generation,
            query,
            whole_word,
            origin,
            backward,
          } => {
            if current.load(Ordering::Relaxed) != generation {
              continue;
            }
            let task = SearchTask {
              generation,
              origin,
              backward,
              current: &current,
              send: &events_send,
            };
            let result = task.run(&mut doc, &mut indexes, &query, whole_word);
            if events_send
              .send(SearchEvent {
                generation,
                hits: Vec::new(),
                done: true,
                error: result.err().map(|e| e.to_string()),
              })
              .is_err()
            {
              return;
            }
          }
        }
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
  ) -> u64 {
    let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = self.send.send(Request::Query {
      generation,
      query: query.to_owned(),
      whole_word,
      origin,
      backward,
    });
    generation
  }

  pub fn cancel(&self) {
    self.generation.fetch_add(1, Ordering::Relaxed);
  }

  pub fn seed(&self, page: PageId, text: Arc<Text>) {
    let _ = self.send.send(Request::Seed(page, text));
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
    doc: &mut Document,
    indexes: &mut Cache<PageId, Index>,
    query: &str,
    whole_word: bool,
  ) -> Result<()> {
    let pattern = search::compile(query, whole_word)?;
    let mut remaining: usize = 10_000;
    for offset in 0..doc.pages.len() {
      if self.current.load(Ordering::Relaxed) != self.generation {
        break;
      }
      let page = if self.backward {
        PageId((self.origin.0 + doc.pages.len() - offset) % doc.pages.len())
      } else {
        PageId((self.origin.0 + offset) % doc.pages.len())
      };
      if indexes.get(&page).is_none() {
        let text = doc.text(page)?;
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
