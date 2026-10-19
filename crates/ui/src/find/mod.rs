use crate::text::Position;
use vpdf_reader::reader::{JumpDescription, Location, Operator};
use vpdf_search::query::Query;
use vpdf_search::search::{self, Hit};

mod definition;
pub use definition::Definition;
#[cfg(test)]
mod definition_tests;

#[derive(Default)]
pub enum Phase {
  #[default]
  Idle,
  Running {
    jump: bool,
  },
  Definition,
  Complete,
}

impl Phase {
  pub fn running(&self) -> bool {
    matches!(self, Self::Running { .. } | Self::Definition)
  }
}

/// What an accepted worker event means for the pane.
#[derive(Debug, Eq, PartialEq)]
pub enum Update {
  Progress {
    done: bool,
  },
  /// The worker failed; the next search needs a fresh one.
  Failed(String),
}

/// A pane's document search: the query, its scan, and the hits so far.
#[derive(Default)]
pub struct Search {
  generation: u64,
  phase: Phase,
  hits: Vec<Hit>,
  query: Query,
  hidden: bool,
  origin: Position,
  /// The search an open prompt replaces, and whether it was hidden.
  previous: Option<(Query, bool)>,
  prompt_origin: Option<Location>,
  yank: Option<(Location, Operator)>,
}

impl Search {
  pub fn prompt_origin(&self) -> Option<Location> {
    self.prompt_origin
  }

  pub(crate) fn begin_prompt(&mut self, origin: Location) {
    self.prompt_origin = Some(origin);
    self.remember_previous();
  }

  pub(crate) fn take_prompt_origin(&mut self) -> Option<Location> {
    self.prompt_origin.take()
  }

  pub(crate) fn relocate_prompt(&mut self, origin: Location) {
    self.prompt_origin = Some(origin);
  }

  /// Reload keeps the open prompt's location.
  pub fn reset_document(&mut self) -> Option<Query> {
    let previous = std::mem::take(self);
    self.prompt_origin = previous.prompt_origin;
    (!previous.hidden && !previous.query.text.is_empty())
      .then_some(previous.query)
  }

  pub fn description(&self, position: Position) -> JumpDescription {
    let mut description =
      JumpDescription::Search { query: self.query.clone(), ordinal: None };
    if matches!(self.phase, Phase::Complete) {
      description.finish_search(&self.query, &self.hits, position);
    }
    description
  }

  pub fn generation(&self) -> u64 {
    self.generation
  }

  pub fn phase(&self) -> &Phase {
    &self.phase
  }

  pub fn hits(&self) -> &[Hit] {
    &self.hits
  }

  pub fn query(&self) -> &Query {
    &self.query
  }

  pub fn hidden(&self) -> bool {
    self.hidden
  }

  pub fn origin(&self) -> Position {
    self.origin
  }

  /// Starts showing scan `generation` of `query` from `origin`.
  pub fn begin(&mut self, query: Query, generation: u64, origin: Position) {
    self.generation = generation;
    self.phase = Phase::Running { jump: true };
    self.hits.clear();
    self.query = query;
    self.hidden = false;
    self.origin = origin;
  }

  /// Installs finished `hits` for `query`, for tests and benches; the
  /// caller stops any running scan.
  #[doc(hidden)]
  pub fn show(&mut self, query: Query, hits: Vec<Hit>) {
    self.generation += 1;
    self.phase = Phase::Complete;
    self.hits = hits;
    self.query = query;
    self.hidden = false;
  }

  /// Ignores the running scan and hides its hits.
  pub fn cancel(&mut self) {
    self.yank = None;
    self.generation += 1;
    self.phase = Phase::Idle;
    self.hidden = true;
  }

  pub fn clear(&mut self) {
    self.hits.clear();
    self.query = Query::default();
  }

  pub fn set_hidden(&mut self, hidden: bool) {
    self.hidden = hidden;
  }

  pub fn set_backward(&mut self, backward: bool) {
    self.query.backward = backward;
  }

  /// Stops the running scan from moving the cursor.
  pub fn stop_jumping(&mut self) {
    if let Phase::Running { jump } = &mut self.phase {
      *jump = false;
    }
  }

  /// Marks the scan as a reference lookup from `origin`.
  pub fn define(&mut self, origin: Position) {
    self.origin = origin;
    self.phase = Phase::Definition;
  }

  /// Restores the reference-lookup phase after a rerun.
  pub fn resume_definition(&mut self) {
    self.phase = Phase::Definition;
  }

  /// Ends the scan; true when it was a reference lookup.
  pub fn complete(&mut self) -> bool {
    let definition = matches!(self.phase, Phase::Definition);
    self.phase = Phase::Complete;
    definition
  }

  pub(crate) fn remember_previous(&mut self) {
    self.previous = Some((self.query.clone(), self.hidden));
  }

  pub(crate) fn forget_previous(&mut self) {
    self.previous = None;
  }

  pub(crate) fn take_previous(&mut self) -> Option<(Query, bool)> {
    self.previous.take()
  }

  pub fn yank_after(&mut self, origin: Location, operator: Operator) {
    self.yank = Some((origin, operator));
  }

  pub fn forget_yank(&mut self) {
    self.yank = None;
  }

  /// The pending search yank once the scan completes, with the hit it
  /// reaches; the yank ends either way.
  pub fn take_yank(&mut self) -> Option<(Location, Operator, Position)> {
    if !matches!(self.phase, Phase::Complete) {
      return None;
    }
    let (origin, operator) = self.yank.take()?;
    let index = search::next(
      &self.hits,
      &origin.cursor,
      self.query.backward,
      operator.count,
      |hit| hit.first,
    )?;
    Some((origin, operator, self.hits[index].first))
  }

  /// Merges ordered hits for the current scan.
  pub fn accept(
    &mut self,
    generation: u64,
    hits: Vec<Hit>,
    completion: Result<bool, String>,
  ) -> Option<Update> {
    if generation != self.generation {
      return None;
    }
    if let Some(first) = hits.first().map(|hit| hit.first) {
      let at = self.hits.partition_point(|hit| hit.first < first);
      self.hits.splice(at..at, hits);
    }
    match completion {
      Ok(done) => Some(Update::Progress { done }),
      Err(error) => {
        self.yank = None;
        self.phase = Phase::Complete;
        Some(Update::Failed(error))
      }
    }
  }

  /// The match a typed query previews: the first past the origin; once
  /// the scan is done, the first from the other end.
  pub fn preview(&self, done: bool) -> Option<Position> {
    if !matches!(self.phase, Phase::Running { jump: true }) {
      return None;
    }
    let (hits, origin) = (&self.hits, self.origin);
    let next = if self.query.backward {
      hits.iter().rfind(|h| h.first < origin).or(hits.last().filter(|_| done))
    } else {
      hits.iter().find(|h| h.first > origin).or(hits.first().filter(|_| done))
    };
    next.map(|hit| hit.first)
  }
}

#[cfg(test)]
mod tests;
