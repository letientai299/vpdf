//! Search: the query prompt, worker results, and definitions.

use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use vpdf_pdf::jobs::search::SearchWorker;
use vpdf_search::query::Query;
use vpdf_search::search;
use vpdf_ui::find::{Definition, Phase, Update};
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::Panel;
use vpdf_ui::prompt::{Prompt, PromptKind};

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
    self.start_query_from(query, self.search.prompt_origin().map(|o| o.cursor))
  }

  pub(crate) fn start_query_from(
    &mut self,
    query: &Query,
    origin: Option<vpdf_model::text::Position>,
  ) -> Result<()> {
    self.cancel_reference();
    search::compile(&query.text, query.whole)?;
    let origin = match origin {
      Some(origin) => origin,
      None => self.reading_position()?,
    };
    let worker = self.search_worker.get_or_insert_with(|| {
      let worker = SearchWorker::new(
        &self.path,
        &self.password,
        self.doc.pages.len(),
        &self.wake,
      );
      for (page, text) in self.doc.seeds() {
        worker.seed(page, text);
      }
      worker
    });
    let generation = match worker.lookup(query, origin.page) {
      Ok(generation) => generation,
      Err(error) => {
        self.track_jobs();
        self.search_worker = None;
        return Err(error);
      }
    };
    self.search.begin(query.clone(), generation, origin);
    self.refresh_panel_note(PanelKind::Results);
    self.reader.notice("Searching…");
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
      let completion = event.error.map_or(Ok(event.done), Err);
      let Some(update) =
        self.search.accept(event.generation, event.hits, completion)
      else {
        continue;
      };
      changed = true;
      let done = match update {
        Update::Progress { done } => done,
        Update::Failed(error) => {
          // The next search starts a fresh worker.
          self.track_jobs();
          self.search_worker = None;
          self.cancel_reference();
          self.error = Some(error);
          continue;
        }
      };
      if self.reference_search(event.generation, done)? {
        continue;
      }
      let mut reading = self.reading_parts();
      reading.view.preview_search(reading.search, done)?;
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
    if let Some(panel) =
      self.panel.as_mut().filter(|panel| panel.view.focused())
    {
      panel.view.begin_search(backward);
    } else {
      let mut reading = self.reading_parts();
      reading.view.begin_search(reading.search)?;
    }
    self.overlays.open_prompt(Prompt::new(PromptKind::Search(backward)));
    Ok(())
  }

  /// Searches as the query is typed, like Vim's `incsearch`.
  pub fn incremental(&mut self, query: &str) -> Result<()> {
    if let Some((panel, outline)) = self.row_search() {
      if panel.kind() == PanelKind::Outline {
        outline.preview_search(panel, query);
      } else {
        panel.preview(query);
      }
      return Ok(());
    }
    let backward = matches!(
      self.overlays.prompt().map(Prompt::kind),
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
    if let Some((panel, outline)) = self.row_search() {
      let found = if panel.kind() == PanelKind::Outline {
        outline.finish_search(panel, query)
      } else {
        panel.finish_search(query)
      };
      if !found {
        anyhow::bail!("Pattern not found: {}", panel.query());
      }
      return Ok(());
    }
    let mut reading = self.reading_parts();
    let submission =
      reading.view.prepare_search(reading.search, query, backward)?;
    if let Some(query) = submission.query() {
      self.start_query_from(query, submission.origin())?;
    }
    let mut reading = self.reading_parts();
    reading.view.finish_search(reading.search, submission)?;
    self.mark_landed();
    Ok(())
  }

  fn finish_search_yank(&mut self) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.finish_search_yank(reading.search)
  }

  pub fn abort_search(&mut self) -> Result<()> {
    if let Some((panel, outline)) = self.row_search() {
      if panel.kind() == PanelKind::Outline {
        outline.abort_search(panel);
      } else {
        panel.abort_search();
      }
      return Ok(());
    }
    let running = matches!(self.search.phase(), Phase::Running { .. });
    self.cancel_search();
    let mut reading = self.reading_parts();
    let Some(restore) = reading.view.abort_search(reading.search, running)?
    else {
      return Ok(());
    };
    if let Some(query) = restore.query {
      self.rerun(&query)?;
    }
    self.search.set_hidden(restore.hidden);
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
    self.refresh_panel_note(PanelKind::Results);
  }

  fn row_search(
    &mut self,
  ) -> Option<(&mut Panel, &mut vpdf_ui::reading::Outline)> {
    let panel = &mut self.panel.as_mut()?.view;
    panel.searching().then_some((panel, &mut self.outline))
  }

  fn return_to_origin(&mut self) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.return_to_origin(reading.search)
  }

  pub fn next_result(&mut self, reverse: bool, count: usize) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.next_result(reading.search, reverse, count)?;
    self.mark_landed();
    Ok(())
  }

  pub fn word_search(&mut self) -> Result<()> {
    let Some(query) = self.reading_view().word_query()? else { return Ok(()) };
    self.start_search(&query, false, true)
  }
}

impl App {
  pub(crate) fn cancel_definition(&mut self) {
    if matches!(self.search.phase(), Phase::Definition) {
      self.cancel_search();
    }
  }

  pub(crate) fn definition(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    self.search.stop_jumping();
    let link = self.link_target(position)?;
    let definition =
      Definition::resolve(position, link, &mut PdfText(&mut self.doc))?;
    self.apply_definition(definition)
  }

  pub(crate) fn finish_definition(&mut self) -> Result<()> {
    self.apply_definition(self.search.definition_result())
  }

  fn apply_definition(&mut self, definition: Definition) -> Result<()> {
    let message = definition.message();
    match definition {
      Definition::Follow(target) => self.follow(&target)?,
      Definition::Outline(selected) => {
        self.open_panel(PanelKind::Outline)?;
        if let Some(panel) = &mut self.panel {
          panel.view.select(selected);
        }
      }
      Definition::Search(lookup) => {
        let origin = lookup.origin;
        self.start_query(&lookup.into())?;
        self.search.define(origin);
      }
      Definition::Found { position, description } => {
        self.go_to(position, true)?;
        self.reader.describe_jump(Some(description));
      }
      Definition::Candidates => self.open_panel(PanelKind::Results)?,
      Definition::Unchanged
      | Definition::External
      | Definition::NoReference
      | Definition::NotFound => {}
    }
    if let Some(message) = message {
      self.reader.notice(message);
    }
    Ok(())
  }
}
