use crate::search::{self, Hit};
use crate::search_worker::SearchEvent;
use crate::text::Position;
use vpdf_reader::reader::{Location, Operator};
use vpdf_search::query::Query;

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
  pub(crate) fn running(&self) -> bool {
    matches!(self, Self::Running { .. } | Self::Definition)
  }
}

/// What an accepted worker event means for the pane.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Update {
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
  yank: Option<(Location, Operator)>,
}

impl Search {
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
  pub(super) fn begin(
    &mut self,
    query: Query,
    generation: u64,
    origin: Position,
  ) {
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
  pub(super) fn cancel(&mut self) {
    self.yank = None;
    self.generation += 1;
    self.phase = Phase::Idle;
    self.hidden = true;
  }

  pub(super) fn clear(&mut self) {
    self.hits.clear();
    self.query = Query::default();
  }

  pub(crate) fn set_hidden(&mut self, hidden: bool) {
    self.hidden = hidden;
  }

  pub(super) fn set_backward(&mut self, backward: bool) {
    self.query.backward = backward;
  }

  /// Stops the running scan from moving the cursor.
  pub(crate) fn stop_jumping(&mut self) {
    if let Phase::Running { jump } = &mut self.phase {
      *jump = false;
    }
  }

  /// Marks the scan as a reference lookup from `origin`.
  pub(crate) fn define(&mut self, origin: Position) {
    self.origin = origin;
    self.phase = Phase::Definition;
  }

  /// Restores the reference-lookup phase after a rerun.
  pub(super) fn resume_definition(&mut self) {
    self.phase = Phase::Definition;
  }

  /// Ends the scan; true when it was a reference lookup.
  pub(super) fn complete(&mut self) -> bool {
    let definition = matches!(self.phase, Phase::Definition);
    self.phase = Phase::Complete;
    definition
  }

  pub(super) fn remember_previous(&mut self) {
    self.previous = Some((self.query.clone(), self.hidden));
  }

  pub(super) fn forget_previous(&mut self) {
    self.previous = None;
  }

  pub(super) fn take_previous(&mut self) -> Option<(Query, bool)> {
    self.previous.take()
  }

  pub(super) fn yank_after(&mut self, origin: Location, operator: Operator) {
    self.yank = Some((origin, operator));
  }

  pub(crate) fn forget_yank(&mut self) {
    self.yank = None;
  }

  /// The pending search yank once the scan completes, with the hit it
  /// reaches; the yank ends either way.
  pub(super) fn take_yank(&mut self) -> Option<(Location, Operator, Position)> {
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

  /// Merges `event` when it belongs to the current scan.
  pub(super) fn accept(&mut self, event: SearchEvent) -> Option<Update> {
    if event.generation != self.generation {
      return None;
    }
    // Each event holds one page's hits in order.
    if let Some(first) = event.hits.first().map(|hit| hit.first) {
      let at = self.hits.partition_point(|hit| hit.first < first);
      self.hits.splice(at..at, event.hits);
    }
    if let Some(error) = event.error {
      self.yank = None;
      self.phase = Phase::Complete;
      return Some(Update::Failed(error));
    }
    Some(Update::Progress { done: event.done })
  }

  /// The match a typed query previews: the first past the origin; once
  /// the scan is done, the first from the other end.
  pub(super) fn preview(&self, done: bool) -> Option<Position> {
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
