use crate::Result;
use crate::cache::Cache;
use crate::document::{Document, MIB};
use crate::text::{PageId, Position, Text};
use std::sync::Arc;

pub trait TextSource {
  fn page_count(&self) -> usize;
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>>;
}

impl TextSource for Document {
  fn page_count(&self) -> usize {
    self.pages.len()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.text(page)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionKind {
  Character,
  Line,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
  pub anchor: Position,
  pub kind: SelectionKind,
}

#[derive(Clone, Copy, Debug)]
pub struct Operator {
  pub count: usize,
  pub register: char,
}

#[derive(Clone, Copy, Debug)]
pub struct Location {
  pub cursor: Position,
  pub scroll: f32,
}

#[derive(Clone, Debug)]
pub struct Yank {
  pub text: String,
  pub register: char,
}

pub struct Reader {
  pub cursor: Position,
  pub scroll: f32,
  pub selection: Option<Selection>,
  pub operator: Option<Operator>,
  pub last_selection: Option<(Selection, Position)>,
  pub desired: Option<usize>,
  pub message: String,
  pub pending_yank: Option<Yank>,
  pub registers: Cache<char, String>,
  history: Vec<Location>,
  future: Vec<Location>,
}

impl Default for Reader {
  fn default() -> Self {
    Self {
      cursor: Position::default(),
      scroll: 0.0,
      selection: None,
      operator: None,
      last_selection: None,
      desired: None,
      message: String::new(),
      pending_yank: None,
      registers: Cache::new(32 * MIB),
      history: Vec::new(),
      future: Vec::new(),
    }
  }
}

impl Reader {
  pub fn clear_history(&mut self) {
    self.history.clear();
    self.future.clear();
    self.last_selection = None;
  }

  pub fn remember(&mut self) {
    let location = Location { cursor: self.cursor, scroll: self.scroll };
    if self.history.last().is_none_or(|p| p.cursor != self.cursor) {
      self.history.push(location);
    }
    if self.history.len() > 100 {
      self.history.remove(0);
    }
    self.future.clear();
  }

  pub fn history(&mut self, forward: bool) {
    let (from, to) = if forward {
      (&mut self.future, &mut self.history)
    } else {
      (&mut self.history, &mut self.future)
    };
    if let Some(location) = from.pop() {
      to.push(Location { cursor: self.cursor, scroll: self.scroll });
      self.cursor = location.cursor;
      self.scroll = location.scroll;
    }
  }

  pub fn jump(
    &mut self,
    pos: Position,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let page = PageId(pos.page.0.min(source.page_count().saturating_sub(1)));
    let pos = source.page_text(page)?.clamp(Position { page, ..pos });
    self.remember();
    self.cursor = pos;
    self.desired = None;
    Ok(())
  }

  pub fn edge(
    &mut self,
    page: PageId,
    last: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let text = source.page_text(page)?;
    let pos = Position {
      page,
      line: if last { usize::MAX } else { 0 },
      glyph: if last { usize::MAX } else { 0 },
    };
    self.cursor = text.clamp(pos);
    Ok(())
  }

  pub fn cancel(&mut self) {
    self.end_selection();
    self.operator = None;
    self.message.clear();
  }

  pub fn visual(&mut self, kind: SelectionKind) {
    if self.selection.is_some_and(|s| s.kind == kind) {
      self.end_selection();
    } else {
      self.selection = Some(Selection { anchor: self.cursor, kind });
    }
  }

  pub fn end_selection(&mut self) {
    if let Some(selection) = self.selection.take() {
      self.last_selection = Some((selection, self.cursor));
    }
  }

  pub fn reselect(&mut self) {
    if let Some((selection, cursor)) = self.last_selection {
      self.selection = Some(selection);
      self.cursor = cursor;
    }
  }

  pub fn swap(&mut self) {
    if let Some(selection) = &mut self.selection {
      std::mem::swap(&mut selection.anchor, &mut self.cursor);
    }
  }
}
