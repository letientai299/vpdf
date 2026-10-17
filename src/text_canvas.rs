use crate::Result;
use crate::app::App;
use crate::motion::Motion;
use crate::reader::SelectionKind;
use crate::text::{Glyph, Position, Text};
use crate::theme;
use std::io::Write;
use std::sync::Arc;
use unicode_width::UnicodeWidthChar;

/// Text view size and origin.
struct View {
  height: usize,
  columns: usize,
  top: Position,
}

impl View {
  /// Glyphs a row draws, with start columns.
  fn shown<'a>(
    &self,
    glyphs: &'a [Glyph],
  ) -> impl Iterator<Item = (usize, usize, &'a Glyph)> {
    fitting(glyphs.iter().enumerate().skip(self.top.glyph), self.columns)
  }

  /// Drawn position at a cell, else nearest.
  fn position(&self, text: &Text, column: usize, row: usize) -> Position {
    let line = self.top.line + row;
    let glyph = text.lines.get(line).map_or(self.top.glyph, |line| {
      let shown = self.shown(&line.glyphs);
      let hit = shown.take_while(|&(_, start, _)| start <= column).last();
      hit.map_or(self.top.glyph, |(index, ..)| index)
    });
    text.clamp(Position { line, glyph, ..self.top })
  }

  /// Cell of `position`, when drawn.
  fn cell(&self, text: &Text, position: Position) -> Option<(usize, usize)> {
    if position.page != self.top.page {
      return None;
    }
    let row = position.line.checked_sub(self.top.line)?;
    let line = text.lines.get(position.line).filter(|_| row < self.height)?;
    let mut shown = self.shown(&line.glyphs);
    let (_, column, _) = shown.find(|&(index, ..)| index == position.glyph)?;
    // A zero-width mark joins the last cell.
    Some((column.min(self.columns.saturating_sub(1)), row))
  }

  /// Moved least to show `cursor`.
  fn keep_visible(self, cursor: Position, text: &Text) -> Self {
    let top = if self.top.page == cursor.page {
      self.top
    } else {
      Position { page: cursor.page, ..Position::default() }
    };
    if self.height == 0 || self.columns == 0 {
      return Self { top, ..self };
    }
    // No margin, so edge clicks never scroll.
    let line = top
      .line
      .min(cursor.line)
      .max((cursor.line + 1).saturating_sub(self.height))
      .min(text.lines.len().saturating_sub(self.height));
    let glyphs = text.lines.get(cursor.line).map_or(&[][..], |l| &l.glyphs);
    let glyphs = glyphs.iter().enumerate().take(cursor.glyph + 1).rev();
    let first =
      fitting(glyphs, self.columns).last().map_or(cursor.glyph, |(i, ..)| i);
    let glyph = top.glyph.min(cursor.glyph).max(first);
    Self { top: Position { line, glyph, ..top }, ..self }
  }

  /// Origin with `line` at `fraction` down.
  fn aligned(&self, line: usize, fraction: f32) -> Position {
    let above = (fraction * self.height.saturating_sub(1) as f32).round();
    Position { line: line.saturating_sub(above as usize), ..self.top }
  }
}

fn cells(glyph: &Glyph) -> usize {
  glyph.ch.width().unwrap_or(0)
}

/// Leading `glyphs` that fit, with start columns.
fn fitting<'a>(
  glyphs: impl Iterator<Item = (usize, &'a Glyph)>,
  columns: usize,
) -> impl Iterator<Item = (usize, usize, &'a Glyph)> {
  let mut width = 0;
  glyphs.map_while(move |(index, glyph)| {
    let start = width;
    width += cells(glyph);
    (width <= columns).then_some((index, start, glyph))
  })
}

impl App {
  /// The text view from `top`, unmoved.
  fn text_view_from(&self, top: Position) -> View {
    let height = usize::from(self.screen.rows.saturating_sub(1));
    let columns = usize::from(self.screen.columns.saturating_sub(self.left));
    View { height, columns, top }
  }

  /// Cursor page text and next frame's view.
  fn next_text_view(&mut self) -> Result<(Arc<Text>, View)> {
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    let view = self.text_view_from(self.text_top).keep_visible(cursor, &text);
    Ok((text, view))
  }

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let page = self.reader.cursor.page;
    let (text, view) = self.next_text_view()?;
    self.text_top = view.top;
    self.text_drawn = Some(view.top);
    let lines = text.lines.iter().skip(view.top.line).take(view.height);
    for (row, line) in lines.enumerate() {
      write!(out, "{}", self.at.goto(row, self.left))?;
      let mut current = "";
      for (glyph, _, ch) in view.shown(&line.glyphs) {
        let line = view.top.line + row;
        let pos = Position { page, line, glyph };
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
    // Clicks hit the frame on screen.
    let (text, view) = match self.text_drawn {
      Some(top) => (self.doc.text(top.page)?, self.text_view_from(top)),
      None => self.next_text_view()?,
    };
    let column = usize::from(column.saturating_sub(self.left));
    Ok(view.position(&text, column, usize::from(row)))
  }

  /// Screen cell of `position` in the text view, when shown.
  pub fn text_cell(
    &mut self,
    position: Position,
  ) -> Result<Option<(u16, u16)>> {
    let (text, view) = self.next_text_view()?;
    let Some((column, row)) = view.cell(&text, position) else {
      return Ok(None);
    };
    Ok(Some((self.left + u16::try_from(column)?, u16::try_from(row)?)))
  }

  /// Cursor line `fraction` down the view.
  pub(crate) fn align_text(&mut self, fraction: f32) -> Result<()> {
    let (_, view) = self.next_text_view()?;
    self.text_top = view.aligned(self.reader.cursor.line, fraction);
    Ok(())
  }

  /// Cursor and view move `lines` lines.
  pub fn scroll_text(&mut self, lines: isize) -> Result<()> {
    let motion = if lines < 0 { Motion::Up } else { Motion::Down };
    let before = self.reader.cursor;
    self.reader.motion(motion, lines.unsigned_abs(), false, &mut self.doc)?;
    let cursor = self.reader.cursor;
    if cursor.page == before.page {
      let line = (self.text_top.line + cursor.line).saturating_sub(before.line);
      self.text_top.line = line;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Bounds, Line, PageId};

  fn text(rows: &[&str]) -> Text {
    let lines = rows.iter().map(|row| Line {
      paragraph: false,
      glyphs: row
        .chars()
        .map(|ch| Glyph { ch, bounds: Bounds::default() })
        .collect(),
    });
    Text { lines: lines.collect(), truncated: false }
  }

  fn at(page: usize, line: usize, glyph: usize) -> Position {
    Position { page: PageId(page), line, glyph }
  }

  #[test]
  fn view_moves_only_to_show_the_cursor() {
    let mut rows = vec!["abcdefghij"; 28];
    rows.extend(["日本語日本語", ""]);
    let text = text(&rows);
    for (name, top, cursor, (height, columns), expected) in [
      ("inside", at(0, 5, 2), at(0, 9, 5), (10, 4), at(0, 5, 2)),
      ("first row", at(0, 5, 0), at(0, 5, 0), (10, 4), at(0, 5, 0)),
      ("last row", at(0, 5, 0), at(0, 14, 0), (10, 4), at(0, 5, 0)),
      ("above", at(0, 5, 0), at(0, 3, 0), (10, 4), at(0, 3, 0)),
      ("below", at(0, 5, 0), at(0, 15, 0), (10, 4), at(0, 6, 0)),
      ("past the end", at(0, 25, 0), at(0, 26, 0), (10, 4), at(0, 20, 0)),
      ("short page", at(0, 5, 0), at(0, 7, 0), (40, 4), at(0, 0, 0)),
      ("other page", at(1, 5, 3), at(0, 2, 1), (10, 4), at(0, 0, 0)),
      ("left", at(0, 0, 5), at(0, 0, 2), (10, 4), at(0, 0, 2)),
      ("last column", at(0, 0, 0), at(0, 0, 3), (10, 4), at(0, 0, 0)),
      ("right", at(0, 0, 0), at(0, 0, 7), (10, 4), at(0, 0, 4)),
      ("wide glyphs", at(0, 20, 0), at(0, 28, 4), (10, 4), at(0, 20, 3)),
      ("empty line", at(0, 20, 5), at(0, 29, 0), (10, 4), at(0, 20, 0)),
      ("no rows", at(0, 5, 2), at(0, 15, 7), (0, 4), at(0, 5, 2)),
      ("no columns", at(0, 5, 2), at(0, 15, 7), (10, 0), at(0, 5, 2)),
    ] {
      let view = View { height, columns, top };
      assert_eq!(view.keep_visible(cursor, &text).top, expected, "{name}");
    }
  }

  #[test]
  fn clicks_and_cells_match_drawn_glyphs() {
    let marks = text(&["abcd\u{301}"]);
    let text = text(&["abc日", "abcdefghij", "日本語日本語", "", "xyz"]);
    let view = |glyph| View { height: 3, columns: 4, top: at(0, 0, glyph) };
    for (name, left, (column, row), expected) in [
      ("first glyph", 0, (0, 0), at(0, 0, 0)),
      ("blank by a cut wide glyph", 0, (3, 0), at(0, 0, 2)),
      ("past the last column", 0, (9, 1), at(0, 1, 3)),
      ("wide glyph's second cell", 0, (1, 2), at(0, 2, 0)),
      ("next wide glyph", 0, (2, 2), at(0, 2, 1)),
      ("empty line", 0, (0, 3), at(0, 3, 0)),
      ("below the text", 0, (0, 6), at(0, 4, 0)),
      ("scrolled right", 2, (0, 1), at(0, 1, 2)),
      ("row left of the view", 5, (1, 0), at(0, 0, 3)),
    ] {
      let position = view(left).position(&text, column, row);
      assert_eq!(position, expected, "{name}");
    }
    for (name, left, position, expected) in [
      ("drawn", 0, at(0, 0, 2), Some((2, 0))),
      ("cut wide glyph", 0, at(0, 0, 3), None),
      ("after a wide glyph", 0, at(0, 2, 1), Some((2, 2))),
      ("empty line", 0, at(0, 3, 0), None),
      ("below the view", 0, at(0, 4, 0), None),
      ("other page", 0, at(1, 0, 0), None),
      ("left of the view", 2, at(0, 1, 1), None),
      ("first after scrolling", 2, at(0, 1, 2), Some((0, 1))),
    ] {
      assert_eq!(view(left).cell(&text, position), expected, "{name}");
    }
    assert_eq!(view(0).cell(&marks, at(0, 0, 4)), Some((3, 0)), "mark");
  }

  #[test]
  fn aligning_puts_the_line_at_a_fraction() {
    let view = View { height: 10, columns: 4, top: at(0, 0, 0) };
    for (fraction, line, top) in
      [(0.0, 30, 30), (0.5, 30, 25), (1.0, 30, 21), (1.0, 3, 0)]
    {
      assert_eq!(view.aligned(line, fraction).line, top, "{fraction} {line}");
    }
  }
}
