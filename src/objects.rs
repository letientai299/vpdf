use crate::Result;
use crate::motion::Motion;
use crate::reader::{Reader, Selection, SelectionKind, TextSource};
use crate::text::Position;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Object {
  Word,
  BigWord,
  Sentence,
  Paragraph,
  Parentheses,
  Brackets,
  Braces,
  Angles,
  DoubleQuote,
  SingleQuote,
}

impl Reader {
  pub fn select_object(
    &mut self,
    kind: Object,
    around: bool,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<()> {
    if source.page_text(self.cursor.page)?.glyph(self.cursor).is_none() {
      self.disarm();
      return Ok(());
    }
    let mut scan = Self::default();
    scan.cursor = self.cursor;
    let count = count
      .saturating_mul(self.operator.map_or(1, |o| o.count))
      .clamp(1, 10_000);
    let bounds = match kind {
      Object::Word | Object::BigWord => {
        scan.word_span(kind == Object::BigWord, count, source)?
      }
      Object::Sentence => scan.sentence_span(count, source)?,
      Object::Paragraph => scan.paragraph_span(count, source)?,
      Object::DoubleQuote | Object::SingleQuote => {
        scan.quote_span(kind, count, source)?
      }
      _ => scan.pair_span(kind, count, source)?,
    };
    let Some((mut first, mut last)) = bounds else {
      self.disarm();
      return Ok(());
    };
    let prose = matches!(
      kind,
      Object::Word | Object::BigWord | Object::Sentence | Object::Paragraph
    );
    if prose && around && kind != Object::Paragraph {
      (first, last) = scan.padded(first, last, source)?;
    } else if !prose && !around {
      scan.cursor = first;
      if !scan.advance(1, source)? || scan.cursor >= last {
        self.disarm();
        return Ok(());
      }
      first = scan.cursor;
      scan.cursor = last;
      scan.advance(-1, source)?;
      last = scan.cursor;
    }
    if let Some(op) = self.operator.take() {
      self.yank_range(
        first,
        last,
        kind == Object::Paragraph && around,
        op.register,
        source,
      )?;
    } else {
      self.selection = Some(Selection {
        anchor: first,
        kind: if kind == Object::Paragraph && around {
          SelectionKind::Line
        } else {
          SelectionKind::Character
        },
      });
      self.cursor = last;
      self.desired = None;
    }
    Ok(())
  }

  pub(crate) fn lexical_motion(
    &mut self,
    motion: Motion,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    match motion {
      Motion::NextSentence | Motion::PreviousSentence => {
        let direction = if motion == Motion::NextSentence { 1 } else { -1 };
        while self.advance(direction, source)? {
          if self.sentence_start(source)? {
            return Ok(true);
          }
        }
      }
      Motion::BigWord => {
        self.run_edge(1, true, source)?;
        if self.advance(1, source)? {
          self.nonblank(1, source)?;
          return Ok(!self.blank(source)?);
        }
      }
      Motion::BackBigWord => {
        if self.advance(-1, source)? {
          self.nonblank(-1, source)?;
          self.run_edge(-1, true, source)?;
        }
      }
      Motion::EndBigWord => {
        if self.advance(1, source)? {
          self.nonblank(1, source)?;
          self.run_edge(1, true, source)?;
        }
      }
      Motion::PreviousBigEnd => {
        self.run_edge(-1, true, source)?;
        if self.advance(-1, source)? {
          self.nonblank(-1, source)?;
        }
      }
      _ => {}
    }
    Ok(false)
  }

  fn blank(&self, source: &mut impl TextSource) -> Result<bool> {
    Ok(self.character(source)?.is_whitespace())
  }

  fn advance(
    &mut self,
    direction: i32,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    let origin = self.cursor;
    while self.step(direction, false, source)? {
      if source.page_text(self.cursor.page)?.glyph(self.cursor).is_some() {
        return Ok(true);
      }
    }
    self.cursor = origin;
    Ok(false)
  }

  fn nonblank(
    &mut self,
    direction: i32,
    source: &mut impl TextSource,
  ) -> Result<()> {
    while self.blank(source)? && self.advance(direction, source)? {}
    Ok(())
  }

  fn padded(
    &mut self,
    mut first: Position,
    mut last: Position,
    source: &mut impl TextSource,
  ) -> Result<(Position, Position)> {
    let original = last;
    self.cursor = last;
    while self.advance(1, source)? && self.blank(source)? {
      last = self.cursor;
    }
    if last != original {
      return Ok((first, last));
    }
    self.cursor = first;
    while self.advance(-1, source)? && self.blank(source)? {
      first = self.cursor;
    }
    Ok((first, last))
  }

  fn run_edge(
    &mut self,
    direction: i32,
    big: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let text = source.page_text(self.cursor.page)?;
    let Some(line) = text.lines.get(self.cursor.line) else {
      return Ok(());
    };
    let classify = |ch: char| {
      if big { u8::from(!ch.is_whitespace()) } else { crate::text::class(ch) }
    };
    let Some((first, last)) = line.run_by(self.cursor.glyph, classify) else {
      return Ok(());
    };
    self.cursor.glyph = if direction > 0 { last } else { first };
    Ok(())
  }

  fn word_span(
    &mut self,
    big: bool,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<Option<(Position, Position)>> {
    if source.page_text(self.cursor.page)?.glyph(self.cursor).is_none() {
      return Ok(None);
    }
    self.run_edge(-1, big, source)?;
    let first = self.cursor;
    for unit in 0..count {
      self.run_edge(1, big, source)?;
      if unit + 1 < count {
        let old = self.cursor;
        if !self.advance(1, source)? {
          break;
        }
        self.nonblank(1, source)?;
        if self.blank(source)? {
          self.cursor = old;
          break;
        }
      }
    }
    Ok(Some((first, self.cursor)))
  }

  fn paragraph_start(&self, source: &mut impl TextSource) -> Result<bool> {
    Ok(
      source
        .page_text(self.cursor.page)?
        .lines
        .get(self.cursor.line)
        .is_some_and(|line| {
          line.paragraph
            && line.glyphs[..self.cursor.glyph.min(line.glyphs.len())]
              .iter()
              .all(|g| g.ch.is_whitespace())
        }),
    )
  }

  fn sentence_start(&mut self, source: &mut impl TextSource) -> Result<bool> {
    if self.blank(source)? {
      return Ok(false);
    }
    if self.paragraph_start(source)? {
      return Ok(true);
    }
    let origin = self.cursor;
    let result = self.after_sentence(source);
    self.cursor = origin;
    result
  }

  fn after_sentence(&mut self, source: &mut impl TextSource) -> Result<bool> {
    let origin = self.cursor;
    if !self.advance(-1, source)? {
      return Ok(true);
    }
    if !self.blank(source)?
      && (origin.page, origin.line) == (self.cursor.page, self.cursor.line)
    {
      return Ok(false);
    }
    self.nonblank(-1, source)?;
    while matches!(
      self.character(source)?,
      ')' | ']' | '}' | '"' | '\'' | '”' | '’'
    ) {
      if !self.advance(-1, source)? {
        return Ok(false);
      }
    }
    match self.character(source)? {
      '!' | '?' => Ok(true),
      '.' => {
        let mut token = String::new();
        loop {
          let ch = self.character(source)?;
          if !ch.is_alphabetic() && ch != '.' {
            break;
          }
          token.push(ch.to_ascii_lowercase());
          if !self.advance(-1, source)? {
            break;
          }
        }
        let token: String = token.chars().rev().collect();
        let abbreviated = matches!(
          token.as_str(),
          "fig."
            | "figs."
            | "eq."
            | "eqs."
            | "al."
            | "e.g."
            | "i.e."
            | "etc."
            | "dr."
            | "prof."
            | "vs."
            | "ref."
            | "refs."
            | "no."
            | "vol."
        ) || (token.chars().count() == 2
          && token.ends_with('.'));
        Ok(!abbreviated)
      }
      _ => Ok(false),
    }
  }

  fn sentence_span(
    &mut self,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<Option<(Position, Position)>> {
    self.nonblank(1, source)?;
    while !self.sentence_start(source)? && self.advance(-1, source)? {}
    let first = self.cursor;
    let mut boundary = false;
    for _ in 0..count {
      boundary = self.lexical_motion(Motion::NextSentence, source)?;
    }
    if boundary {
      self.advance(-1, source)?;
    }
    self.nonblank(-1, source)?;
    Ok(Some((first, self.cursor)))
  }

  fn paragraph_span(
    &mut self,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<Option<(Position, Position)>> {
    self.cursor.glyph = 0;
    if !self.paragraph_start(source)? {
      self.paragraph(false, source)?;
    }
    let first = self.cursor;
    let mut boundary = false;
    for _ in 0..count {
      boundary = self.paragraph(true, source)?;
    }
    if boundary {
      self.advance(-1, source)?;
    }
    self.nonblank(-1, source)?;
    Ok(Some((first, self.cursor)))
  }

  fn pair_span(
    &mut self,
    kind: Object,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<Option<(Position, Position)>> {
    let (open, close) = match kind {
      Object::Parentheses => ('(', ')'),
      Object::Brackets => ('[', ']'),
      Object::Braces => ('{', '}'),
      Object::Angles => ('<', '>'),
      _ => unreachable!(),
    };
    let origin = self.cursor;
    let mut first = origin;
    for level in 0..count {
      if level > 0 && !self.advance(-1, source)? {
        return Ok(None);
      }
      if level == 0
        && self.character(source)? == close
        && !self.advance(-1, source)?
      {
        return Ok(None);
      }
      let mut depth = 0usize;
      loop {
        let ch = self.character(source)?;
        if ch == open && depth == 0 {
          break;
        }
        if ch == open {
          depth -= 1;
        }
        if ch == close {
          depth += 1;
        }
        if !self.advance(-1, source)? {
          return Ok(None);
        }
      }
      first = self.cursor;
    }
    let mut depth = 1usize;
    while self.advance(1, source)? {
      let ch = self.character(source)?;
      if ch == open {
        depth += 1;
      }
      if ch == close {
        depth -= 1;
        if depth == 0 {
          return Ok((self.cursor >= origin).then_some((first, self.cursor)));
        }
      }
    }
    Ok(None)
  }

  fn quote_span(
    &mut self,
    kind: Object,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<Option<(Position, Position)>> {
    let origin = self.cursor;
    self.quote_anchor(kind, source)?;
    let mut first = None;
    let mut last = origin;
    let mut remaining = count;
    loop {
      let until = first.is_none().then_some(origin);
      let Some((start, end)) = self.quote_pair(kind, until, None, source)?
      else {
        return Ok(first.map(|first| (first, last)));
      };
      if first.is_some() || (start <= origin && origin <= end) {
        first.get_or_insert(start);
        last = end;
        remaining -= 1;
      }
      if remaining == 0 || !self.advance(1, source)? {
        return Ok(first.map(|first| (first, last)));
      }
    }
  }

  fn quote_pair(
    &mut self,
    kind: Object,
    until: Option<Position>,
    block: Option<Position>,
    source: &mut impl TextSource,
  ) -> Result<Option<(Position, Position)>> {
    let mut opening = None;
    loop {
      if opening.is_none() && until.is_some_and(|end| self.cursor > end) {
        return Ok(None);
      }
      if block.is_some_and(|start| self.cursor != start)
        && self.cursor.glyph == 0
        && self.paragraph_start(source)?
      {
        return Ok(None);
      }
      let ch = self.character(source)?;
      let delimiter = quote_character(kind, ch) && self.quote_mark(source)?;
      match (delimiter, opening) {
        (true, Some((start, closing))) if ch == closing => {
          return Ok(Some((start, self.cursor)));
        }
        (true, None)
          if !matches!(ch, '”' | '’') && self.quote_opener(ch, source)? =>
        {
          let closing = match ch {
            '“' => '”',
            '‘' => '’',
            _ => ch,
          };
          opening = Some((self.cursor, closing));
        }
        _ => {}
      }
      if !self.advance(1, source)? {
        return Ok(None);
      }
    }
  }

  fn local_quote(
    &mut self,
    kind: Object,
    anchor: Position,
    origin: Position,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    self.cursor = anchor;
    while let Some((start, end)) =
      self.quote_pair(kind, Some(origin), Some(anchor), source)?
    {
      if start <= origin && origin <= end {
        let after = self.quote_next(source)?.unwrap_or(' ');
        return Ok(!after.is_alphanumeric());
      }
      if !self.advance(1, source)? {
        break;
      }
    }
    Ok(false)
  }

  fn quote_anchor(
    &mut self,
    kind: Object,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let mut limit = self.cursor;
    self.cursor.glyph = 0;
    if !self.paragraph_start(source)? {
      self.paragraph(false, source)?;
    }
    loop {
      let anchor = self.cursor;
      if self.local_quote(kind, anchor, limit, source)? {
        self.cursor = anchor;
        return Ok(());
      }
      self.cursor = anchor;
      let found = self.first_quote(kind, limit, source)?;
      if found && !self.quote_continuation(source)? {
        self.cursor = anchor;
        return Ok(());
      }
      self.cursor = anchor;
      self.paragraph(false, source)?;
      if self.cursor == anchor {
        return Ok(());
      }
      limit = anchor;
    }
  }

  fn first_quote(
    &mut self,
    kind: Object,
    limit: Position,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    while self.cursor <= limit {
      if quote_character(kind, self.character(source)?)
        && self.quote_mark(source)?
      {
        return Ok(true);
      }
      if !self.advance(1, source)? {
        break;
      }
    }
    Ok(false)
  }

  fn quote_continuation(
    &mut self,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    let ch = self.character(source)?;
    if matches!(ch, '”' | '’') {
      return Ok(true);
    }
    if matches!(ch, '“' | '‘') {
      return Ok(false);
    }
    let origin = self.cursor;
    let before = if origin.glyph == 0 {
      ' '
    } else {
      self.advance(-1, source)?;
      self.character(source)?
    };
    self.cursor = origin;
    let after =
      if self.advance(1, source)? { self.character(source)? } else { ' ' };
    self.cursor = origin;
    Ok(
      origin.glyph == 0
        || (before.is_whitespace() && after.is_whitespace())
        || matches!(
          after,
          ',' | '.' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '—'
        )
        || (!before.is_whitespace()
          && !quote_edge(before)
          && after.is_whitespace()),
    )
  }

  fn quote_opener(
    &mut self,
    ch: char,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    if ch != '\'' || self.cursor.glyph == 0 {
      return Ok(true);
    }
    let origin = self.cursor;
    let before =
      if self.advance(-1, source)? { self.character(source)? } else { ' ' };
    self.cursor = origin;
    Ok(!before.is_alphanumeric())
  }

  fn quote_next(&self, source: &mut impl TextSource) -> Result<Option<char>> {
    let next = Position { glyph: self.cursor.glyph + 1, ..self.cursor };
    Ok(source.page_text(next.page)?.glyph(next).map(|g| g.ch))
  }

  fn quote_mark(&self, source: &mut impl TextSource) -> Result<bool> {
    let text = source.page_text(self.cursor.page)?;
    let Some(line) = text.lines.get(self.cursor.line) else {
      return Ok(false);
    };
    let index = self.cursor.glyph;
    let Some(glyph) = line.glyphs.get(index) else {
      return Ok(false);
    };
    let before = index.checked_sub(1).map(|i| line.glyphs[i].ch);
    let after = line.glyphs.get(index + 1).map(|g| g.ch);
    let slashes =
      line.glyphs[..index].iter().rev().take_while(|g| g.ch == '\\').count();
    Ok(
      slashes % 2 == 0
        && !(matches!(glyph.ch, '\'' | '’')
          && before.is_some_and(char::is_alphanumeric)
          && after.is_some_and(char::is_alphanumeric)),
    )
  }
}

fn quote_edge(ch: char) -> bool {
  matches!(ch, '(' | '[' | '{' | '<' | ':' | ',' | ';' | '=' | '—' | '-')
}

fn quote_character(kind: Object, ch: char) -> bool {
  match kind {
    Object::DoubleQuote => matches!(ch, '"' | '“' | '”'),
    Object::SingleQuote => matches!(ch, '\'' | '‘' | '’'),
    _ => false,
  }
}
