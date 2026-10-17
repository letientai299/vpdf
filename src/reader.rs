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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
  Normal,
  Cursor,
  Visual(SelectionKind),
}

#[derive(Clone, Copy, Default)]
enum EntryMode {
  #[default]
  Normal,
  Cursor,
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

#[derive(Clone, Copy, Debug, Default)]
pub struct Location {
  pub cursor: Position,
  pub scroll: f32,
  pub anchor: Option<(PageId, (f32, f32))>,
  pub text_top: Option<Position>,
  pub target: Option<Position>,
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
  /// Yanked text; the named, `"`, and `0` registers share one copy.
  pub registers: Cache<char, Arc<str>>,
  entry: EntryMode,
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
      entry: EntryMode::Normal,
      history: Vec::new(),
      future: Vec::new(),
    }
  }
}

impl Reader {
  pub fn mode(&self) -> Mode {
    if let Some(selection) = self.selection {
      return Mode::Visual(selection.kind);
    }
    match self.entry {
      EntryMode::Normal => Mode::Normal,
      EntryMode::Cursor => Mode::Cursor,
    }
  }

  pub fn uses_cursor(&self) -> bool {
    self.mode() != Mode::Normal
  }

  pub fn viewport_mode(&self) -> bool {
    !self.uses_cursor() && self.operator.is_none()
  }

  pub(crate) fn enter_cursor(&mut self) {
    self.entry = EntryMode::Cursor;
  }

  pub(crate) fn normal(&mut self) {
    self.entry = EntryMode::Normal;
  }

  pub fn clear_history(&mut self) {
    self.history.clear();
    self.future.clear();
    self.last_selection = None;
  }

  pub fn remember(&mut self) {
    self.record(Location {
      cursor: self.cursor,
      scroll: self.scroll,
      ..Location::default()
    });
  }

  /// Adds `location` to the jump list.
  pub fn record(&mut self, location: Location) {
    match self.history.last_mut() {
      Some(previous) if previous.cursor == location.cursor => {
        *previous = location;
      }
      _ => self.history.push(location),
    }
    if self.history.len() > 100 {
      self.history.remove(0);
    }
    self.future.clear();
  }

  pub fn jumps(&self) -> impl Iterator<Item = Position> {
    self.history.iter().chain(self.future.iter().rev()).map(|p| p.cursor)
  }

  pub fn history(&mut self, forward: bool) {
    let current = Location {
      cursor: self.cursor,
      scroll: self.scroll,
      ..Location::default()
    };
    if let Some(location) = self.recall(forward, current) {
      self.cursor = location.cursor;
      self.scroll = location.scroll;
    }
  }

  pub fn recall(
    &mut self,
    forward: bool,
    current: Location,
  ) -> Option<Location> {
    let (from, to) = if forward {
      (&mut self.future, &mut self.history)
    } else {
      (&mut self.history, &mut self.future)
    };
    let location = from.pop()?;
    to.push(current);
    Some(location)
  }

  pub fn jump(
    &mut self,
    pos: Position,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let pos = clamp(pos, source)?;
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
    self.disarm();
    self.message.clear();
  }

  /// Drops a pending operator and message.
  pub fn disarm(&mut self) {
    if self.operator.take().is_some() {
      self.message.clear();
    }
  }

  /// Starts, ends, or changes the kind of a selection, as `v` and `V`.
  pub fn visual(&mut self, kind: SelectionKind) {
    match &mut self.selection {
      Some(selection) if selection.kind == kind => self.end_selection(),
      Some(selection) => selection.kind = kind,
      None => self.selection = Some(Selection { anchor: self.cursor, kind }),
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

/// `pos` moved onto a page and glyph of `source`.
pub fn clamp(pos: Position, source: &mut impl TextSource) -> Result<Position> {
  let page = PageId(pos.page.0.min(source.page_count().saturating_sub(1)));
  Ok(source.page_text(page)?.clamp(Position { page, ..pos }))
}
