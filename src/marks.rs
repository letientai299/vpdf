use crate::Result;
use crate::app::{App, Request};
use crate::panel::{Destination, Row};
use crate::state::Mark;
use crate::text::Position;
use std::collections::BTreeMap;

impl App {
  /// `m{a-z}` marks this document; `m{A-Z}` marks across documents.
  pub fn set_mark(&mut self, name: char) {
    if name.is_ascii_uppercase() {
      self.request = Some(Request::SetMark(name));
    } else {
      self.marks.insert(name, self.reader.cursor);
    }
    self.reader.message = format!("Mark {name} set");
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
    self.reader.jump(target, &mut self.doc)?;
    self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
    self.ensure_cursor()
  }

  /// Text of the extracted line at `position`, for mark listings.
  fn line_text(&mut self, position: Position) -> String {
    let Ok(text) = self.doc.text(position.page) else { return String::new() };
    text.lines.get(position.line).map_or_else(String::new, |line| {
      line
        .glyphs
        .iter()
        .map(|glyph| glyph.ch)
        .collect::<String>()
        .trim()
        .to_owned()
    })
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
