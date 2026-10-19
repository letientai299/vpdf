use crate::Result;
use crate::app::App;
use crate::keys::PanelKind;
use crate::panel::Panel;
use crate::pdf_text::PdfText;
use crate::prompt::{Prompt, PromptKind};
use crate::search;
use crate::search_worker::SearchWorker;

mod state;

use state::Update;
pub use state::{Phase, Search};
pub use vpdf_search::query::Query;

impl App {
  pub fn start_search(
    &mut self,
    query: &str,
    backward: bool,
    whole: bool,
  ) -> Result<()> {
    if query.is_empty() {
      self.search.set_backward(backward);
      return self.next_result(false, 1);
    }
    self.start_query(&Query {
      text: query.to_owned(),
      backward,
      whole,
      reference: None,
    })
  }

  pub(crate) fn start_query(&mut self, query: &Query) -> Result<()> {
    search::compile(&query.text, query.whole)?;
    let origin = match self.search_origin {
      Some(origin) => origin.cursor,
      None => self.reading_position()?,
    };
    let worker = self.search_worker.get_or_insert_with(|| {
      let worker =
        SearchWorker::new(&self.path, &self.password, self.doc.pages.len());
      for (page, text) in self.doc.seeds() {
        worker.seed(page, text);
      }
      worker
    });
    let generation = match worker.lookup(query, origin.page) {
      Ok(generation) => generation,
      Err(error) => {
        self.search_worker = None;
        return Err(error);
      }
    };
    self.search.begin(query.clone(), generation, origin);
    self.reader.message = "Searching…".into();
    Ok(())
  }

  pub fn search_updates(&mut self) -> Result<bool> {
    if self.search_worker.is_none() && self.search.phase().running() {
      let query = self.search.query().clone();
      let definition = matches!(self.search.phase(), Phase::Definition);
      self.rerun(&query)?;
      if definition {
        self.search.resume_definition();
      }
    }
    let Some(worker) = &self.search_worker else { return Ok(false) };
    let events = worker.updates(self.search.generation());
    if events.is_empty() {
      return Ok(false);
    }
    let mut changed = false;
    for event in events {
      let Some(update) = self.search.accept(event) else { continue };
      changed = true;
      let done = match update {
        Update::Progress { done } => done,
        Update::Failed(error) => {
          // The next search starts a fresh worker.
          self.search_worker = None;
          self.error = Some(error);
          continue;
        }
      };
      if let Some(target) = self.search.preview(done) {
        // Typing previews matches without filling the jump list; each
        // judges the view the search started from, as Vim's incsearch.
        if let Some(origin) = self.search_origin {
          self.restore_location(origin)?;
          self.reader.cursor = target;
          self.target = Some(target);
          self.land()?;
        } else {
          self.go_to(target, true)?;
          self
            .reader
            .describe_jump(Some(self.search_description(self.reader.cursor)));
        }
        self.search.stop_jumping();
      }
      self.reader.message = format!(
        "{} matches{}",
        self.search.hits().len(),
        if done { "" } else { " · searching" }
      );
      if done {
        if self.search.complete() {
          self.finish_definition()?;
        }
        self
          .reader
          .finish_search_jumps(self.search.query(), self.search.hits());
        self.refill_panel(PanelKind::Jumps)?;
        if self.search.hits().is_empty() {
          self.return_to_origin()?;
        }
      }
    }
    self.finish_search_yank()?;
    self.refill_panel(PanelKind::Results)?;
    Ok(changed)
  }

  /// Searches the focused panel's rows, else the document.
  pub fn begin_search(&mut self, backward: bool) -> Result<()> {
    if let Some(panel) = self.panel.as_mut().filter(|panel| panel.focused) {
      panel.begin_search(backward);
    } else {
      let location = self.location()?;
      self.search_origin = Some(location);
      self.search.remember_previous();
    }
    self.prompt = Some(Prompt::new(PromptKind::Search(backward)));
    Ok(())
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
      self.cancel_search();
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
    if let Some(operator) = self.reader.operator.take() {
      let origin = self.search_origin.take().unwrap_or(self.location()?);
      self.search.forget_previous();
      let query = if query.is_empty() {
        self.search.query().text.clone()
      } else {
        query.to_owned()
      };
      self.restore_location(origin)?;
      search::compile(&query, false)?;
      if query.is_empty() {
        return Ok(());
      }
      let same = Query { text: query.clone(), backward, ..Query::default() };
      if matches!(self.search.phase(), Phase::Idle)
        || self.search.query() != &same
      {
        self.start_search(&query, backward, false)?;
      }
      self.search.stop_jumping();
      self.search.yank_after(origin, operator);
      return self.finish_search_yank();
    }
    let origin = self.search_origin.take();
    self.search.forget_previous();
    let running = !matches!(self.search.phase(), Phase::Idle);
    let same = Query { text: query.to_owned(), backward, ..Query::default() };
    if !(running && self.search.query() == &same) {
      self.search_origin = origin;
      let result = self.start_search(query, backward, false);
      self.search_origin = None;
      result?;
    }
    if let Some(origin) = origin.filter(|o| o.cursor != self.reader.cursor) {
      self.reader.record(origin);
    }
    if self.search.query() == &same
      && matches!(
        self.search.phase(),
        Phase::Running { jump: false } | Phase::Complete
      )
      && self
        .search
        .hits()
        .binary_search_by_key(&self.reader.cursor, |hit| hit.first)
        .is_ok()
    {
      self
        .reader
        .describe_jump(Some(self.search_description(self.reader.cursor)));
      self.flash_landing();
    }
    Ok(())
  }

  fn finish_search_yank(&mut self) -> Result<()> {
    let Some((origin, operator, target)) = self.search.take_yank() else {
      return Ok(());
    };
    if target != origin.cursor {
      let first = target.min(origin.cursor);
      self.reader.cursor = target.max(origin.cursor);
      if self.reader.step(-1, false, &mut PdfText(&mut self.doc))? {
        self.reader.yank_range(
          first,
          self.reader.cursor,
          false,
          operator.register,
          &mut PdfText(&mut self.doc),
        )?;
      }
    }
    self.restore_location(origin)
  }

  pub fn abort_search(&mut self) -> Result<()> {
    if let Some(panel) = self.row_search() {
      panel.abort_search();
      return Ok(());
    }
    let running = matches!(self.search.phase(), Phase::Running { .. });
    self.cancel_search();
    self.return_to_origin()?;
    self.search_origin = None;
    let Some((query, hidden)) = self.search.take_previous() else {
      return Ok(());
    };
    if running || &query != self.search.query() {
      self.rerun(&query)?;
    }
    self.search.set_hidden(hidden);
    Ok(())
  }

  /// Runs `query` again without moving the cursor.
  pub fn rerun(&mut self, query: &Query) -> Result<()> {
    if query.text.is_empty() {
      self.search.clear();
      return Ok(());
    }
    self.start_query(query)?;
    self.search.stop_jumping();
    Ok(())
  }

  /// Stops the scan and hides its hits.
  pub(crate) fn cancel_search(&mut self) {
    if let Some(worker) = &self.search_worker {
      worker.cancel();
    }
    self.search.cancel();
  }

  fn row_search(&mut self) -> Option<&mut Panel> {
    self.panel.as_mut().filter(|panel| panel.searching())
  }

  fn return_to_origin(&mut self) -> Result<()> {
    let Some(origin) = self.search_origin else { return Ok(()) };
    self.restore_location(origin)
  }

  pub fn next_result(&mut self, reverse: bool, count: usize) -> Result<()> {
    let cursor = self.reading_position()?;
    let hits = self.search.hits();
    let backward = self.search.query().backward != reverse;
    let Some(index) =
      search::next(hits, &cursor, backward, count, |hit| hit.first)
    else {
      self.reader.message = "No matches".into();
      return Ok(());
    };
    let target = hits[index].first;
    let wrapped = if backward { target >= cursor } else { target <= cursor };
    self.go_to(target, true)?;
    self
      .reader
      .describe_jump(Some(self.search_description(self.reader.cursor)));
    self.search.set_hidden(false);
    self.reader.message = if wrapped {
      "Search wrapped".into()
    } else {
      format!("Match {} / {}", index + 1, self.search.hits().len())
    };
    Ok(())
  }

  pub fn word_search(&mut self) -> Result<()> {
    let Some(cursor) = self.explicit_target()? else { return Ok(()) };
    let text = self.doc.text(cursor.page)?;
    let Some(line) = text.lines.get(cursor.line) else {
      return Ok(());
    };
    let at = cursor.glyph;
    if line.glyphs.get(at).is_none_or(|g| crate::text::class(g.ch) != 1) {
      return Ok(());
    }
    let Some((first, last)) = line.run(at) else { return Ok(()) };
    let query: String =
      line.glyphs[first..=last].iter().map(|g| g.ch).collect();
    self.start_search(&query, false, true)
  }
}
