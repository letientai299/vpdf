use crate::Result;
use crate::app::App;
use crate::keys::PanelKind;
use crate::panel::Panel;
use crate::prompt::{Prompt, PromptKind};
use crate::reader::{self, Location};
use crate::search::{self, Hit};
use crate::search_worker::{SearchEvent, SearchWorker};
use crate::text::Position;
use std::sync::mpsc::TryRecvError;

#[derive(Default)]
pub enum Phase {
  #[default]
  Idle,
  Running {
    jump: bool,
  },
  Complete,
}

/// A pattern with its direction and word matching.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Query {
  pub text: String,
  pub backward: bool,
  pub whole: bool,
}

#[derive(Default)]
pub struct Search {
  pub worker: Option<SearchWorker>,
  pub generation: u64,
  pub phase: Phase,
  pub hits: Vec<Hit>,
  pub query: Query,
  pub hidden: bool,
  pub origin: Position,
  /// The search an open prompt replaces, and whether it was hidden.
  pub previous: Option<(Query, bool)>,
}

impl Search {
  pub fn cancel(&mut self) {
    if let Some(worker) = &self.worker {
      worker.cancel();
    }
    self.generation += 1;
    self.phase = Phase::Idle;
    self.hidden = true;
  }
}

impl App {
  pub fn start_search(
    &mut self,
    query: &str,
    backward: bool,
    whole: bool,
  ) -> Result<()> {
    if query.is_empty() {
      self.search.query.backward = backward;
      return self.next_result(false, 1);
    }
    search::compile(query, whole)?;
    let origin = self.search_origin.map_or(self.reader.cursor, |o| o.cursor);
    if self.search.worker.is_none() {
      let worker = SearchWorker::new(&self.path, &self.password);
      for (page, text) in self.doc.seeds() {
        worker.seed(page, text);
      }
      self.search.worker = Some(worker);
    }
    let Some(worker) = &self.search.worker else {
      return Ok(());
    };
    match worker.query(query, whole, origin.page, backward) {
      Ok(generation) => self.search.generation = generation,
      Err(error) => {
        self.search.worker = None;
        return Err(error);
      }
    }
    self.search.phase = Phase::Running { jump: true };
    self.search.hits.clear();
    self.search.query = Query { text: query.to_owned(), backward, whole };
    self.search.hidden = false;
    self.search.origin = origin;
    self.reader.message = "Searching…".into();
    Ok(())
  }

  pub fn search_updates(&mut self) -> Result<bool> {
    if self.search.worker.is_none()
      && matches!(self.search.phase, Phase::Running { .. })
    {
      let query = self.search.query.clone();
      self.rerun(&query)?;
    }
    let Some(worker) = &self.search.worker else { return Ok(false) };
    let mut events = Vec::new();
    loop {
      match worker.events.try_recv() {
        Ok(event) => events.push(event),
        Err(TryRecvError::Empty) => break,
        Err(TryRecvError::Disconnected) => {
          events.push(SearchEvent {
            generation: self.search.generation,
            hits: Vec::new(),
            done: true,
            error: Some("Search worker stopped".into()),
          });
          break;
        }
      }
    }
    if events.is_empty() {
      return Ok(false);
    }
    let mut changed = false;
    for event in events {
      if event.generation != self.search.generation {
        continue;
      }
      changed = true;
      // Each event holds one page's hits in order.
      if let Some(first) = event.hits.first().map(|hit| hit.first) {
        let at = self.search.hits.partition_point(|hit| hit.first < first);
        self.search.hits.splice(at..at, event.hits);
      }
      if let Some(error) = event.error {
        // The next search starts a fresh worker.
        self.search.worker = None;
        self.error = Some(error);
        self.search.phase = Phase::Complete;
        continue;
      }
      if matches!(self.search.phase, Phase::Running { jump: true })
        && let Some(target) = self.first_match(event.done)
      {
        // Typing previews matches without filling the jump list.
        if self.search_origin.is_some() {
          self.reader.cursor = target;
          self.reader.scroll = self.layout.pages[target.page.0].top;
          self.ensure_cursor()?;
        } else {
          self.go_to(target, true)?;
        }
        self.search.phase = Phase::Running { jump: false };
      }
      self.reader.message = format!(
        "{} matches{}",
        self.search.hits.len(),
        if event.done { "" } else { " · searching" }
      );
      if event.done {
        self.search.phase = Phase::Complete;
        if self.search.hits.is_empty() {
          self.return_to_origin()?;
        }
      }
    }
    self.refill_panel(PanelKind::Results)?;
    Ok(changed)
  }

  /// The first match past the search origin; once the scan is done,
  /// the first from the other end.
  fn first_match(&self, done: bool) -> Option<Position> {
    let (hits, origin) = (&self.search.hits, self.search.origin);
    let next = if self.search.query.backward {
      hits.iter().rfind(|h| h.first < origin).or(hits.last().filter(|_| done))
    } else {
      hits.iter().find(|h| h.first > origin).or(hits.first().filter(|_| done))
    };
    next.map(|hit| hit.first)
  }

  /// Searches the focused panel's rows, else the document.
  pub fn begin_search(&mut self, backward: bool) {
    if let Some(panel) = self.panel.as_mut().filter(|panel| panel.focused) {
      panel.begin_search(backward);
    } else {
      let location =
        Location { cursor: self.reader.cursor, scroll: self.reader.scroll };
      self.search_origin = Some(location);
      self.search.previous =
        Some((self.search.query.clone(), self.search.hidden));
    }
    self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
  }

  /// Searches as the query is typed, like Vim's `incsearch`.
  pub fn incremental(&mut self, query: &str) -> Result<()> {
    if let Some(panel) = self.row_search() {
      panel.preview(query);
      return Ok(());
    }
    let backward = matches!(
      self.prompt.as_ref().map(|p| p.kind),
      Some(PromptKind::Search(true))
    );
    if query.is_empty() || search::compile(query, false).is_err() {
      self.search.cancel();
      return self.return_to_origin();
    }
    self.start_search(query, backward, false)
  }

  /// Keeps the previewed match and records the start as a jump.
  pub fn finish_search(&mut self, query: &str, backward: bool) -> Result<()> {
    if let Some(panel) = self.row_search() {
      if !panel.finish_search(query) {
        anyhow::bail!("Pattern not found: {}", panel.search.query);
      }
      return Ok(());
    }
    let origin = self.search_origin.take();
    self.search.previous = None;
    let running = !matches!(self.search.phase, Phase::Idle);
    let same = Query { text: query.to_owned(), backward, whole: false };
    if !(running && self.search.query == same) {
      self.search_origin = origin;
      let result = self.start_search(query, backward, false);
      self.search_origin = None;
      result?;
    }
    if let Some(origin) = origin.filter(|o| o.cursor != self.reader.cursor) {
      self.reader.record(origin);
    }
    Ok(())
  }

  pub fn abort_search(&mut self) -> Result<()> {
    if let Some(panel) = self.row_search() {
      panel.abort_search();
      return Ok(());
    }
    let running = matches!(self.search.phase, Phase::Running { .. });
    self.search.cancel();
    self.return_to_origin()?;
    self.search_origin = None;
    let Some((query, hidden)) = self.search.previous.take() else {
      return Ok(());
    };
    if running || query != self.search.query {
      self.rerun(&query)?;
    }
    self.search.hidden = hidden;
    Ok(())
  }

  /// Runs `query` again without moving the cursor.
  pub fn rerun(&mut self, query: &Query) -> Result<()> {
    if query.text.is_empty() {
      self.search.hits.clear();
      self.search.query = Query::default();
      return Ok(());
    }
    self.start_search(&query.text, query.backward, query.whole)?;
    self.search.phase = Phase::Running { jump: false };
    Ok(())
  }

  fn row_search(&mut self) -> Option<&mut Panel> {
    self.panel.as_mut().filter(|panel| panel.searching())
  }

  fn return_to_origin(&mut self) -> Result<()> {
    let Some(origin) = self.search_origin else { return Ok(()) };
    self.reader.cursor = reader::clamp(origin.cursor, &mut self.doc)?;
    self.reader.scroll = origin.scroll;
    self.ensure_cursor()
  }

  pub fn next_result(&mut self, reverse: bool, count: usize) -> Result<()> {
    let hits = &self.search.hits;
    if hits.is_empty() {
      self.reader.message = "No matches".into();
      return Ok(());
    }
    let backward = self.search.query.backward != reverse;
    let cursor = self.reader.cursor;
    let start = if backward {
      hits.iter().rposition(|h| h.first < cursor).unwrap_or(hits.len() - 1)
    } else {
      hits.iter().position(|h| h.first > cursor).unwrap_or(0)
    };
    let offset = (count.saturating_sub(1)) % hits.len();
    let index = if backward {
      (start + hits.len() - offset) % hits.len()
    } else {
      (start + offset) % hits.len()
    };
    let target = hits[index].first;
    let wrapped = if backward { target >= cursor } else { target <= cursor };
    self.go_to(target, true)?;
    self.search.hidden = false;
    self.reader.message = if wrapped {
      "Search wrapped".into()
    } else {
      format!("Match {} / {}", index + 1, self.search.hits.len())
    };
    Ok(())
  }

  pub fn word_search(&mut self) -> Result<()> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let Some(line) = text.lines.get(self.reader.cursor.line) else {
      return Ok(());
    };
    let at = self.reader.cursor.glyph;
    if line.glyphs.get(at).is_none_or(|g| crate::text::class(g.ch) != 1) {
      return Ok(());
    }
    let Some((first, last)) = line.run(at) else { return Ok(()) };
    let query: String =
      line.glyphs[first..=last].iter().map(|g| g.ch).collect();
    self.start_search(&query, false, true)
  }
}
