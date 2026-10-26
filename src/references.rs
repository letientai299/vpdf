//! Ordered open targets owned by one pane.
use crate::Result;
use crate::app::App;
use anyhow::Context;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use vpdf_model::document::Target;
use vpdf_model::reference::Operation;
use vpdf_model::text::{Bounds, PageId, Position};

#[derive(Default)]
pub(crate) struct State {
  operations: VecDeque<Operation>,
  page: Option<PageId>,
  waiting: Option<(u64, u64)>,
  pub rectangle: Option<Rectangle>,
}

pub(crate) struct Rectangle {
  pub page: PageId,
  pub bounds: Bounds,
  pub until: Instant,
}

impl App {
  pub(crate) fn apply_reference(
    &mut self,
    operations: &[Operation],
  ) -> Result<()> {
    self.cancel_reference();
    if !operations.is_empty() && self.search.phase().running() {
      self.cancel_search();
    }
    self.reference = State {
      operations: operations.iter().cloned().collect(),
      page: Some(self.navigation_position().page),
      ..State::default()
    };
    self.error = None;
    let result = self.reference_operations();
    if let Err(error) = &result {
      self.cancel_reference();
      self.error = Some(error.to_string());
    }
    result
  }

  pub(crate) fn reference_pending(&self) -> bool {
    self.reference.waiting.is_some()
  }

  pub(crate) fn cancel_reference(&mut self) {
    if self.reference.waiting.take().is_some() {
      self.cancel_search();
    }
    self.reference.operations.clear();
  }

  fn reference_operations(&mut self) -> Result<()> {
    while let Some(operation) = self.reference.operations.pop_front() {
      let page =
        self.reference.page.unwrap_or_else(|| self.navigation_position().page);
      match operation {
        Operation::Page(number) => {
          let page = PageId(number.get() - 1);
          self.doc.validate(page)?;
          self.follow_reference(&Target::Page(page))?;
        }
        Operation::NamedDestination(name) => {
          let target = self.doc.named_destination(&name)?;
          self.follow_reference(&target)?;
        }
        Operation::Structure(id) => {
          let target = self.doc.structure_target(&id)?;
          self.follow_reference(&target)?;
        }
        Operation::Comment(name) => {
          let bounds = self.doc.comment_bounds(page, &name)?;
          if self.text_mode {
            self.follow_reference(&Target::Point {
              page,
              x: bounds.x0,
              y: bounds.y0,
            })?;
          } else {
            self.reference_rectangle(page, bounds)?;
          }
        }
        Operation::Highlight(values) => {
          let bounds = self.doc.highlight_bounds(page, values)?;
          self.reference_rectangle(page, bounds)?;
        }
        Operation::Search(words) => {
          let remaining = std::mem::take(&mut self.reference.operations);
          self.start_query_from(
            &vpdf_search::query::Query::words(&words),
            Some(Position::default()),
          )?;
          self.search.stop_jumping();
          self.reference.operations = remaining;
          self.reference.waiting =
            Some((self.search.generation(), self.document_generation));
          return Ok(());
        }
      }
    }
    Ok(())
  }

  fn follow_reference(&mut self, target: &Target) -> Result<()> {
    let page = match target {
      Target::Page(page) | Target::Point { page, .. } => *page,
      Target::Uri(_) => anyhow::bail!("External reference target unsupported"),
    };
    self.follow(target)?;
    if let Target::Point { x, y, .. } = *target
      && !self.text_mode
    {
      self
        .reading_view()
        .reveal_bounds(page, Bounds { x0: x, x1: x, y0: y, y1: y });
      self.reader.aim(target.clone());
    }
    self.reference.page = Some(page);
    Ok(())
  }

  fn reference_rectangle(
    &mut self,
    page: PageId,
    bounds: Bounds,
  ) -> Result<()> {
    anyhow::ensure!(!self.text_mode, "Highlight requires page view");
    let target = Target::Point { page, x: bounds.x0, y: bounds.y0 };
    self.follow_reference(&target)?;
    self.reading_view().reveal_bounds(page, bounds);
    self.reader.aim(target);
    self.reference.rectangle = Some(Rectangle {
      page,
      bounds,
      until: Instant::now() + Duration::from_secs(3),
    });
    Ok(())
  }

  pub(crate) fn reference_search(
    &mut self,
    generation: u64,
    done: bool,
  ) -> Result<bool> {
    let Some((expected, document)) = self.reference.waiting else {
      return Ok(false);
    };
    if expected != generation || document != self.document_generation {
      self.cancel_reference();
      return Ok(false);
    }
    if !done {
      return Ok(true);
    }
    self.reference.waiting = None;
    self.search.complete();
    let result = (|| {
      let position = self
        .search
        .hits()
        .first()
        .context("Reference search found no matches")?
        .first;
      self.go_to(position, true)?;
      self.reference.page = Some(position.page);
      self.search.land(position);
      self.reference_operations()
    })();
    if result.is_err() {
      self.cancel_reference();
    }
    result.map(|()| true)
  }

  pub(crate) fn expire_reference(&mut self, now: Instant) -> bool {
    if self.reference.rectangle.as_ref().is_some_and(|r| now >= r.until) {
      self.reference.rectangle = None;
      return true;
    }
    false
  }
}
