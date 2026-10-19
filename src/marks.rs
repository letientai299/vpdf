use crate::Result;
use crate::app::{App, Request};
use crate::state::Mark;
use std::collections::BTreeMap;
use vpdf_model::text::{PageId, Position};
use vpdf_ui::panel::Row;

impl App {
  /// `m{a-z}` marks this document; `m{A-Z}` marks across documents.
  pub fn set_mark(&mut self, name: char) -> Result<()> {
    if name.is_ascii_uppercase() {
      self.request = Some(Request::SetMark(name));
    } else {
      let position = self.reading_position()?;
      self.marks.insert(name, position);
    }
    self.reader.notice(format!("Mark {name} set"));
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

  pub fn go_to_page(&mut self, page: PageId) -> Result<()> {
    self.reading_view().go_to_page(page)
  }

  pub fn land_at(&mut self, position: Position) -> Result<()> {
    self.reading_view().land_at(position)
  }

  /// This document's marks, then global marks.
  pub fn mark_rows(&mut self, global: &BTreeMap<char, Mark>) -> Vec<Row> {
    let local: Vec<_> = self.marks.iter().map(|(n, p)| (*n, *p)).collect();
    let mut rows: Vec<_> = local
      .into_iter()
      .map(|(name, position)| {
        let text = self.doc.text(position.page).ok();
        Row::local_mark(name, position, text.as_deref())
      })
      .collect();
    rows.extend(
      global
        .iter()
        .map(|(name, mark)| Row::global_mark(*name, mark.cursor, &mark.path)),
    );
    rows
  }
}
