use crate::Result;
use crate::app::App;
use crate::marks::Span;
use crate::pdf_text::PdfText;
use crate::text::{Bounds, Glyph, Line, PageId, Position, Text};
use std::io::Write;
use std::sync::Arc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vpdf_reader::reader::{SelectionKind, TextSource};
use vpdf_ui::theme;

/// A line that stands in for an empty page.
static BLANK: Line = Line { paragraph: false, glyphs: Vec::new() };

/// Rows `page` takes: its lines, or one blank row when it has none,
/// then a break before the next page.
///
/// A row is a `Position` whose `line` past the page's lines is that
/// break, so rows order as positions do.
fn page_rows(pages: &mut impl TextSource, page: PageId) -> Result<usize> {
  let count = pages.page_count();
  let text = pages.page_text(page)?;
  Ok(rows_in(&text, page, count))
}

/// `page_rows` of a read page.
fn rows_in(text: &Text, page: PageId, count: usize) -> usize {
  lines_in(text) + usize::from(page.0 + 1 < count)
}

/// Line rows of a read page.
fn lines_in(text: &Text) -> usize {
  text.lines.len().max(1)
}

/// `at` moved `rows` rows within the document, and how far it moved.
fn step(
  pages: &mut impl TextSource,
  mut at: Position,
  rows: isize,
) -> Result<(Position, usize)> {
  let count = pages.page_count();
  let mut left = rows.unsigned_abs();
  let mut moved = 0;
  if rows < 0 {
    while left > at.line {
      if at.page.0 == 0 {
        return Ok((Position { line: 0, ..at }, moved + at.line));
      }
      left -= at.line + 1;
      moved += at.line + 1;
      at.page.0 -= 1;
      at.line = page_rows(pages, at.page)? - 1;
    }
    return Ok((Position { line: at.line - left, ..at }, moved + left));
  }
  loop {
    let rows = page_rows(pages, at.page)?;
    let room = (rows - 1).saturating_sub(at.line);
    if left <= room {
      return Ok((Position { line: at.line + left, ..at }, moved + left));
    }
    if at.page.0 + 1 >= count {
      return Ok((Position { line: at.line + room, ..at }, moved + room));
    }
    left -= room + 1;
    moved += room + 1;
    at = Position { page: PageId(at.page.0 + 1), line: 0, ..at };
  }
}

/// `top` within the document, filling the view where it can.
fn clamp(
  pages: &mut impl TextSource,
  mut top: Position,
  size: (usize, usize),
) -> Result<Position> {
  let count = pages.page_count();
  top.page.0 = top.page.0.min(count.saturating_sub(1));
  let rows = page_rows(pages, top.page)?;
  top.line = top.line.min(rows - 1);
  // Reads later pages only when short.
  if let Some(last) = size.0.checked_sub(1)
    && rows - top.line <= last
  {
    let (_, ahead) = step(pages, top, last as isize)?;
    top = step(pages, top, -((last - ahead) as isize))?.0;
  }
  if top.glyph == 0 {
    return Ok(top);
  }
  Ok(View::new(pages, size, top)?.clamped().top)
}

/// A text view row.
#[derive(Clone, Copy)]
enum Row<'a> {
  /// A page line; `glyph` is 0.
  Line(Position, &'a Line),
  /// The break after a page's lines.
  Break(Position),
}

/// Text view size, origin, and the pages its rows show.
struct View {
  height: usize,
  columns: usize,
  /// First row; `glyph` is the column offset.
  top: Position,
  /// `top`'s page.
  first: Arc<Text>,
  /// Pages after it, through one row past the view; often none.
  more: Vec<Arc<Text>>,
  /// Pages in the document.
  count: usize,
}

impl View {
  /// `height` rows and `columns` columns from `top`.
  fn new(
    pages: &mut impl TextSource,
    (height, columns): (usize, usize),
    top: Position,
  ) -> Result<Self> {
    let count = pages.page_count();
    let first = pages.page_text(top.page)?;
    let mut rows = rows_in(&first, top.page, count).saturating_sub(top.line);
    let mut more = Vec::new();
    let mut page = PageId(top.page.0 + 1);
    // One more row places presses below.
    while page.0 < count && rows <= height {
      let text = pages.page_text(page)?;
      rows += rows_in(&text, page, count);
      more.push(text);
      page.0 += 1;
    }
    Ok(Self { height, columns, top, first, more, count })
  }

  /// Read pages from `top`'s.
  fn texts(&self) -> impl Iterator<Item = (PageId, &Arc<Text>)> {
    let first = self.top.page.0;
    let texts = std::iter::once(&self.first).chain(&self.more);
    texts.enumerate().map(move |(index, text)| (PageId(first + index), text))
  }

  /// Rows from the top, through the read pages.
  fn all_rows(&self) -> impl Iterator<Item = Row<'_>> {
    let (top, count) = (self.top, self.count);
    self.texts().flat_map(move |(page, text)| {
      let start = if page == top.page { top.line } else { 0 };
      let blank = text.lines.is_empty().then_some(&BLANK);
      let lines = text.lines.iter().chain(blank).enumerate().skip(start);
      let lines = lines.map(move |(line, row)| {
        Row::Line(Position { page, line, glyph: 0 }, row)
      });
      let line = lines_in(text);
      let gap = (page.0 + 1 < count && start <= line)
        .then_some(Row::Break(Position { page, line, glyph: 0 }));
      lines.chain(gap)
    })
  }

  /// Rows the view draws.
  fn rows(&self) -> impl Iterator<Item = Row<'_>> {
    self.all_rows().take(self.height)
  }

  /// Row showing `position`'s line, and the line.
  fn row_of(&self, position: Position) -> Option<(usize, &Line)> {
    self.rows().enumerate().find_map(|(row, shown)| match shown {
      Row::Line(at, line)
        if at.page == position.page && at.line == position.line =>
      {
        Some((row, line))
      }
      _ => None,
    })
  }

  /// Pages with drawn rows.
  fn shown_pages(&self) -> std::ops::Range<usize> {
    let last = self.rows().last().map_or(self.top.page, |row| match row {
      Row::Line(at, _) | Row::Break(at) => at.page,
    });
    self.top.page.0..last.0 + 1
  }

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
  fn position(&self, column: usize, row: usize) -> Position {
    // One row past the view, for drags.
    let rows = row.min(self.height) + 1;
    match self.all_rows().take(rows).last() {
      Some(Row::Line(at, line)) => {
        let shown = self.shown(&line.glyphs);
        let hit = shown.take_while(|&(_, start, _)| start <= column).last();
        let glyph = hit.map_or(self.top.glyph, |(index, ..)| index);
        let glyph = glyph.min(line.glyphs.len().saturating_sub(1));
        Position { glyph, ..at }
      }
      // A break reads as its page's end.
      Some(Row::Break(at)) => {
        let text = self.texts().find(|(page, _)| *page == at.page);
        let end = Position { glyph: usize::MAX, ..at };
        text.map_or(at, |(_, text)| text.clamp(end))
      }
      None => self.top,
    }
  }

  /// Position drawn at a cell, if any.
  fn hit(&self, column: usize, row: usize) -> Option<Position> {
    let Row::Line(at, line) = self.rows().nth(row)? else { return None };
    let (glyph, ..) = self.shown(&line.glyphs).find(|&(_, start, glyph)| {
      (start..start + cells(glyph)).contains(&column)
    })?;
    Some(Position { glyph, ..at })
  }

  /// Cell of `position`, when drawn.
  fn cell(&self, position: Position) -> Option<(usize, usize)> {
    let (row, line) = self.row_of(position)?;
    let mut shown = self.shown(&line.glyphs);
    let (_, column, _) = shown.find(|&(index, ..)| index == position.glyph)?;
    // A zero-width mark joins the last cell.
    Some((column.min(self.columns.saturating_sub(1)), row))
  }

  /// Column offset no wider than the drawn lines need.
  fn clamped(mut self) -> Self {
    let overflow = |row| match row {
      Row::Line(_, line) => {
        let width = line.glyphs.iter().map(cells).sum::<usize>();
        width.saturating_sub(self.columns)
      }
      Row::Break(_) => 0,
    };
    // Stops at a line needing the offset.
    let widest = self.rows().map(overflow).try_fold(0, |max, overflow| {
      let max = overflow.max(max);
      if max >= self.top.glyph { Err(max) } else { Ok(max) }
    });
    if let Ok(max) = widest {
      self.top.glyph = max;
    }
    self
  }

  /// Origin moved least to show `cursor`.
  fn keep_visible(
    &self,
    cursor: Position,
    pages: &mut impl TextSource,
  ) -> Result<Position> {
    let top = self.top;
    if self.height == 0 || self.columns == 0 {
      let other = Position { page: cursor.page, ..Position::default() };
      return Ok(if top.page == cursor.page { top } else { other });
    }
    // No margin, so edge clicks never scroll.
    let line = Position { glyph: top.glyph, ..cursor };
    let mut top = if self.row_of(cursor).is_some() {
      top
    } else if line < top {
      line
    } else {
      step(pages, line, 1 - self.height as isize)?.0
    };
    let text = pages.page_text(cursor.page)?;
    let glyphs = text.lines.get(cursor.line).map_or(&[][..], |l| &l.glyphs);
    let start = glyphs.iter().take(cursor.glyph).map(cells).sum::<usize>();
    let end = start + glyphs.get(cursor.glyph).map_or(0, cells);
    top.glyph = top.glyph.min(start).max(end.saturating_sub(self.columns));
    clamp(pages, top, (self.height, self.columns))
  }

  /// Origin with `at`'s line at `fraction` down.
  fn aligned(
    &self,
    at: Position,
    fraction: f32,
    pages: &mut impl TextSource,
  ) -> Result<Position> {
    let above = (fraction * self.height.saturating_sub(1) as f32).round();
    let at = Position { glyph: self.top.glyph, ..at };
    let top = step(pages, at, -(above as isize))?.0;
    clamp(pages, top, (self.height, self.columns))
  }

  /// Whether `cursor`'s line clears the edge rows.
  fn inside(&self, cursor: Position) -> bool {
    self
      .row_of(cursor)
      .is_some_and(|(row, _)| row > 0 && row + 2 <= self.height)
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
  pub(crate) fn clamp_text(&mut self, top: Position) -> Result<Position> {
    let size = self.text_size();
    clamp(&mut PdfText(&mut self.doc), top, size)
  }

  /// Text view rows and columns.
  fn text_size(&self) -> (usize, usize) {
    let height = usize::from(self.screen.rows.saturating_sub(1));
    let columns = usize::from(self.screen.columns.saturating_sub(self.left));
    (height, columns)
  }

  /// The text view from `top`, unmoved.
  fn text_view_from(&mut self, top: Position) -> Result<View> {
    let size = self.text_size();
    View::new(&mut PdfText(&mut self.doc), size, top)
  }

  /// The next frame's view.
  fn next_text_view(&mut self) -> Result<View> {
    self.text_view_from(self.text_top)
  }

  /// `at` moved `rows` text view rows.
  fn text_step(&mut self, at: Position, rows: isize) -> Result<Position> {
    Ok(step(&mut PdfText(&mut self.doc), at, rows)?.0)
  }

  /// Pages the next frame shows.
  pub(crate) fn text_pages(&mut self) -> Result<std::ops::Range<usize>> {
    Ok(self.next_text_view()?.shown_pages())
  }

  /// First drawn glyph that is not a space.
  pub(crate) fn first_text(&mut self) -> Result<Option<Position>> {
    let view = self.next_text_view()?;
    Ok(view.rows().find_map(|row| {
      let Row::Line(at, line) = row else { return None };
      let mut shown = view.shown(&line.glyphs);
      let (glyph, ..) =
        shown.find(|(_, _, glyph)| !glyph.ch.is_whitespace())?;
      Some(Position { glyph, ..at })
    }))
  }

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let view = self.next_text_view()?;
    self.text_top = view.top;
    self.text_drawn = Some(view.top);
    let hovered = self.hover_segments();
    for (row, shown) in view.rows().enumerate() {
      write!(out, "{}", self.at.goto(row, self.left))?;
      let (at, line) = match shown {
        Row::Line(at, line) => (at, line),
        Row::Break(at) => {
          self.page_break(out, at, view.columns)?;
          continue;
        }
      };
      let band = self.line_style(at);
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
      let beside = |(on, bounds): &&(PageId, Bounds)| {
        *on == at.page
          && level
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
        let pos = Position { glyph, ..at };
        let linked = near.iter().any(|(_, bounds)| {
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

  /// A rule naming the next page, at break `at`.
  fn page_break(
    &self,
    out: &mut impl Write,
    at: Position,
    columns: usize,
  ) -> Result<()> {
    let style =
      if self.selects(at) { theme::TEXT_SELECTION } else { theme::DIM };
    let label = self.doc.label(PageId(at.page.0 + 1));
    let head = vpdf_terminal::terminal::clean(&format!("── {label} "), columns);
    let fill = columns.saturating_sub(head.width());
    write!(out, "{style}{head}{}\x1b[0m", "─".repeat(fill))?;
    Ok(())
  }

  /// Glyph `view` draws at a pane cell.
  fn glyph_in(&self, view: &View, column: u16, row: u16) -> Option<Position> {
    let column = usize::from(column.saturating_sub(self.left));
    view.hit(column, usize::from(row))
  }

  /// The view on screen.
  fn shown_text_view(&mut self) -> Result<View> {
    match self.text_drawn {
      Some(top) => self.text_view_from(top),
      None => self.next_text_view(),
    }
  }

  pub fn text_position(&mut self, column: u16, row: u16) -> Result<Position> {
    let view = self.shown_text_view()?;
    let column = usize::from(column.saturating_sub(self.left));
    Ok(view.position(column, usize::from(row)))
  }

  /// Whether a shown row is a page break.
  pub(crate) fn text_break(&mut self, row: u16) -> Result<bool> {
    let view = self.shown_text_view()?;
    Ok(matches!(view.rows().nth(usize::from(row)), Some(Row::Break(_))))
  }

  /// Next frame's row of `position`'s line.
  pub fn text_row(&mut self, position: Position) -> Result<Option<usize>> {
    let view = self.next_text_view()?;
    Ok(view.row_of(position).map(|(row, _)| row))
  }

  /// Glyph at a shown cell.
  pub(crate) fn text_shown(
    &mut self,
    column: u16,
    row: u16,
  ) -> Result<Option<Position>> {
    let shown = self.shown_text_view()?;
    Ok(self.glyph_in(&shown, column, row))
  }

  /// Glyph at a cell of the next frame.
  pub fn text_hit(
    &mut self,
    column: u16,
    row: u16,
  ) -> Result<Option<Position>> {
    // Hover runs before the frame draws.
    let next = self.next_text_view()?;
    Ok(self.glyph_in(&next, column, row))
  }

  /// Screen cell of `position` in the text view, when shown.
  pub fn text_cell(
    &mut self,
    position: Position,
  ) -> Result<Option<(u16, u16)>> {
    let view = self.next_text_view()?;
    let Some((column, row)) = view.cell(position) else {
      return Ok(None);
    };
    Ok(Some((self.left + u16::try_from(column)?, u16::try_from(row)?)))
  }

  /// Cursor line `fraction` down the view.
  pub(crate) fn align_text(&mut self, fraction: f32) -> Result<()> {
    let view = self.next_text_view()?;
    self.text_top = view.aligned(
      self.reader.cursor,
      fraction,
      &mut PdfText(&mut self.doc),
    )?;
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
    let view = self.next_text_view()?;
    self.text_top =
      view.keep_visible(self.reader.cursor, &mut PdfText(&mut self.doc))?;
    Ok(())
  }

  /// Text `land`: the cursor one row below the top, as on pages.
  pub(crate) fn land_text(&mut self) -> Result<()> {
    let cursor = self.reader.cursor;
    if !self.next_text_view()?.inside(cursor) {
      let top = self.text_step(Position { glyph: 0, ..cursor }, -1)?;
      self.text_top = self.clamp_text(top)?;
    }
    self.reveal_text()
  }

  pub(crate) fn nearest_text(
    &mut self,
    point: (f32, f32),
  ) -> Result<Option<Position>> {
    let view = self.next_text_view()?;
    let mut nearest = None;
    let mut distance = (f32::INFINITY, f32::INFINITY);
    for (row, item) in view.rows().enumerate() {
      let Row::Line(at, line) = item else { continue };
      for (glyph, column, item) in view.shown(&line.glyphs) {
        if item.ch.is_whitespace() {
          continue;
        }
        let next = (
          (row as f32 - point.1).abs(),
          (column as f32 + f32::from(self.left) - point.0).abs(),
        );
        if next < distance {
          distance = next;
          nearest = Some(Position { glyph, ..at });
        }
      }
    }
    Ok(nearest)
  }

  pub fn scroll_text(&mut self, lines: isize) -> Result<()> {
    let top = self.text_step(self.text_top, lines)?;
    self.text_top = self.clamp_text(top)?;
    Ok(())
  }

  pub(crate) fn pan_text(&mut self, columns: isize) -> Result<()> {
    self.text_top.glyph = self.text_top.glyph.saturating_add_signed(columns);
    self.text_top = self.clamp_text(self.text_top)?;
    Ok(())
  }

  fn text_style(&self, position: Position, linked: bool) -> &'static str {
    let flashed = self.click_mark() == Some(position);
    if flashed || self.reader.uses_cursor() && position == self.reader.cursor {
      return theme::TEXT_CURSOR;
    }
    if self.selects(position) {
      return theme::TEXT_SELECTION;
    }
    if !self.search.hidden()
      && crate::search::contains(self.search.hits(), position)
    {
      return theme::TEXT_MATCH;
    }
    if linked {
      return theme::TEXT_LINK;
    }
    self.line_style(position)
  }

  /// Whether the selection covers `position`.
  fn selects(&self, position: Position) -> bool {
    self.reader.selection.is_some_and(|selection| {
      let range =
        vpdf_reader::motion::range(selection.anchor, self.reader.cursor);
      let linewise = selection.kind == SelectionKind::Line;
      vpdf_reader::motion::covers(range, linewise, position)
    })
  }

  fn line_style(&self, position: Position) -> &'static str {
    if self.landing.as_ref().is_some_and(|landing| {
      landing.span == Span::Line
        && position.page == landing.position.page
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
  use crate::text::{Bounds, Line};

  fn text(rows: &[&str]) -> Arc<Text> {
    let lines = rows.iter().map(|row| Line {
      paragraph: false,
      glyphs: row
        .chars()
        .map(|ch| Glyph { ch, bounds: Bounds::default() })
        .collect(),
    });
    Arc::new(Text { lines: lines.collect(), truncated: false })
  }

  fn at(page: usize, line: usize, glyph: usize) -> Position {
    Position { page: PageId(page), line, glyph }
  }

  fn view(
    pages: &mut Vec<Arc<Text>>,
    (height, columns): (usize, usize),
    top: Position,
  ) -> View {
    View::new(pages, (height, columns), top).expect("view")
  }

  /// The origin a `(height, columns)` view at `top` keeps for `cursor`.
  fn kept(
    pages: &[Arc<Text>],
    top: Position,
    cursor: Position,
    size: (usize, usize),
  ) -> Result<Position> {
    let mut pages = pages.to_vec();
    view(&mut pages, size, top).keep_visible(cursor, &mut pages)
  }

  #[test]
  fn view_moves_only_to_show_the_cursor() -> Result<()> {
    let mut rows = vec!["abcdefghij"; 28];
    rows.extend(["日本語日本語", ""]);
    let one = [text(&rows)];
    for (name, top, cursor, size, expected) in [
      ("inside", at(0, 5, 2), at(0, 9, 5), (10, 4), at(0, 5, 2)),
      ("first row", at(0, 5, 0), at(0, 5, 0), (10, 4), at(0, 5, 0)),
      ("last row", at(0, 5, 0), at(0, 14, 0), (10, 4), at(0, 5, 0)),
      ("above", at(0, 5, 0), at(0, 3, 0), (10, 4), at(0, 3, 0)),
      ("below", at(0, 5, 0), at(0, 15, 0), (10, 4), at(0, 6, 0)),
      ("past the end", at(0, 25, 0), at(0, 26, 0), (10, 4), at(0, 20, 0)),
      ("short page", at(0, 5, 0), at(0, 7, 0), (40, 4), at(0, 0, 0)),
      ("left", at(0, 0, 5), at(0, 0, 2), (10, 4), at(0, 0, 2)),
      ("last column", at(0, 0, 0), at(0, 0, 3), (10, 4), at(0, 0, 0)),
      ("right", at(0, 0, 0), at(0, 0, 7), (10, 4), at(0, 0, 4)),
      ("wide glyphs", at(0, 20, 0), at(0, 28, 4), (10, 4), at(0, 20, 6)),
      ("empty line", at(0, 20, 5), at(0, 29, 0), (10, 4), at(0, 20, 0)),
      ("no rows", at(0, 5, 2), at(0, 15, 7), (0, 4), at(0, 5, 2)),
      ("no columns", at(0, 5, 2), at(0, 15, 7), (10, 0), at(0, 5, 2)),
    ] {
      assert_eq!(kept(&one, top, cursor, size)?, expected, "{name}");
    }
    Ok(())
  }

  #[test]
  fn view_follows_the_cursor_across_pages() -> Result<()> {
    let six = ["abcdefghij"; 6];
    let three = [text(&six), text(&six), text(&six[..3])];
    for (name, top, cursor, expected) in [
      ("next page", at(0, 0, 0), at(1, 2, 0), at(0, 6, 0)),
      ("page above", at(1, 3, 0), at(0, 4, 0), at(0, 4, 0)),
      ("over a break", at(0, 4, 0), at(1, 0, 0), at(0, 4, 0)),
      ("same page below", at(1, 0, 0), at(1, 5, 0), at(1, 2, 0)),
      ("pages away", at(0, 0, 0), at(2, 1, 0), at(1, 5, 0)),
    ] {
      assert_eq!(kept(&three, top, cursor, (4, 4))?, expected, "{name}");
    }
    Ok(())
  }

  #[test]
  fn rows_step_over_breaks_and_stop_at_the_ends() -> Result<()> {
    // Lines, break, blank, break, lines.
    let mut pages = vec![text(&["a", "b", "c"]), text(&[]), text(&["d", "e"])];
    for (name, from, rows, expected) in [
      ("onto a break", at(0, 1, 0), 2, (at(0, 3, 0), 2)),
      ("onto an empty page", at(0, 1, 0), 3, (at(1, 0, 0), 3)),
      ("past an empty page", at(0, 1, 0), 5, (at(2, 0, 0), 5)),
      ("past the end", at(0, 1, 0), 10, (at(2, 1, 0), 6)),
      ("back over breaks", at(2, 0, 2), -3, (at(0, 3, 2), 3)),
      ("past the start", at(2, 1, 0), -10, (at(0, 0, 0), 7)),
      ("still", at(1, 0, 0), 0, (at(1, 0, 0), 0)),
    ] {
      assert_eq!(step(&mut pages, from, rows)?, expected, "{name}");
    }
    for (name, top, size, expected) in [
      ("fills from the end", at(2, 1, 0), (4, 4), at(1, 0, 0)),
      ("past a page's rows", at(0, 9, 0), (2, 4), at(0, 3, 0)),
      ("past the pages", at(5, 0, 0), (2, 4), at(2, 0, 0)),
      ("short document", at(1, 0, 0), (9, 4), at(0, 0, 0)),
      ("no rows", at(2, 1, 0), (0, 4), at(2, 1, 0)),
      ("narrow lines", at(0, 0, 3), (2, 4), at(0, 0, 0)),
    ] {
      assert_eq!(clamp(&mut pages, top, size)?, expected, "{name}");
    }
    Ok(())
  }

  #[test]
  fn clicks_and_cells_match_drawn_glyphs() {
    let mut marks = vec![text(&["abcd\u{301}"])];
    let mut pages =
      vec![text(&["abc日", "abcdefghij", "日本語日本語", "", "xyz"])];
    let mut shown =
      |height, glyph| view(&mut pages, (height, 4), at(0, 0, glyph));
    for (name, left, (column, row), expected) in [
      ("first glyph", 0, (0, 0), at(0, 0, 0)),
      ("blank by a cut wide glyph", 0, (3, 0), at(0, 0, 2)),
      ("past the last column", 0, (9, 1), at(0, 1, 3)),
      ("wide glyph's second cell", 0, (1, 2), at(0, 2, 0)),
      ("next wide glyph", 0, (2, 2), at(0, 2, 1)),
      ("empty line", 0, (0, 3), at(0, 3, 0)),
      ("below the view", 0, (0, 6), at(0, 3, 0)),
      ("scrolled right", 2, (0, 1), at(0, 1, 2)),
      ("row left of the view", 5, (1, 0), at(0, 0, 3)),
    ] {
      let position = shown(3, left).position(column, row);
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
      assert_eq!(shown(3, left).cell(position), expected, "{name}");
    }
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
      assert_eq!(shown(5, left).hit(column, row), expected, "{name}");
    }
    assert_eq!(shown(2, 0).hit(0, 2), None, "below a short view");
    let mark = view(&mut marks, (3, 4), at(0, 0, 0));
    assert_eq!(mark.cell(at(0, 0, 4)), Some((3, 0)), "mark");
    assert_eq!(mark.hit(3, 0), Some(at(0, 0, 3)), "mark");
  }

  /// Rows past a page's lines show the next page after a break.
  #[test]
  fn rows_map_across_page_breaks() -> Result<()> {
    let mut pages = vec![text(&["ab", "cd"]), text(&["ef", "gh"])];
    let shown = view(&mut pages, (4, 4), at(0, 1, 0));
    for (name, cell, position, hit) in [
      ("line above the break", (1, 0), at(0, 1, 1), Some(at(0, 1, 1))),
      ("the break", (1, 1), at(0, 1, 1), None),
      ("next page", (1, 2), at(1, 0, 1), Some(at(1, 0, 1))),
      ("below the rows", (0, 5), at(1, 1, 0), None),
    ] {
      assert_eq!(shown.position(cell.0, cell.1), position, "{name}");
      assert_eq!(shown.hit(cell.0, cell.1), hit, "{name}");
    }
    for (name, position, cell, inside) in [
      ("first row", at(0, 1, 1), Some((1, 0)), false),
      ("after the break", at(1, 0, 0), Some((0, 2)), true),
      ("last row", at(1, 1, 1), Some((1, 3)), false),
      ("above the view", at(0, 0, 0), None, false),
    ] {
      assert_eq!(shown.cell(position), cell, "{name}");
      assert_eq!(shown.inside(position), inside, "{name}");
    }
    assert_eq!(shown.shown_pages(), 0..2);
    let mut empty = vec![text(&["ab"]), text(&[]), text(&["cd"])];
    let blank = view(&mut empty, (4, 4), at(0, 0, 0));
    assert_eq!(blank.position(1, 2), at(1, 0, 0), "blank row");
    assert_eq!(blank.position(1, 3), at(1, 0, 0), "empty page's break");
    assert_eq!(blank.hit(0, 2), None, "blank row");
    assert!(blank.inside(at(1, 0, 0)), "blank row");
    assert_eq!(blank.keep_visible(at(1, 0, 0), &mut empty)?, at(0, 0, 0));
    assert_eq!(view(&mut pages, (1, 4), at(0, 1, 0)).shown_pages(), 0..1);
    Ok(())
  }

  #[test]
  fn aligning_puts_the_line_at_a_fraction() -> Result<()> {
    let one = vec![text(&["abcd"; 40])];
    let two = vec![text(&["abcd"; 5]), text(&["abcd"; 5])];
    for (pages, height, fraction, line, top) in [
      (&one, 10, 0.0, at(0, 30, 0), at(0, 30, 0)),
      (&one, 10, 0.5, at(0, 30, 0), at(0, 25, 0)),
      (&one, 10, 1.0, at(0, 30, 0), at(0, 21, 0)),
      (&one, 10, 1.0, at(0, 3, 0), at(0, 0, 0)),
      (&two, 4, 1.0, at(1, 1, 0), at(0, 4, 0)),
    ] {
      let mut pages = pages.clone();
      let shown = view(&mut pages, (height, 4), at(0, 0, 0));
      let top_at = shown.aligned(line, fraction, &mut pages)?;
      assert_eq!(top_at, top, "{fraction} {line:?}");
    }
    Ok(())
  }
}
