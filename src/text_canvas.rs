use crate::Result;
use crate::app::App;
use crate::reader::SelectionKind;
use crate::text::Position;
use crate::theme;
use std::io::Write;
use unicode_width::UnicodeWidthChar;

impl App {
  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let height = self.screen.rows.saturating_sub(1) as usize;
    let start = self.reader.cursor.line.saturating_sub(height / 2);
    let columns = self.screen.columns.saturating_sub(self.left) as usize;
    let horizontal = self.reader.cursor.glyph.saturating_sub(columns / 2);
    for (row, line) in text.lines.iter().skip(start).take(height).enumerate() {
      write!(out, "\x1b[{};{}H", row + 1, self.left + 1)?;
      let mut width = 0;
      for (glyph, ch) in line.glyphs.iter().enumerate().skip(horizontal) {
        width += ch.ch.width().unwrap_or(0);
        if width > columns {
          break;
        }
        let pos =
          Position { page: self.reader.cursor.page, line: start + row, glyph };
        let style = self.text_style(pos);
        write!(out, "{style}{}", ch.ch)?;
      }
      write!(out, "\x1b[0m")?;
    }
    Ok(())
  }

  pub fn text_position(&mut self, column: u16, row: u16) -> Result<Position> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let height = self.screen.rows.saturating_sub(1) as usize;
    let start = self.reader.cursor.line.saturating_sub(height / 2);
    let columns = self.screen.columns.saturating_sub(self.left) as usize;
    let horizontal = self.reader.cursor.glyph.saturating_sub(columns / 2);
    let line = start + usize::from(row);
    let mut position =
      Position { page: self.reader.cursor.page, line, glyph: horizontal };
    let mut width = 0;
    if let Some(line) = text.lines.get(line) {
      for (index, glyph) in line.glyphs.iter().enumerate().skip(horizontal) {
        position.glyph = index;
        width += glyph.ch.width().unwrap_or(0);
        if width > usize::from(column.saturating_sub(self.left)) {
          break;
        }
      }
    }
    Ok(text.clamp(position))
  }

  /// Screen cell of `position` in the text view, when shown.
  pub fn text_cell(
    &mut self,
    position: Position,
  ) -> Result<Option<(u16, u16)>> {
    let cursor = self.reader.cursor;
    if position.page != cursor.page {
      return Ok(None);
    }
    let text = self.doc.text(cursor.page)?;
    let height = self.screen.rows.saturating_sub(1) as usize;
    let start = cursor.line.saturating_sub(height / 2);
    let columns = self.screen.columns.saturating_sub(self.left) as usize;
    let horizontal = cursor.glyph.saturating_sub(columns / 2);
    let Some(line) = text.lines.get(position.line) else { return Ok(None) };
    if position.line < start || position.line >= start + height {
      return Ok(None);
    }
    let width: usize = line
      .glyphs
      .iter()
      .take(position.glyph)
      .skip(horizontal)
      .map(|glyph| glyph.ch.width().unwrap_or(0))
      .sum();
    if position.glyph < horizontal || width >= columns {
      return Ok(None);
    }
    let row = u16::try_from(position.line - start)?;
    Ok(Some((self.left + u16::try_from(width)?, row)))
  }

  fn text_style(&self, position: Position) -> &'static str {
    if position == self.reader.cursor {
      return theme::TEXT_CURSOR;
    }
    if let Some(selection) = self.reader.selection {
      let (first, last) =
        crate::motion::range(selection.anchor, self.reader.cursor);
      let selected = if selection.kind == SelectionKind::Line {
        (position.page, position.line) >= (first.page, first.line)
          && (position.page, position.line) <= (last.page, last.line)
      } else {
        position >= first && position <= last
      };
      if selected {
        return theme::TEXT_SELECTION;
      }
    }
    if !self.search.hidden
      && self
        .search
        .hits
        .iter()
        .any(|hit| position >= hit.first && position <= hit.last)
    {
      return theme::TEXT_MATCH;
    }
    theme::RESET
  }
}
