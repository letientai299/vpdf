use crate::Result;
use crate::app::App;
use crate::reader::SelectionKind;
use crate::text::{Bounds, Glyph, Position, Text};
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
    let offset = self.top.glyph;
    fitting(glyphs.iter().enumerate(), offset.saturating_add(self.columns))
      .filter_map(move |(index, column, glyph)| {
        column.checked_sub(offset).map(|column| (index, column, glyph))
      })
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

  /// Position drawn at a cell, if any.
  fn hit(&self, text: &Text, column: usize, row: usize) -> Option<Position> {
    let line = self.top.line + row;
    let glyphs = &text.lines.get(line).filter(|_| row < self.height)?.glyphs;
    let (glyph, ..) = self.shown(glyphs).find(|&(_, start, glyph)| {
      (start..start + cells(glyph)).contains(&column)
    })?;
    Some(Position { line, glyph, ..self.top })
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

  fn clamped(mut self, text: &Text) -> Self {
    self.top.line =
      self.top.line.min(text.lines.len().saturating_sub(self.height.max(1)));
    if self.top.glyph == 0 {
      return self;
    }
    let mut max = 0;
    for line in text.lines.iter().skip(self.top.line).take(self.height) {
      let width = line.glyphs.iter().map(cells).sum::<usize>();
      max = max.max(width.saturating_sub(self.columns));
      if max >= self.top.glyph {
        return self;
      }
    }
    self.top.glyph = max;
    self
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
    let start = glyphs.iter().take(cursor.glyph).map(cells).sum::<usize>();
    let end = start + glyphs.get(cursor.glyph).map_or(0, cells);
    let glyph = top.glyph.min(start).max(end.saturating_sub(self.columns));
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
  pub(crate) fn clamp_text(&mut self, mut top: Position) -> Result<Position> {
    top.page.0 = top.page.0.min(self.doc.pages.len().saturating_sub(1));
    let text = self.doc.text(top.page)?;
    Ok(self.text_view_from(top).clamped(&text).top)
  }

  /// The text view from `top`, unmoved.
  fn text_view_from(&self, top: Position) -> View {
    let height = usize::from(self.screen.rows.saturating_sub(1));
    let columns = usize::from(self.screen.columns.saturating_sub(self.left));
    View { height, columns, top }
  }

  /// Text and the current viewport.
  fn next_text_view(&mut self) -> Result<(Arc<Text>, View)> {
    let text = self.doc.text(self.text_top.page)?;
    let view = self.text_view_from(self.text_top);
    Ok((text, view))
  }

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let page = self.text_top.page;
    let (text, view) = self.next_text_view()?;
    self.text_top = view.top;
    self.text_drawn = Some(view.top);
    let hovered = self.hover_segments().filter(|(on, _)| *on == page);
    let hovered = hovered.map_or_default(|(_, segments)| segments);
    let lines = text.lines.iter().skip(view.top.line).take(view.height);
    for (row, line) in lines.enumerate() {
      write!(out, "{}", self.at.goto(row, self.left))?;
      let position = Position { page, line: view.top.line + row, glyph: 0 };
      let band = self.line_style(position);
      if band != theme::RESET {
        write!(
          out,
          "{}{}{}",
          band,
          " ".repeat(view.columns),
          self.at.goto(row, self.left)
        )?;
      }
      // Test glyphs against this line's segments.
      let ends = line.glyphs.first().zip(line.glyphs.last());
      let level = ends.map(|(first, last)| first.bounds.union(last.bounds));
      let beside = |bounds: &&Bounds| {
        level
          .is_some_and(|level| level.y1 >= bounds.y0 && level.y0 <= bounds.y1)
      };
      let near: Vec<_> = hovered.iter().filter(beside).collect();
      let mut current = "";
      let mut written = 0;
      for (glyph, column, ch) in view.shown(&line.glyphs) {
        if column > written {
          write!(out, "{}", " ".repeat(column - written))?;
        }
        written = column + cells(ch);
        let line = view.top.line + row;
        let pos = Position { page, line, glyph };
        let linked = near.iter().any(|bounds| {
          let (x, y) = ch.bounds.centre();
          bounds.contains(x, y)
        });
        let style = self.text_style(pos, linked);
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

  /// Glyph `view` draws at a pane cell.
  fn glyph_in(
    &self,
    (text, view): (Arc<Text>, View),
    column: u16,
    row: u16,
  ) -> Option<Position> {
    let column = usize::from(column.saturating_sub(self.left));
    view.hit(&text, column, usize::from(row))
  }

  /// Text and the view on screen.
  fn shown_text_view(&mut self) -> Result<(Arc<Text>, View)> {
    match self.text_drawn {
      Some(top) => Ok((self.doc.text(top.page)?, self.text_view_from(top))),
      None => self.next_text_view(),
    }
  }

  pub fn text_position(&mut self, column: u16, row: u16) -> Result<Position> {
    let (text, view) = self.shown_text_view()?;
    let column = usize::from(column.saturating_sub(self.left));
    Ok(view.position(&text, column, usize::from(row)))
  }

  /// Glyph at a shown cell.
  pub(crate) fn text_shown(
    &mut self,
    column: u16,
    row: u16,
  ) -> Result<Option<Position>> {
    let shown = self.shown_text_view()?;
    Ok(self.glyph_in(shown, column, row))
  }

  /// Glyph at a cell of the next frame.
  pub fn text_hit(
    &mut self,
    column: u16,
    row: u16,
  ) -> Result<Option<Position>> {
    // Hover runs before the frame draws.
    let next = self.next_text_view()?;
    Ok(self.glyph_in(next, column, row))
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
    self.text_top =
      self.clamp_text(view.aligned(self.reader.cursor.line, fraction))?;
    Ok(())
  }

  pub(crate) fn text_origin(&mut self, position: Position) -> Result<()> {
    let text = self.doc.text(position.page)?;
    let column = text.lines.get(position.line).map_or(0, |line| {
      line.glyphs.iter().take(position.glyph).map(cells).sum()
    });
    self.text_top = self.clamp_text(Position { glyph: column, ..position })?;
    Ok(())
  }

  pub(crate) fn reveal_text(&mut self) -> Result<()> {
    let text = self.doc.text(self.reader.cursor.page)?;
    self.text_top = self
      .text_view_from(self.text_top)
      .keep_visible(self.reader.cursor, &text)
      .top;
    Ok(())
  }

  pub fn scroll_text(&mut self, lines: isize) -> Result<()> {
    let mut remaining = lines.unsigned_abs();
    let down = lines > 0;
    while remaining > 0 {
      let text = self.doc.text(self.text_top.page)?;
      let height = usize::from(self.layout.screen.rows.saturating_sub(1));
      let last = text.lines.len().saturating_sub(height.max(1));
      let room = if down {
        last.saturating_sub(self.text_top.line)
      } else {
        self.text_top.line
      };
      let step = room.min(remaining);
      self.text_top.line = self.text_top.line.saturating_add_signed(if down {
        step as isize
      } else {
        -(step as isize)
      });
      remaining -= step;
      if remaining == 0 {
        break;
      }
      if down {
        if self.text_top.page.0 + 1 >= self.doc.pages.len() {
          break;
        }
        self.text_top.page.0 += 1;
        self.text_top.line = 0;
      } else {
        if self.text_top.page.0 == 0 {
          break;
        }
        self.text_top.page.0 -= 1;
        self.text_top.line = self
          .doc
          .text(self.text_top.page)?
          .lines
          .len()
          .saturating_sub(height.max(1));
      }
      remaining -= 1;
    }
    self.text_top = self.clamp_text(self.text_top)?;
    Ok(())
  }

  pub(crate) fn pan_text(&mut self, columns: isize) -> Result<()> {
    self.text_top.glyph = self.text_top.glyph.saturating_add_signed(columns);
    self.text_top = self.clamp_text(self.text_top)?;
    Ok(())
  }

  fn text_style(&self, position: Position, linked: bool) -> &'static str {
    if self.reader.uses_cursor() && position == self.reader.cursor {
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
    if linked {
      return theme::TEXT_LINK;
    }
    self.line_style(position)
  }

  fn line_style(&self, position: Position) -> &'static str {
    if self.landing.as_ref().is_some_and(|landing| {
      position.page == landing.position.page
        && position.line == landing.position.line
    }) {
      return theme::TEXT_LANDING;
    }
    if self.on_cursorline(position) {
      return theme::TEXT_CURSORLINE;
    }
    theme::RESET
  }

  fn on_cursorline(&self, position: Position) -> bool {
    self.cursorline
      && self.reader.selection.is_some()
      && position.page == self.reader.cursor.page
      && position.line == self.reader.cursor.line
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
      ("wide glyphs", at(0, 20, 0), at(0, 28, 4), (10, 4), at(0, 20, 6)),
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
      ("clipped left glyph", 1, at(0, 2, 0), None),
      ("one cell pan", 1, at(0, 2, 1), Some((1, 2))),
      ("two cell pan", 2, at(0, 2, 1), Some((0, 2))),
      ("empty line", 0, at(0, 3, 0), None),
      ("below the view", 0, at(0, 4, 0), None),
      ("other page", 0, at(1, 0, 0), None),
      ("left of the view", 2, at(0, 1, 1), None),
      ("first after scrolling", 2, at(0, 1, 2), Some((0, 1))),
    ] {
      assert_eq!(view(left).cell(&text, position), expected, "{name}");
    }
    assert_eq!(view(0).cell(&marks, at(0, 0, 4)), Some((3, 0)), "mark");
    for (name, left, (column, row), expected) in [
      ("first glyph", 0, (0, 0), Some(at(0, 0, 0))),
      ("blank by a cut wide glyph", 0, (3, 0), None),
      ("past the last column", 0, (9, 1), None),
      ("wide glyph's second cell", 0, (1, 2), Some(at(0, 2, 0))),
      ("next wide glyph", 0, (2, 2), Some(at(0, 2, 1))),
      ("empty line", 0, (0, 3), None),
      ("last line", 0, (2, 4), Some(at(0, 4, 2))),
      ("below the view", 0, (0, 5), None),
      ("scrolled right", 2, (0, 1), Some(at(0, 1, 2))),
      ("row left of the view", 5, (1, 0), None),
    ] {
      let view = View { height: 5, ..view(left) };
      assert_eq!(view.hit(&text, column, row), expected, "{name}");
    }
    let short = View { height: 2, ..view(0) };
    assert_eq!(short.hit(&text, 0, 2), None, "below a short view");
    assert_eq!(view(0).hit(&marks, 3, 0), Some(at(0, 0, 3)), "mark");
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
