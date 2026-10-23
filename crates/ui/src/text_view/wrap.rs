use super::{Glyph, PageId, Position, Result, Text, TextSource, cells};

pub(super) fn starts(glyphs: &[Glyph], columns: usize) -> Vec<usize> {
  let mut starts = vec![0];
  let mut start = 0;
  while start < glyphs.len() {
    let mut width = 0;
    let mut end = start;
    let mut space = None;
    while let Some(glyph) = glyphs.get(end) {
      if width + cells(glyph) > columns.max(1) && end > start {
        break;
      }
      width += cells(glyph);
      end += 1;
      if glyph.ch.is_whitespace() {
        space = Some(end);
      }
    }
    if end >= glyphs.len() {
      break;
    }
    start = space.filter(|&space| space > start).unwrap_or(end);
    starts.push(start);
  }
  starts
}

fn rows(
  text: &Text,
  page: PageId,
  count: usize,
  columns: usize,
) -> Vec<Position> {
  let mut rows: Vec<_> = text
    .lines
    .iter()
    .enumerate()
    .flat_map(|(line, row)| {
      starts(&row.glyphs, columns).into_iter().map(move |glyph| Position {
        page,
        line,
        glyph,
      })
    })
    .collect();
  if rows.is_empty() {
    rows.push(Position { page, ..Position::default() });
  }
  if page.0 + 1 < count {
    rows.push(Position { page, line: text.lines.len().max(1), glyph: 0 });
  }
  rows
}

pub(super) fn step(
  pages: &mut impl TextSource,
  mut at: Position,
  delta: isize,
  columns: usize,
) -> Result<(Position, usize)> {
  let count = pages.page_count();
  at.page.0 = at.page.0.min(count.saturating_sub(1));
  let mut left = delta.unsigned_abs();
  let mut moved = 0;
  loop {
    let text = pages.page_text(at.page)?;
    let rows = rows(&text, at.page, count, columns);
    let index = rows.partition_point(|&row| row <= at).saturating_sub(1);
    let room = if delta < 0 { index } else { rows.len() - index - 1 };
    let advance = room.min(left);
    at = rows[if delta < 0 { index - advance } else { index + advance }];
    left -= advance;
    moved += advance;
    if left == 0
      || delta < 0 && at.page.0 == 0
      || delta >= 0 && at.page.0 + 1 >= count
    {
      return Ok((at, moved));
    }
    left -= 1;
    moved += 1;
    at = if delta < 0 {
      Position {
        page: PageId(at.page.0 - 1),
        line: usize::MAX,
        glyph: usize::MAX,
      }
    } else {
      Position { page: PageId(at.page.0 + 1), ..Position::default() }
    };
  }
}

pub(super) fn clamp(
  pages: &mut impl TextSource,
  top: Position,
  (height, columns): (usize, usize),
) -> Result<Position> {
  let top = step(pages, top, 0, columns)?.0;
  let wanted = height.saturating_sub(1);
  let (_, ahead) = step(pages, top, wanted as isize, columns)?;
  Ok(step(pages, top, -((wanted - ahead) as isize), columns)?.0)
}
