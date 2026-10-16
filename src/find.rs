use crate::Result;
use crate::app::App;
use crate::keys::PanelKind;
use crate::prompt::{Prompt, PromptKind};
use crate::reader::Location;
use crate::search::{self, Hit};
use crate::search_worker::SearchWorker;
use crate::text::Position;

#[derive(Default)]
pub enum Phase {
  #[default]
  Idle,
  Running {
    jump: bool,
  },
  Complete,
}

#[derive(Default)]
pub struct Search {
  pub worker: Option<SearchWorker>,
  pub generation: u64,
  pub phase: Phase,
  pub hits: Vec<Hit>,
  pub query: String,
  pub backward: bool,
  pub hidden: bool,
  pub origin: Position,
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
    self.search.generation = worker.query(query, whole, origin.page, backward);
    self.search.phase = Phase::Running { jump: true };
    self.search.hits.clear();
    self.search.query = query.to_owned();
    self.search.backward = backward;
    self.search.hidden = false;
    self.search.origin = origin;
    self.reader.message = "Searching…".into();
    Ok(())
  }

  pub fn search_updates(&mut self) -> Result<bool> {
    let Some(worker) = &self.search.worker else { return Ok(false) };
    let events: Vec<_> = worker.events.try_iter().collect();
    if events.is_empty() {
      return Ok(false);
    }
    let mut changed = false;
    for event in events {
      if event.generation != self.search.generation {
        continue;
      }
      changed = true;
      self.search.hits.extend(event.hits);
      self.search.hits.sort_unstable_by_key(|hit| hit.first);
      if let Some(error) = event.error {
        self.error = Some(error);
        self.search.phase = Phase::Complete;
        continue;
      }
      if matches!(self.search.phase, Phase::Running { jump: true }) {
        let next = if self.search.backward {
          self.search.hits.iter().rfind(|h| h.first < self.search.origin)
        } else {
          self.search.hits.iter().find(|h| h.first > self.search.origin)
        };
        if let Some(hit) = next {
          // Typing previews matches without filling the jump list.
          if self.search_origin.is_some() {
            self.reader.cursor = hit.first;
          } else {
            self.reader.jump(hit.first, &mut self.doc)?;
          }
          self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
          self.ensure_cursor()?;
          self.search.phase = Phase::Running { jump: false };
        } else if event.done && !self.search.hits.is_empty() {
          self.next_result(false, 1)?;
        }
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
    if self.panel.as_ref().is_some_and(|p| p.kind == PanelKind::Results) {
      let rows = self.panel_rows(PanelKind::Results)?;
      if let Some(panel) = &mut self.panel {
        panel.rows = rows;
        panel.selected = panel.selected.min(panel.rows.len().saturating_sub(1));
      }
    }
    Ok(changed)
  }

  pub fn begin_search(&mut self, backward: bool) {
    let location =
      Location { cursor: self.reader.cursor, scroll: self.reader.scroll };
    self.search_origin = Some(location);
    self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
  }

  /// Searches as the query is typed, like Vim's `incsearch`.
  pub fn incremental(&mut self, query: &str) -> Result<()> {
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
    let origin = self.search_origin.take();
    let running = !matches!(self.search.phase, Phase::Idle);
    if !(running && self.search.query == query) {
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
    self.search.cancel();
    self.return_to_origin()?;
    self.search_origin = None;
    Ok(())
  }

  fn return_to_origin(&mut self) -> Result<()> {
    let Some(origin) = self.search_origin else { return Ok(()) };
    self.reader.cursor = origin.cursor;
    self.reader.scroll = origin.scroll;
    self.ensure_cursor()
  }

  pub fn next_result(&mut self, reverse: bool, count: usize) -> Result<()> {
    let hits = &self.search.hits;
    if hits.is_empty() {
      self.reader.message = "No matches".into();
      return Ok(());
    }
    let backward = self.search.backward != reverse;
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
    self.reader.jump(target, &mut self.doc)?;
    self.reader.scroll = self.layout.pages[target.page.0].top;
    self.ensure_cursor()?;
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
    let mut first = at;
    let mut last = at;
    while first > 0 && crate::text::class(line.glyphs[first - 1].ch) == 1 {
      first -= 1;
    }
    while last + 1 < line.glyphs.len()
      && crate::text::class(line.glyphs[last + 1].ch) == 1
    {
      last += 1;
    }
    let query: String =
      line.glyphs[first..=last].iter().map(|g| g.ch).collect();
    self.start_search(&query, false, true)
  }
}
