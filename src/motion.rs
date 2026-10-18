use crate::Result;
use crate::reader::{Reader, TextSource};
use crate::text::{PageId, Position, class};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Motion {
  Left,
  Right,
  Down,
  Up,
  Word,
  BackWord,
  EndWord,
  BigWord,
  BackBigWord,
  EndBigWord,
  PreviousBigEnd,
  NextSentence,
  PreviousSentence,
  NextParagraph,
  PreviousParagraph,
  LineStart,
  FirstNonblank,
  LineEnd,
  First,
  Last,
  NextPage,
  PreviousPage,
}

impl Reader {
  pub fn motion(
    &mut self,
    motion: Motion,
    count: usize,
    explicit: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let mut start = self.cursor;
    let count =
      count.saturating_mul(self.operator.map_or(1, |o| o.count)).min(10_000);
    if !matches!(motion, Motion::Down | Motion::Up) {
      self.desired = None;
    }
    let boundary = matches!(
      motion,
      Motion::First | Motion::Last | Motion::NextPage | Motion::PreviousPage
    );
    let mut paragraph_boundary = false;
    let mut lexical_boundary = false;
    if boundary {
      self.boundary(motion, count, explicit, source)?;
    } else if motion == Motion::LineEnd {
      for _ in 1..count {
        self.step(1, true, source)?;
      }
      self.small_motion(motion, source)?;
      self.desired = self.operator.is_none().then_some(usize::MAX);
    } else {
      for _ in 0..count {
        if matches!(motion, Motion::NextParagraph | Motion::PreviousParagraph) {
          paragraph_boundary =
            self.paragraph(motion == Motion::NextParagraph, source)?;
        } else if matches!(
          motion,
          Motion::BigWord
            | Motion::BackBigWord
            | Motion::EndBigWord
            | Motion::PreviousBigEnd
            | Motion::NextSentence
            | Motion::PreviousSentence
        ) {
          lexical_boundary = self.lexical_motion(motion, source)?;
        } else {
          self.small_motion(motion, source)?;
        }
      }
    }
    if let Some(op) = self.operator.take() {
      if (motion == Motion::Word && self.cursor > start)
        || (matches!(motion, Motion::BigWord | Motion::NextSentence)
          && lexical_boundary)
        || (motion == Motion::NextParagraph && paragraph_boundary)
      {
        self.step(-1, false, source)?;
      }
      if matches!(
        motion,
        Motion::PreviousParagraph
          | Motion::PreviousSentence
          | Motion::BackBigWord
      ) {
        let destination = self.cursor;
        self.cursor = start;
        if !self.step(-1, false, source)? || destination > self.cursor {
          self.cursor = start;
          return Ok(());
        }
        start = destination;
      }
      let linewise = boundary || matches!(motion, Motion::Down | Motion::Up);
      self.yank_range(start, self.cursor, linewise, op.register, source)?;
      self.cursor = start;
    }
    Ok(())
  }

  fn boundary(
    &mut self,
    motion: Motion,
    count: usize,
    explicit: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let last_page = source.page_count().saturating_sub(1);
    let page = match motion {
      Motion::First | Motion::Last if explicit => count.saturating_sub(1),
      Motion::First => 0,
      Motion::Last => last_page,
      Motion::NextPage => self.cursor.page.0.saturating_add(count),
      Motion::PreviousPage => self.cursor.page.0.saturating_sub(count),
      _ => self.cursor.page.0,
    }
    .min(last_page);
    if self.operator.is_none() {
      self.remember();
    }
    self.edge(PageId(page), motion == Motion::Last && !explicit, source)
  }

  fn small_motion(
    &mut self,
    motion: Motion,
    source: &mut impl TextSource,
  ) -> Result<()> {
    match motion {
      Motion::Left => {
        self.step(-1, false, source)?;
      }
      Motion::Right => {
        self.step(1, false, source)?;
      }
      Motion::Down => {
        self.step(1, true, source)?;
      }
      Motion::Up => {
        self.step(-1, true, source)?;
      }
      Motion::Word | Motion::BackWord | Motion::EndWord => {
        self.word(motion, source)?;
      }
      Motion::LineStart | Motion::FirstNonblank | Motion::LineEnd => {
        let text = source.page_text(self.cursor.page)?;
        if let Some(line) = text.lines.get(self.cursor.line) {
          self.cursor.glyph = match motion {
            Motion::LineEnd => line.glyphs.len().saturating_sub(1),
            Motion::FirstNonblank => line
              .glyphs
              .iter()
              .position(|g| !g.ch.is_whitespace())
              .unwrap_or(0),
            _ => 0,
          };
        }
      }
      _ => {}
    }
    Ok(())
  }

  pub fn step(
    &mut self,
    direction: i32,
    vertical: bool,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    let old = self.cursor;
    let text = source.page_text(old.page)?;
    let Some(line) = text.lines.get(old.line) else {
      return self.step_page(direction, source);
    };
    let next = old.glyph as i64 + i64::from(direction);
    if !vertical && next >= 0 && (next as usize) < line.glyphs.len() {
      self.cursor.glyph = next as usize;
      return Ok(true);
    }
    let desired = self.desired.unwrap_or(old.glyph);
    let next_line = old.line as i64 + i64::from(direction);
    if next_line < 0 || next_line as usize >= text.lines.len() {
      if !self.step_page(direction, source)? {
        return Ok(false);
      }
    } else {
      self.cursor.line = next_line as usize;
      self.cursor.glyph = if direction > 0 { 0 } else { usize::MAX };
    }
    if vertical {
      self.cursor.glyph = desired;
      self.desired = Some(desired);
    }
    self.cursor = source.page_text(self.cursor.page)?.clamp(self.cursor);
    Ok(self.cursor != old)
  }

  fn step_page(
    &mut self,
    direction: i32,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    let next = self.cursor.page.0 as i64 + i64::from(direction);
    if next < 0 || next as usize >= source.page_count() {
      return Ok(false);
    }
    self.edge(PageId(next as usize), direction < 0, source)?;
    Ok(true)
  }

  pub(crate) fn character(&self, source: &mut impl TextSource) -> Result<char> {
    Ok(
      source
        .page_text(self.cursor.page)?
        .glyph(self.cursor)
        .map_or(' ', |g| g.ch),
    )
  }

  fn word(
    &mut self,
    motion: Motion,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let direction = if motion == Motion::BackWord { -1 } else { 1 };
    if motion != Motion::Word
      && !(self.step(direction, false, source)?
        && self.skip_blank(direction, source)?)
    {
      return Ok(());
    }
    let initial = class(self.character(source)?);
    for _ in 0..100_000 {
      let previous = self.cursor;
      if !self.step(direction, false, source)? {
        break;
      }
      let run_ended = class(self.character(source)?) != initial
        || (self.cursor.page, self.cursor.line)
          != (previous.page, previous.line);
      if !run_ended {
        continue;
      }
      if motion == Motion::Word {
        self.skip_blank(1, source)?;
      } else {
        self.cursor = previous;
      }
      break;
    }
    Ok(())
  }

  /// Steps over blank space; false at the end of the document.
  fn skip_blank(
    &mut self,
    direction: i32,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    for _ in 0..100_000 {
      if class(self.character(source)?) != 0 {
        return Ok(true);
      }
      if !self.step(direction, false, source)? {
        return Ok(false);
      }
    }
    Ok(true)
  }
}

pub fn range(a: Position, b: Position) -> (Position, Position) {
  if a <= b { (a, b) } else { (b, a) }
}

/// Whether `pos` lies in the ordered range, by whole lines when
/// `linewise`.
pub fn covers(
  (first, last): (Position, Position),
  linewise: bool,
  pos: Position,
) -> bool {
  if linewise {
    let line = (pos.page, pos.line);
    (first.page, first.line) <= line && line <= (last.page, last.line)
  } else {
    first <= pos && pos <= last
  }
}
