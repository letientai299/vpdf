use crate::Result;
use crate::app::{App, Request};
use crate::panel::{Destination, Row};
use crate::state::Mark;
use crate::text::Position;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub struct Landing {
  pub position: Position,
  pub until: Instant,
}

impl Landing {
  pub const DURATION: Duration = Duration::from_millis(600);
}

/// A target and its landed view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landed {
  target: Position,
  scroll: f32,
  pan: f32,
  text_top: Position,
}

impl App {
  /// `m{a-z}` marks this document; `m{A-Z}` marks across documents.
  pub fn set_mark(&mut self, name: char) -> Result<()> {
    if name.is_ascii_uppercase() {
      self.request = Some(Request::SetMark(name));
    } else {
      let position = self.reading_position()?;
      self.marks.insert(name, position);
    }
    self.reader.message = format!("Mark {name} set");
    Ok(())
  }

  /// `'x` goes to the mark's line, `` `x `` to its exact glyph.
  pub fn go_mark(&mut self, name: char, exact: bool) -> Result<()> {
    if name.is_ascii_uppercase() {
      self.request = Some(Request::GoMark(name, exact));
      return Ok(());
    }
    let position = *self
      .marks
      .get(&name)
      .ok_or_else(|| anyhow::anyhow!("Mark not set: {name}"))?;
    self.go_to(position, exact)
  }

  pub fn go_to(&mut self, position: Position, exact: bool) -> Result<()> {
    let target =
      if exact { position } else { Position { glyph: 0, ..position } };
    self.jump_cursor(target)?;
    self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
    self.ensure_cursor()?;
    self.flash_landing();
    Ok(())
  }

  /// Follows a link or row to `position`.
  pub fn land_at(&mut self, position: Position) -> Result<()> {
    self.jump_cursor(position)?;
    self.land()?;
    self.flash_landing();
    self.landed = Some(self.landed_view(self.reader.cursor));
    Ok(())
  }

  /// `target` in the current view.
  pub(crate) fn landed_view(&self, target: Position) -> Landed {
    Landed {
      target,
      scroll: self.reader.scroll,
      pan: self.layout.pan,
      text_top: self.text_top,
    }
  }

  /// Moves the cursor as a recorded jump.
  fn jump_cursor(&mut self, target: Position) -> Result<()> {
    let origin = self.location()?;
    self.reader.cursor = crate::reader::clamp(target, &mut self.doc)?;
    self.reader.record(origin);
    self.reader.describe_jump(None);
    self.reader.desired = None;
    self.target = Some(self.reader.cursor);
    Ok(())
  }

  pub(crate) fn flash_landing(&mut self) {
    self.landing = Some(Landing {
      position: self.reader.cursor,
      until: Instant::now() + Landing::DURATION,
    });
  }

  /// Text of the extracted line at `position`, for mark listings.
  fn line_text(&mut self, position: Position) -> String {
    let Ok(text) = self.doc.text(position.page) else { return String::new() };
    let line = text.lines.get(position.line);
    line.map_or_else(String::new, |line| line.text().trim().to_owned())
  }

  /// This document's marks, then global marks.
  pub fn mark_rows(&mut self, global: &BTreeMap<char, Mark>) -> Vec<Row> {
    let local: Vec<_> = self.marks.iter().map(|(n, p)| (*n, *p)).collect();
    let mut rows: Vec<_> = local
      .into_iter()
      .map(|(name, position)| {
        Row::new(
          format!("{name}  {}", self.line_text(position)),
          format!("p{}", position.page.0 + 1),
          Destination::Position(position),
        )
      })
      .collect();
    rows.extend(global.iter().map(|(name, mark)| {
      let file = mark.path.file_name().unwrap_or_default().to_string_lossy();
      Row::new(
        format!("{name}  {file}"),
        format!("p{}", mark.cursor.page.0 + 1),
        Destination::Mark(*name),
      )
    }));
    rows
  }
}
