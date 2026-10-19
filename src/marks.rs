use crate::Result;
use crate::app::{App, Request};
use crate::pdf_text::PdfText;
use crate::state::Mark;
use crate::text::{PageId, Position};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use vpdf_ui::panel::{Destination, Row};

pub struct Landing {
  pub position: Position,
  pub until: Instant,
  pub span: Span,
}

/// What a landing tints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Span {
  /// A jump's destination line.
  Line,
  /// A click's target glyph.
  Glyph,
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

  /// Goes to a position: a shown one keeps the view, else it lands as
  /// followed links do.
  pub fn go_to(&mut self, position: Position, exact: bool) -> Result<()> {
    let target =
      if exact { position } else { Position { glyph: 0, ..position } };
    self.land_at(target)
  }

  /// Shows `page` from its top, as page targets do.
  pub fn go_to_page(&mut self, page: PageId) -> Result<()> {
    self.jump_cursor(Position { page, ..Position::default() })?;
    self.show_page(self.reader.cursor.page)?;
    self.ensure_cursor()?;
    self.flash_landing();
    Ok(())
  }

  /// Follows a link or row to `position`.
  pub fn land_at(&mut self, position: Position) -> Result<()> {
    let position =
      vpdf_reader::reader::clamp(position, &mut PdfText(&mut self.doc))?;
    // A repeat, as a double click's, stays.
    let here = self.landed == Some(self.landed_view(position))
      && self.reader.cursor == position;
    if !here {
      self.jump_cursor(position)?;
      self.land()?;
      self.landed = Some(self.landed_view(self.reader.cursor));
    }
    self.flash_landing();
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
    self.reader.cursor =
      vpdf_reader::reader::clamp(target, &mut PdfText(&mut self.doc))?;
    self.reader.record(origin);
    self.reader.describe_jump(None);
    self.reader.desired = None;
    self.target = Some(self.reader.cursor);
    Ok(())
  }

  pub(crate) fn flash_landing(&mut self) {
    self.flash(self.reader.cursor, Span::Line);
  }

  /// The click mark, shown in Normal only.
  pub(crate) fn click_mark(&self) -> Option<Position> {
    let landing = self.landing.as_ref()?;
    let normal = self.reader.viewport_mode() && self.reader.selection.is_none();
    (landing.span == Span::Glyph && normal).then_some(landing.position)
  }

  /// Tints `position`'s `span` briefly.
  pub(crate) fn flash(&mut self, position: Position, span: Span) {
    let until = Instant::now() + Landing::DURATION;
    self.landing = Some(Landing { position, until, span });
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
