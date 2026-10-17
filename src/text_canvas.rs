use crate::Result;
use crate::app::App;
use crate::reader::SelectionKind;
use crate::text::Position;
use crate::theme;
use std::io::Write;
use unicode_width::UnicodeWidthChar;

/// Rows, first line, columns, and first glyph of the text view.
struct View {
  height: usize,
  start: usize,
  columns: usize,
  horizontal: usize,
}

impl App {
  fn text_view(&self) -> View {
    let cursor = self.reader.cursor;
    let height = usize::from(self.screen.rows.saturating_sub(1));
    let columns = usize::from(self.screen.columns.saturating_sub(self.left));
    View {
      height,
      start: cursor.line.saturating_sub(height / 2),
      columns,
      horizontal: cursor.glyph.saturating_sub(columns / 2),
    }
  }

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let view = self.text_view();
    let lines = text.lines.iter().skip(view.start).take(view.height);
    for (row, line) in lines.enumerate() {
      write!(out, "{}", self.at.goto(row, self.left))?;
      let mut width = 0;
      let mut current = "";
      for (glyph, ch) in line.glyphs.iter().enumerate().skip(view.horizontal) {
        width += ch.ch.width().unwrap_or(0);
        if width > view.columns {
          break;
        }
        let line = view.start + row;
        let pos = Position { page: self.reader.cursor.page, line, glyph };
        let style = self.text_style(pos);
        let style = if style == theme::RESET && self.panel_focused() {
          theme::DIM
        } else {
          style
        };
        if style != current {
          write!(out, "{style}")?;
          current = style;
        }
        write!(out, "{}", ch.ch)?;
      }
      write!(out, "\x1b[0m")?;
    }
    Ok(())
  }

  pub fn text_position(&mut self, column: u16, row: u16) -> Result<Position> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let view = self.text_view();
    let line = view.start + usize::from(row);
    let mut position =
      Position { page: self.reader.cursor.page, line, glyph: view.horizontal };
    let mut width = 0;
    if let Some(line) = text.lines.get(line) {
      let glyphs = line.glyphs.iter().enumerate().skip(view.horizontal);
      for (index, glyph) in glyphs {
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
    let view = self.text_view();
    let Some(line) = text.lines.get(position.line) else { return Ok(None) };
    if position.line < view.start || position.line >= view.start + view.height {
      return Ok(None);
    }
    let width: usize = line
      .glyphs
      .iter()
      .take(position.glyph)
      .skip(view.horizontal)
      .map(|glyph| glyph.ch.width().unwrap_or(0))
      .sum();
    if position.glyph < view.horizontal || width >= view.columns {
      return Ok(None);
    }
    let row = u16::try_from(position.line - view.start)?;
    Ok(Some((self.left + u16::try_from(width)?, row)))
  }

  fn text_style(&self, position: Position) -> &'static str {
    if position == self.reader.cursor {
      return theme::TEXT_CURSOR;
    }
    if let Some(selection) = self.reader.selection {
      let range = crate::motion::range(selection.anchor, self.reader.cursor);
      let linewise = selection.kind == SelectionKind::Line;
      if crate::motion::covers(range, linewise, position) {
        return theme::TEXT_SELECTION;
      }
    }
    if !self.search.hidden
      && crate::search::contains(&self.search.hits, position)
    {
      return theme::TEXT_MATCH;
    }
    theme::RESET
  }
}
