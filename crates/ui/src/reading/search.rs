use super::View;
use crate::Result;
use crate::find::{Phase, Search};
use crate::source::Pages;
use crate::text::Position;
use vpdf_reader::reader::{Location, Operator, TextSource};
use vpdf_search::{query::Query, search};

pub struct Submission {
  kind: Commit,
}

enum Commit {
  Done,
  Yank { query: Option<Query>, origin: Location, operator: Operator },
  Jump { query: Query, origin: Option<Location>, start: bool },
}

impl Submission {
  pub fn query(&self) -> Option<&Query> {
    match &self.kind {
      Commit::Done => None,
      Commit::Yank { query, .. } => query.as_ref(),
      Commit::Jump { query, start, .. } => start.then_some(query),
    }
  }

  pub fn origin(&self) -> Option<Position> {
    match self.kind {
      Commit::Jump { origin, .. } => origin.map(|origin| origin.cursor),
      _ => None,
    }
  }
}

pub struct RestoreSearch {
  pub query: Option<Query>,
  pub hidden: bool,
}

impl<S: TextSource + Pages> View<'_, S> {
  pub fn begin_search(&mut self, search: &mut Search) -> Result<()> {
    search.begin_prompt(self.location()?);
    Ok(())
  }

  pub fn prepare_search(
    &mut self,
    search: &mut Search,
    query: &str,
    backward: bool,
  ) -> Result<Submission> {
    if let Some(operator) = self.reader.operator.take() {
      let origin = search.take_prompt_origin().unwrap_or(self.location()?);
      search.forget_previous();
      let query = if query.is_empty() {
        search.query().text.clone()
      } else {
        query.to_owned()
      };
      self.restore_location(origin)?;
      search::compile(&query, false)?;
      if query.is_empty() {
        return Ok(Submission { kind: Commit::Done });
      }
      let query = Query { text: query, backward, ..Query::default() };
      let start =
        matches!(search.phase(), Phase::Idle) || search.query() != &query;
      return Ok(Submission {
        kind: Commit::Yank { query: start.then_some(query), origin, operator },
      });
    }
    let origin = search.take_prompt_origin();
    search.forget_previous();
    let running = !matches!(search.phase(), Phase::Idle);
    let query = Query { text: query.to_owned(), backward, ..Query::default() };
    let mut start = !(running && search.query() == &query);
    if start && query.text.is_empty() {
      search.set_backward(backward);
      self.next_result(search, false, 1)?;
      start = false;
    }
    Ok(Submission { kind: Commit::Jump { query, origin, start } })
  }

  /// Apply only after the requested scan starts.
  pub fn finish_search(
    &mut self,
    search: &mut Search,
    submission: Submission,
  ) -> Result<()> {
    match submission.kind {
      Commit::Done => {}
      Commit::Yank { origin, operator, .. } => {
        search.stop_jumping();
        search.yank_after(origin, operator);
        self.finish_search_yank(search)?;
      }
      Commit::Jump { query, origin, .. } => {
        if let Some(origin) = origin.filter(|o| o.cursor != self.reader.cursor)
        {
          self.reader.record(origin);
        }
        if search.query() == &query
          && matches!(
            search.phase(),
            Phase::Running { jump: false } | Phase::Complete
          )
          && search
            .hits()
            .binary_search_by_key(&self.reader.cursor, |hit| hit.first)
            .is_ok()
        {
          self
            .reader
            .describe_jump(Some(search.description(self.reader.cursor)));
          self.flash_landing();
        }
      }
    }
    Ok(())
  }

  pub fn abort_search(
    &mut self,
    search: &mut Search,
    interrupted: bool,
  ) -> Result<Option<RestoreSearch>> {
    self.return_to_origin(search)?;
    search.take_prompt_origin();
    let Some((query, hidden)) = search.take_previous() else {
      return Ok(None);
    };
    let rerun = interrupted || &query != search.query();
    Ok(Some(RestoreSearch { query: rerun.then_some(query), hidden }))
  }

  pub fn return_to_origin(&mut self, search: &Search) -> Result<()> {
    let Some(origin) = search.prompt_origin() else { return Ok(()) };
    self.restore_location(origin)
  }

  pub fn reclamp_search(&mut self, search: &mut Search) -> Result<()> {
    if let Some(mut origin) = search.prompt_origin() {
      origin.cursor =
        vpdf_reader::reader::clamp(origin.cursor, &mut self.source)?;
      origin.anchor =
        origin.anchor.filter(|(page, _)| page.0 < self.source.count());
      if let Some(top) = origin.text_top {
        origin.text_top = Some(self.text().clamp(top)?);
      }
      origin.target = None;
      search.relocate_prompt(origin);
    }
    Ok(())
  }

  pub fn preview_search(
    &mut self,
    search: &mut Search,
    done: bool,
  ) -> Result<()> {
    if let Some(target) = search.preview(done) {
      if let Some(origin) = search.prompt_origin() {
        self.preview(origin, target)?;
      } else {
        self.land_at(target)?;
        self.reader.describe_jump(Some(search.description(self.reader.cursor)));
      }
      search.stop_jumping();
    }
    self.reader.message = format!(
      "{} matches{}",
      search.hits().len(),
      if done { "" } else { " · searching" }
    );
    Ok(())
  }

  pub fn next_result(
    &mut self,
    search: &mut Search,
    reverse: bool,
    count: usize,
  ) -> Result<()> {
    let cursor = self.position()?;
    let hits = search.hits();
    let backward = search.query().backward != reverse;
    let Some(index) =
      search::next(hits, &cursor, backward, count, |hit| hit.first)
    else {
      self.reader.message = "No matches".into();
      return Ok(());
    };
    let target = hits[index].first;
    let wrapped = if backward { target >= cursor } else { target <= cursor };
    self.land_at(target)?;
    self.reader.describe_jump(Some(search.description(self.reader.cursor)));
    search.set_hidden(false);
    self.reader.message = if wrapped {
      "Search wrapped".into()
    } else {
      format!("Match {} / {}", index + 1, search.hits().len())
    };
    Ok(())
  }

  pub fn word_query(&mut self) -> Result<Option<String>> {
    let Some(cursor) = self.explicit_target()? else { return Ok(None) };
    let text = self.source.page_text(cursor.page)?;
    let Some(line) = text.lines.get(cursor.line) else { return Ok(None) };
    let at = cursor.glyph;
    if line.glyphs.get(at).is_none_or(|g| crate::text::class(g.ch) != 1) {
      return Ok(None);
    }
    let Some((first, last)) = line.run(at) else { return Ok(None) };
    Ok(Some(line.glyphs[first..=last].iter().map(|g| g.ch).collect()))
  }
}
