use crate::Result;
use crate::paint::Marks;
use crate::text::{Bounds, Glyph, Line, PageId, Position, Text};
use std::sync::Arc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vpdf_model::geometry::At;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::{Character, Draw, Op};
use vpdf_model::style::Role;
use vpdf_reader::reader::{SelectionKind, TextSource};

mod wrap;

/// Pending and drawn origins can differ.
#[derive(Default)]
pub struct TextView {
  top: Position,
  drawn: Option<Position>,
  wrapped: bool,
}

#[derive(Clone, Copy)]
pub struct Frame {
  pub screen: Screen,
  pub at: At,
  pub left: u16,
  /// Rows above the text: the header, when a text row fits below it.
  pub header: u16,
}

impl Frame {
  /// A pane's text view; cells stay pane-relative.
  pub fn new(screen: Screen, at: At, left: u16) -> Self {
    // Header, a text row, and the status row.
    let header = u16::from(screen.rows > 2);
    Self { screen, at, left, header }
  }

  /// Text rows between the header and the status row.
  pub fn rows(self) -> u16 {
    self.screen.rows.saturating_sub(1 + self.header)
  }

  /// The view row at a pane row; none on the header.
  fn view_row(self, row: u16) -> Option<usize> {
    row.checked_sub(self.header).map(usize::from)
  }
}

pub struct Pane<'a, S> {
  state: &'a mut TextView,
  source: S,
  frame: Frame,
}

impl TextView {
  pub fn set_wrap(&mut self, wrapped: bool) {
    self.wrapped = wrapped;
    self.invalidate();
  }

  pub fn top(&self) -> Position {
    self.top
  }

  pub fn invalidate(&mut self) {
    self.drawn = None;
  }

  /// Preserve this pane's drawn origin.
  pub fn copy_origin(&mut self, other: &Self) {
    self.top = other.top;
  }

  pub fn bind<S>(&mut self, source: S, frame: Frame) -> Pane<'_, S> {
    Pane { state: self, source, frame }
  }
}

/// A line that stands in for an empty page.
static BLANK: Line = Line { ocr: None, paragraph: false, glyphs: Vec::new() };

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
  Ok(View::new(pages, size, top, false)?.clamped().top)
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
  wrapped: bool,
}

impl View {
  /// `height` rows and `columns` columns from `top`.
  fn new(
    pages: &mut impl TextSource,
    (height, columns): (usize, usize),
    top: Position,
    wrapped: bool,
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
    Ok(Self { height, columns, top, first, more, count, wrapped })
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
    let (wrapped, columns) = (self.wrapped, self.columns);
    self.texts().flat_map(move |(page, text)| {
      let start = if page == top.page { top.line } else { 0 };
      let blank = text.lines.is_empty().then_some(&BLANK);
      let lines = text.lines.iter().chain(blank).enumerate().skip(start);
      let lines = lines.flat_map(move |(line, row)| {
        let starts =
          if wrapped { wrap::starts(&row.glyphs, columns) } else { vec![0] };
        starts
          .into_iter()
          .filter(move |&glyph| {
            !wrapped
              || page != top.page
              || line != top.line
              || glyph >= top.glyph
          })
          .map(move |glyph| Row::Line(Position { page, line, glyph }, row))
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
  fn row_of(&self, position: Position) -> Option<(usize, Position, &Line)> {
    self.rows().enumerate().find_map(|(row, shown)| match shown {
      Row::Line(at, line)
        if at.page == position.page
          && at.line == position.line
          && (!self.wrapped
            || line.glyphs.is_empty()
            || self
              .shown(at, &line.glyphs)
              .any(|(index, ..)| index == position.glyph)) =>
      {
        Some((row, at, line))
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
    at: Position,
    glyphs: &'a [Glyph],
  ) -> impl Iterator<Item = (usize, usize, &'a Glyph)> {
    let start = if self.wrapped { at.glyph } else { 0 };
    let offset = if self.wrapped { 0 } else { self.top.glyph };
    let end = if self.wrapped {
      wrap::starts(glyphs, self.columns)
        .into_iter()
        .find(|&next| next > start)
        .unwrap_or(glyphs.len())
    } else {
      glyphs.len()
    };
    fitting(
      glyphs.iter().enumerate().take(end).skip(start),
      offset.saturating_add(self.columns),
    )
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
        let shown = self.shown(at, &line.glyphs);
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
    let (glyph, ..) =
      self.shown(at, &line.glyphs).find(|&(_, start, glyph)| {
        (start..start + cells(glyph)).contains(&column)
      })?;
    Some(Position { glyph, ..at })
  }

  /// Cell of `position`, when drawn.
  fn cell(&self, position: Position) -> Option<(usize, usize)> {
    let (row, at, line) = self.row_of(position)?;
    let mut shown = self.shown(at, &line.glyphs);
    let (_, column, _) = shown.find(|&(index, ..)| index == position.glyph)?;
    // A zero-width mark joins the last cell.
    Some((column.min(self.columns.saturating_sub(1)), row))
  }

  /// Column offset no wider than the drawn lines need.
  fn clamped(mut self) -> Self {
    if self.wrapped {
      return self;
    }
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
    if self.wrapped {
      let top = if self.row_of(cursor).is_some() {
        self.top
      } else {
        let shift =
          if cursor < self.top { 0 } else { 1 - self.height as isize };
        wrap::step(pages, cursor, shift, self.columns)?.0
      };
      return wrap::clamp(pages, top, (self.height, self.columns));
    }
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
    if self.wrapped {
      let above = (fraction * self.height.saturating_sub(1) as f32).round();
      let top = wrap::step(pages, at, -(above as isize), self.columns)?.0;
      return wrap::clamp(pages, top, (self.height, self.columns));
    }
    let above = (fraction * self.height.saturating_sub(1) as f32).round();
    let at = Position { glyph: self.top.glyph, ..at };
    let top = step(pages, at, -(above as isize))?.0;
    clamp(pages, top, (self.height, self.columns))
  }

  /// Whether `cursor`'s line clears the edge rows.
  fn inside(&self, cursor: Position) -> bool {
    self
      .row_of(cursor)
      .is_some_and(|(row, ..)| row > 0 && row + 2 <= self.height)
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

impl<S: TextSource + crate::source::Pages> Pane<'_, S> {
  pub fn clamp(&mut self, top: Position) -> Result<Position> {
    let size = self.text_size();
    if self.state.wrapped {
      wrap::clamp(&mut self.source, top, size)
    } else {
      clamp(&mut self.source, top, size)
    }
  }

  /// Clamp the restored origin.
  pub fn restore(&mut self, top: Position) -> Result<()> {
    self.state.top = self.clamp(top)?;
    Ok(())
  }

  pub fn reclamp(&mut self) -> Result<()> {
    self.restore(self.state.top)
  }

  /// Text view rows and columns.
  fn text_size(&self) -> (usize, usize) {
    let height = usize::from(self.frame.rows());
    let columns =
      usize::from(self.frame.screen.columns.saturating_sub(self.frame.left));
    (height, columns)
  }

  /// The text view from `top`, unmoved.
  fn text_view_from(&mut self, top: Position) -> Result<View> {
    let size = self.text_size();
    View::new(&mut self.source, size, top, self.state.wrapped)
  }

  /// The next frame's view.
  fn next_text_view(&mut self) -> Result<View> {
    self.text_view_from(self.state.top)
  }

  /// `at` moved `rows` text view rows.
  fn text_step(&mut self, at: Position, rows: isize) -> Result<Position> {
    if self.state.wrapped {
      let columns = self.text_size().1;
      Ok(wrap::step(&mut self.source, at, rows, columns)?.0)
    } else {
      Ok(step(&mut self.source, at, rows)?.0)
    }
  }

  /// Pages the next frame shows.
  pub fn pages(&mut self) -> Result<std::ops::Range<usize>> {
    Ok(self.next_text_view()?.shown_pages())
  }

  /// First drawn glyph that is not a space.
  pub fn first(&mut self) -> Result<Option<Position>> {
    let view = self.next_text_view()?;
    Ok(view.rows().find_map(|row| {
      let Row::Line(at, line) = row else { return None };
      let mut shown = view.shown(at, &line.glyphs);
      let (glyph, ..) =
        shown.find(|(_, _, glyph)| !glyph.ch.is_whitespace())?;
      Some(Position { glyph, ..at })
    }))
  }

  /// Draws the rows; the pages they show.
  pub fn scene<'a>(
    &mut self,
    scene: &mut impl Draw<'a>,
    paint: &Marks<'_>,
  ) -> Result<std::ops::Range<usize>> {
    let view = self.next_text_view()?;
    self.state.top = view.top;
    self.state.drawn = Some(view.top);
    let hovered = paint.hovered;
    for (row, shown) in view.rows().enumerate() {
      let row = row + usize::from(self.frame.header);
      crate::paint::move_to(scene, self.frame.at, row, self.frame.left);
      let (at, line) = match shown {
        Row::Line(at, line) => (at, line),
        Row::Break(at) => {
          self.scene_break(scene, at, view.columns, paint);
          continue;
        }
      };
      let band = paint.line_style(at);
      if band != Role::Reset {
        scene.extend([Op::Style(band), Op::Spaces(view.columns)]);
        crate::paint::move_to(scene, self.frame.at, row, self.frame.left);
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
      let mut current = None;
      let mut written = 0;
      for (glyph, column, ch) in view.shown(at, &line.glyphs) {
        if column > written {
          scene.push(Op::Spaces(column - written));
        }
        written = column + cells(ch);
        let pos = Position { glyph, ..at };
        let linked = near.iter().any(|(_, bounds)| {
          let (x, y) = ch.bounds.centre();
          bounds.contains(x, y)
        });
        let style = paint.text_style(pos, linked);
        let style =
          if style == Role::Reset && paint.dim { Role::Dim } else { style };
        if Some(style) != current {
          scene.push(Op::Style(style));
          current = Some(style);
        }
        if let Some(character) = Character::new(ch.ch) {
          scene.push(Op::Character(character));
        }
      }
      scene.push(Op::Style(Role::Reset));
    }
    Ok(view.shown_pages())
  }

  /// A rule naming the next page, at break `at`.
  fn scene_break<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: Position,
    columns: usize,
    paint: &Marks<'_>,
  ) {
    let style = if paint.selects(at) { Role::TextSelection } else { Role::Dim };
    let label = self.source.label(PageId(at.page.0 + 1));
    let head = vpdf_model::text::clean(&format!("── {label} "), columns);
    let fill = columns.saturating_sub(head.width());
    scene.push(Op::Style(style));
    scene.text(&head);
    scene.text(&"─".repeat(fill));
    scene.push(Op::Style(Role::Reset));
  }

  /// Glyph `view` draws at a pane cell.
  fn glyph_in(&self, view: &View, column: u16, row: u16) -> Option<Position> {
    let column = usize::from(column.saturating_sub(self.frame.left));
    view.hit(column, self.frame.view_row(row)?)
  }

  /// The view on screen.
  fn shown_text_view(&mut self) -> Result<View> {
    match self.state.drawn {
      Some(top) => self.text_view_from(top),
      None => self.next_text_view(),
    }
  }

  /// The position at a pane cell; the header reads as the first row.
  pub fn position(&mut self, column: u16, row: u16) -> Result<Position> {
    let view = self.shown_text_view()?;
    let column = usize::from(column.saturating_sub(self.frame.left));
    Ok(view.position(column, self.frame.view_row(row).unwrap_or(0)))
  }

  /// Whether a pane row shows no text: a page break or the header.
  pub fn is_break(&mut self, row: u16) -> Result<bool> {
    let Some(row) = self.frame.view_row(row) else { return Ok(true) };
    let view = self.shown_text_view()?;
    Ok(matches!(view.rows().nth(row), Some(Row::Break(_))))
  }

  /// Next frame's row of `position`'s line.
  pub fn row(&mut self, position: Position) -> Result<Option<usize>> {
    let view = self.next_text_view()?;
    Ok(view.row_of(position).map(|(row, ..)| row))
  }

  /// Glyph at a shown cell.
  pub fn shown(&mut self, column: u16, row: u16) -> Result<Option<Position>> {
    let shown = self.shown_text_view()?;
    Ok(self.glyph_in(&shown, column, row))
  }

  /// Glyph at a cell of the next frame.
  pub fn hit(&mut self, column: u16, row: u16) -> Result<Option<Position>> {
    // Hover runs before the frame draws.
    let next = self.next_text_view()?;
    Ok(self.glyph_in(&next, column, row))
  }

  /// Screen cell of `position` in the text view, when shown.
  pub fn cell(&mut self, position: Position) -> Result<Option<(u16, u16)>> {
    let view = self.next_text_view()?;
    let Some((column, row)) = view.cell(position) else {
      return Ok(None);
    };
    let row = self.frame.header + u16::try_from(row)?;
    Ok(Some((self.frame.left + u16::try_from(column)?, row)))
  }

  /// Cursor line `fraction` down the view.
  pub fn align(&mut self, cursor: Position, fraction: f32) -> Result<()> {
    let view = self.next_text_view()?;
    self.state.top = view.aligned(cursor, fraction, &mut self.source)?;
    Ok(())
  }

  pub fn origin(&mut self, position: Position) -> Result<()> {
    if self.state.wrapped {
      self.state.top = self.clamp(position)?;
      return Ok(());
    }
    let text = self.source.page_text(position.page)?;
    let column = text.lines.get(position.line).map_or(0, |line| {
      line.glyphs.iter().take(position.glyph).map(cells).sum()
    });
    self.state.top = self.clamp(Position { glyph: column, ..position })?;
    Ok(())
  }

  pub fn reveal(&mut self, cursor: Position) -> Result<()> {
    let view = self.next_text_view()?;
    self.state.top = view.keep_visible(cursor, &mut self.source)?;
    Ok(())
  }

  /// Text `land`: the cursor one row below the top, as on pages.
  pub fn land(&mut self, cursor: Position) -> Result<()> {
    if !self.next_text_view()?.inside(cursor) {
      let top = self.text_step(Position { glyph: 0, ..cursor }, -1)?;
      self.state.top = self.clamp(top)?;
    }
    self.reveal(cursor)
  }

  /// Glyph nearest a pane `point`, which counts the header row.
  pub fn nearest(&mut self, point: (f32, f32)) -> Result<Option<Position>> {
    let view = self.next_text_view()?;
    let header = f32::from(self.frame.header);
    let mut nearest = None;
    let mut distance = (f32::INFINITY, f32::INFINITY);
    for (row, item) in view.rows().enumerate() {
      let Row::Line(at, line) = item else { continue };
      for (glyph, column, item) in view.shown(at, &line.glyphs) {
        if item.ch.is_whitespace() {
          continue;
        }
        let next = (
          (row as f32 + header - point.1).abs(),
          (column as f32 + f32::from(self.frame.left) - point.0).abs(),
        );
        if next < distance {
          distance = next;
          nearest = Some(Position { glyph, ..at });
        }
      }
    }
    Ok(nearest)
  }

  pub fn scroll(&mut self, lines: isize) -> Result<()> {
    let top = self.text_step(self.state.top, lines)?;
    self.state.top = self.clamp(top)?;
    Ok(())
  }

  pub fn pan(&mut self, columns: isize) -> Result<()> {
    if self.state.wrapped {
      return Ok(());
    }
    self.state.top.glyph = self.state.top.glyph.saturating_add_signed(columns);
    self.state.top = self.clamp(self.state.top)?;
    Ok(())
  }
}
impl Marks<'_> {
  fn text_style(&self, position: Position, linked: bool) -> Role {
    let flashed = self.flash == Some(position);
    if flashed || self.reader.uses_cursor() && position == self.reader.cursor {
      return Role::TextCursor;
    }
    if self.selects(position) {
      return Role::TextSelection;
    }
    if vpdf_search::search::contains(self.matches, position) {
      return Role::TextMatch;
    }
    if linked {
      return Role::TextLink;
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

  fn line_style(&self, position: Position) -> Role {
    if self.landing.as_ref().is_some_and(|landing| {
      position.page == landing.page && position.line == landing.line
    }) {
      return Role::TextLanding;
    }
    if self.on_cursorline(position) {
      return Role::TextCursorline;
    }
    Role::Reset
  }

  fn on_cursorline(&self, position: Position) -> bool {
    self.cursorline
      && self.reader.uses_cursor()
      && position.page == self.reader.cursor.page
      && position.line == self.reader.cursor.line
  }
}

/// The text view's top row: the pages it shows and the copy modes.
pub struct Header<'a> {
  pub first: &'a str,
  pub last: &'a str,
}

impl Header<'_> {
  pub fn scene<'a>(&self, scene: &mut impl Draw<'a>, at: At, columns: u16) {
    let pages = if self.first == self.last {
      format!(" p{}", self.first)
    } else {
      format!(" p{}–{}", self.first, self.last)
    };
    let context = format!("{pages} joined · yY raw");
    crate::paint::move_to(scene, at, 0u16, 0u16);
    let title = vpdf_model::text::clean(" TEXT ", usize::from(columns));
    let room = usize::from(columns).saturating_sub(title.width());
    let context = vpdf_model::text::clean(&context, room);
    scene.push(Op::Style(Role::Badge(vpdf_model::style::Color::Cyan)));
    scene.text(&title);
    scene.push(Op::Style(Role::Muted));
    scene.text(&context);
    scene.extend([Op::Spaces(room - context.width()), Op::Style(Role::Reset)]);
  }
}

#[cfg(test)]
mod tests;
