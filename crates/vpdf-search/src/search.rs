use crate::Result;
use crate::text::{PageId, Position, Text};
use fancy_regex::Regex;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

mod codec;
mod join;
#[cfg(test)]
mod multilingual;
mod normalize;
mod words;
use join::Join;

#[derive(Clone, Copy)]
enum Form {
  Printed,
  Folded,
  Exact,
  RegexFolded,
  RegexExact,
}

impl Form {
  fn folded(self) -> bool {
    matches!(self, Self::Folded | Self::RegexFolded)
  }

  fn literal(self) -> bool {
    matches!(self, Self::Folded | Self::Exact)
  }
}

pub struct Pattern {
  regex: Regex,
  exact: bool,
  raw: bool,
  whole: bool,
  source: String,
  loose: OnceLock<Regex>,
}

fn normalized(text: &str, form: Form) -> String {
  let mut out = String::new();
  normalize::scan(text.chars().map(|ch| (ch, (), ())), form, |ch, (), ()| {
    out.push(ch);
  });
  out
}

impl Pattern {
  fn form(&self) -> Form {
    match (self.raw, self.exact) {
      (false, false) => Form::Folded,
      (false, true) => Form::Exact,
      (true, false) => Form::RegexFolded,
      (true, true) => Form::RegexExact,
    }
  }

  pub fn ranges(&self, text: &str) -> Vec<(usize, usize)> {
    let mut haystack = String::new();
    let mut offsets = Vec::new();
    let input =
      text.char_indices().map(|(at, ch)| (ch, at, at + ch.len_utf8()));
    normalize::scan(input, self.form(), |ch, start, end| {
      let ch = if ch.is_whitespace() { ' ' } else { ch };
      if ch == ' ' && haystack.ends_with(' ') {
        return;
      }
      haystack.push(ch);
      offsets.extend(std::iter::repeat_n((start, end), ch.len_utf8()));
    });
    let boundaries = self.whole.then(|| words::boundaries(&haystack)).flatten();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let _ = self.visit(&haystack, boundaries.as_deref(), |start, end| {
      let range = (offsets[start].0, offsets[end - 1].1);
      if let Some(last) = ranges.last_mut()
        && range.0 < last.1
      {
        last.1 = last.1.max(range.1);
        return true;
      }
      ranges.push(range);
      true
    });
    ranges
  }
}

#[derive(Clone, Debug)]
pub struct Hit {
  pub first: Position,
  pub last: Position,
  pub excerpt: String,
}

#[derive(Default)]
pub struct Index {
  page: PageId,
  text: String,
  /// Where each byte of `text` came from, run-length encoded: a byte
  /// per glyph runs on, so a Latin-script index takes about twice its
  /// text rather than nine times, and a long manual's indexes stay
  /// cached. Multi-byte text keeps a run per character.
  runs: Vec<Run>,
  /// Lines as printed, for excerpts.
  lines: Vec<Box<str>>,
  joins: Vec<Join>,
  covers: Vec<Cover>,
  words: OnceLock<Option<Box<[u32]>>>,
}

/// Bytes of `text` from `byte` on, up to the next run, with their line
/// and glyph; a page stays far below `u32::MAX` of each.
#[derive(Clone, Copy)]
struct Run {
  byte: u32,
  line: u32,
  glyph: u32,
  /// Each byte is the next glyph; else every byte is `glyph`, as in a
  /// multi-byte character.
  stepped: bool,
}

#[derive(Clone, Copy)]
struct Cover {
  start: u32,
  end: u32,
  line: u32,
  glyph: u32,
}

impl Run {
  fn anchor(self, byte: u32) -> (u32, u32) {
    let step = if self.stepped { byte - self.byte } else { 0 };
    (self.line, self.glyph + step)
  }
}

pub fn word_at(line: &crate::text::Line, glyph: usize) -> Option<String> {
  words::word_at(line, glyph)
}

/// Whether a hyphen joins lines.
pub fn hyphenated(line: &str) -> bool {
  line.ends_with('-')
}

impl Index {
  pub fn new(page: PageId, text: &Text) -> Self {
    let lines: Vec<Box<str>> =
      text.lines.iter().map(|line| line.text().into()).collect();
    let mut index = Self { page, ..Self::default() };
    let joins = join::decisions(text);
    let rows = text.lines.iter().enumerate().map(|(at, row)| {
      (at, row.glyphs.iter().map(|g| g.ch), lines[at].is_ascii())
    });
    index.append(rows, 0, Form::Folded, usize::MAX, &joins);
    index.lines = lines;
    index.joins = joins;
    index.text.shrink_to_fit();
    index.runs.shrink_to_fit();
    index.covers.shrink_to_fit();
    index
  }

  /// Joins `rows` as searched, from glyph `first`.
  fn append<'a>(
    &mut self,
    rows: impl Iterator<Item = (usize, impl Iterator<Item = char> + 'a, bool)>,
    first: usize,
    form: Form,
    limit: usize,
    joins: &[Join],
  ) {
    for (index, (line, row, ascii)) in rows.enumerate() {
      let skip = if index == 0 { first } else { 0 };
      let row =
        row.enumerate().skip(skip).take(limit.saturating_sub(self.text.len()));
      self.append_row(row, line, form, ascii, limit);
      self.join_line(joins.get(line).copied().unwrap_or(Join::Space), form);
      if self.text.len() >= limit {
        break;
      }
    }
  }

  #[inline]
  fn append_row(
    &mut self,
    row: impl Iterator<Item = (usize, char)>,
    line: usize,
    form: Form,
    ascii: bool,
    limit: usize,
  ) {
    if matches!(form, Form::Printed) {
      for (_, ch) in row {
        if self.text.len() >= limit {
          return;
        }
        self.push_char(ch);
      }
      return;
    }
    if ascii {
      for (glyph, ch) in row {
        let ch = if form.folded() { ch.to_ascii_lowercase() } else { ch };
        self.push(ch, (line as u32, glyph as u32));
      }
      return;
    }
    let input = row.map(|(glyph, ch)| {
      let at = (line as u32, glyph as u32);
      (ch, at, at)
    });
    normalize::scan(input, form, |ch, first, last| {
      self.push_source(ch, first, last);
    });
  }

  fn push_source(&mut self, ch: char, first: (u32, u32), last: (u32, u32)) {
    let start = self.text.len() as u32;
    self.push(ch, first);
    let end = self.text.len() as u32;
    if first == last || start == end {
      return;
    }
    if let Some(cover) = self.covers.last_mut()
      && cover.end == start
      && (cover.line, cover.glyph) == last
    {
      cover.end = end;
      return;
    }
    self.covers.push(Cover { start, end, line: last.0, glyph: last.1 });
  }

  fn join_line(&mut self, join: Join, form: Form) {
    if join == Join::Hyphen && self.text.ends_with('-') {
      self.text.pop();
      let len = self.text.len();
      if self.runs.last().is_some_and(|run| run.byte as usize == len) {
        self.runs.pop();
      }
    } else if join == Join::Space {
      if matches!(form, Form::Printed) && !self.text.is_empty() {
        self.push_char(' ');
      } else if !matches!(form, Form::Printed)
        && let Some(anchor) = self.last_anchor()
      {
        self.push(' ', anchor);
      }
    }
  }

  #[inline]
  fn push_char(&mut self, ch: char) -> Option<u32> {
    let ch = if ch.is_whitespace() { ' ' } else { ch };
    if ch == ' ' && self.text.ends_with(' ') {
      return None;
    }
    let byte = self.text.len() as u32;
    self.text.push(ch);
    Some(byte)
  }

  #[inline]
  fn push(&mut self, ch: char, anchor: (u32, u32)) {
    let Some(byte) = self.push_char(ch) else { return };
    let single = self.text.len() == byte as usize + 1;
    if let Some(run) = self.runs.last()
      && run.stepped
      && single
      && run.anchor(byte) == anchor
    {
      return;
    }
    let (line, glyph) = anchor;
    self.runs.push(Run { byte, line, glyph, stepped: single });
  }

  fn last_anchor(&self) -> Option<(u32, u32)> {
    let last = self.text.len().checked_sub(1)?;
    Some(self.anchor(last))
  }

  fn anchor(&self, byte: usize) -> (u32, u32) {
    let byte = byte as u32;
    let at = self.runs.partition_point(|run| run.byte <= byte) - 1;
    self.runs[at].anchor(byte)
  }

  fn position(&self, byte: usize) -> Position {
    let (line, glyph) = self.anchor(byte);
    Position { page: self.page, line: line as usize, glyph: glyph as usize }
  }

  #[inline]
  pub fn matches(&self, pattern: &Pattern, limit: usize) -> Result<Vec<Hit>> {
    let mut hits = Vec::new();
    if limit == 0 {
      return Ok(hits);
    }
    let exact = if pattern.exact || pattern.raw {
      let mut exact = Self { page: self.page, ..Self::default() };
      let rows = self
        .lines
        .iter()
        .enumerate()
        .map(|(at, line)| (at, line.chars(), line.is_ascii()));
      exact.append(rows, 0, pattern.form(), usize::MAX, &self.joins);
      Some(exact)
    } else {
      None
    };
    let index = exact.as_ref().unwrap_or(self);
    let boundaries = if pattern.whole {
      index.words.get_or_init(|| words::boundaries(&index.text)).as_deref()
    } else {
      None
    };
    pattern.visit(&index.text, boundaries, |start, end| {
      let first = index.position(start);
      let mut last = index.position(end - 1);
      let cover =
        index.covers.partition_point(|cover| cover.start < end as u32);
      if cover > 0 {
        let cover = index.covers[cover - 1];
        if cover.end as usize > start {
          last = last.max(Position {
            page: self.page,
            line: cover.line as usize,
            glyph: cover.glyph as usize,
          });
        }
      }
      hits.push(Hit { first, last, excerpt: self.excerpt(first) });
      hits.len() < limit
    })?;
    Ok(hits)
  }

  /// 80 printed characters from `from`.
  fn excerpt(&self, from: Position) -> String {
    let mut printed = Self::default();
    let rows = self
      .lines
      .iter()
      .enumerate()
      .map(|(at, line)| (at, line.chars(), line.is_ascii()));
    // Four bytes cover any character.
    printed.append(
      rows.skip(from.line),
      from.glyph,
      Form::Printed,
      80 * 4,
      &self.joins,
    );
    printed.text.chars().take(80).collect()
  }

  pub fn definitions(
    &self,
    reference: &crate::reference::Reference,
    limit: usize,
  ) -> Vec<Hit> {
    self
      .lines
      .iter()
      .enumerate()
      .filter_map(|(line, text)| {
        let (start, end) = reference.destination(text)?;
        let first = Position {
          page: self.page,
          line,
          glyph: text[..start].chars().count(),
        };
        let last = Position {
          glyph: text[..end].chars().count().saturating_sub(1),
          ..first
        };
        Some(Hit { first, last, excerpt: self.excerpt(first) })
      })
      .take(limit)
      .collect()
  }

  pub fn weight(&self) -> usize {
    let lines = self.lines.iter().map(|line| line.len()).sum::<usize>();
    self.text.capacity()
      + self.runs.capacity() * size_of::<Run>()
      + self.covers.capacity() * size_of::<Cover>()
      + self.joins.capacity() * size_of::<Join>()
      + self
        .words
        .get()
        .and_then(|words| words.as_ref())
        .map_or(0, |words| words.len() * size_of::<u32>())
      + lines
      + self.lines.capacity() * size_of::<Box<str>>()
  }
}

/// Whether a hit covers `pos`; `hits` are sorted and apart.
pub fn contains(hits: &[Hit], pos: Position) -> bool {
  let index = hits.partition_point(|hit| hit.first <= pos);
  index > 0 && pos <= hits[index - 1].last
}

/// Selects a counted match among sorted keys.
pub fn next<T, K: Ord>(
  items: &[T],
  from: &K,
  backward: bool,
  count: usize,
  key: impl Fn(&T) -> K,
) -> Option<usize> {
  let total = items.len();
  if total == 0 {
    return None;
  }
  let offset = count.saturating_sub(1) % total;
  let index = if backward {
    let end = items.partition_point(|item| key(item) < *from);
    (end + total - 1 - offset) % total
  } else {
    let start = items.partition_point(|item| key(item) <= *from);
    (start + offset) % total
  };
  Some(index)
}

pub fn compile(query: &str, whole_word: bool) -> Result<Pattern> {
  let exact = query.chars().any(char::is_uppercase);
  let raw = query.starts_with("\\v");
  let pattern = if raw {
    query[2..].nfkd().collect::<String>()
  } else {
    let form = if exact { Form::Exact } else { Form::Folded };
    let normalized = normalized(query, form);
    let words = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    fancy_regex::escape(&words).to_string()
  };
  let flags = if exact { "" } else { "(?i)" };
  let source = format!("{flags}(?:{pattern})");
  let bounded =
    if whole_word { format!(r"\b(?:{source})\b") } else { source.clone() };
  let regex = words::regex(&bounded)?;
  Ok(Pattern {
    regex,
    exact,
    raw,
    whole: whole_word,
    source,
    loose: OnceLock::new(),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Bounds, Glyph, Line};
  use unicode_casefold::UnicodeCaseFold;

  pub(super) fn sample(lines: &[&str]) -> Text {
    Text {
      lines: lines
        .iter()
        .map(|s| Line {
          paragraph: true,
          glyphs: s
            .chars()
            .map(|ch| Glyph { quad: None, ch, bounds: Bounds::default() })
            .collect(),
        })
        .collect(),
      truncated: false,
    }
  }

  #[test]
  fn counted_matches_wrap() {
    let items = [2, 5, 9];
    let cases = [0, 2, 3, 5, 9, 10].into_iter().flat_map(|from| {
      [false, true].into_iter().flat_map(move |backward| {
        [0, 1, 2, 3, 4, 10, usize::MAX]
          .map(move |count| (from, backward, count))
      })
    });
    for (from, backward, count) in cases {
      let mut expected = from;
      for _ in 0..=(count.saturating_sub(1) % items.len()) {
        expected = if backward {
          *items.iter().rfind(|&&item| item < expected).unwrap_or(&9)
        } else {
          *items.iter().find(|&&item| item > expected).unwrap_or(&2)
        };
      }
      let index = next(&items, &from, backward, count, |&item| item).unwrap();
      assert_eq!(items[index], expected, "{from} {backward} {count}");
    }
    assert_eq!(next::<usize, usize>(&[], &0, false, 1, |&item| item), None);
  }

  #[test]
  fn normalizes_and_maps_unicode() -> Result<()> {
    let text = sample(&["Straße ﬁsh inter-", "national café"]);
    let index = Index::new(PageId(0), &text);
    for query in ["strasse", "fish", "international", "café"] {
      assert_eq!(index.matches(&compile(query, false)?, 1000)?.len(), 1);
    }
    let hits = index.matches(&compile("international", false)?, 1000)?;
    assert_eq!(hits[0].first.line, 0);
    assert_eq!(hits[0].last.line, 1);
    Ok(())
  }

  /// U28: excerpts keep the printed case.
  #[test]
  fn excerpts_keep_printed_text() -> Result<()> {
    let text = sample(&["Straße  ﬁsh Inter-", "national Café", "Next"]);
    let index = Index::new(PageId(0), &text);
    for (query, excerpt) in [
      ("strasse", "Straße ﬁsh International Café Next "),
      ("International", "International Café Next "),
      ("café", "Café Next "),
    ] {
      let hits = index.matches(&compile(query, false)?, 1000)?;
      assert_eq!(hits[0].excerpt, excerpt, "{query}");
    }
    let long = sample(&[&"Word ".repeat(30)]);
    let hits =
      Index::new(PageId(0), &long).matches(&compile("word", false)?, 1)?;
    assert_eq!(hits[0].excerpt.chars().count(), 80);
    Ok(())
  }

  /// Runs answer what a per-byte table would: multi-byte and expanded
  /// characters, joined hyphens, and collapsed spaces.
  #[test]
  fn runs_match_byte_anchors() {
    let lines = ["Straße ﬁsh inter-", "national  cafe", "日本語 x\ty", "-"];
    let text = sample(&lines);
    let index = Index::new(PageId(0), &text);
    let mut expected: Vec<(u32, u32)> = Vec::new();
    let mut flat = String::new();
    for (line, row) in text.lines.iter().enumerate() {
      let chars = row.glyphs.iter().enumerate().flat_map(|(glyph, ch)| {
        let anchor = (line as u32, glyph as u32);
        std::iter::once(ch.ch).nfkd().case_fold().map(move |ch| (ch, anchor))
      });
      for (ch, anchor) in chars {
        let ch = if ch.is_whitespace() { ' ' } else { ch };
        if ch == ' ' && flat.ends_with(' ') {
          continue;
        }
        flat.push(ch);
        expected.extend(std::iter::repeat_n(anchor, ch.len_utf8()));
      }
      if hyphenated(&flat) {
        flat.pop();
        expected.pop();
      } else if let Some(&anchor) = expected.last() {
        flat.push(' ');
        expected.push(anchor);
      }
    }
    assert_eq!(index.text, flat);
    let actual: Vec<_> =
      (0..flat.len()).map(|byte| index.anchor(byte)).collect();
    assert_eq!(actual, expected);
  }

  #[test]
  fn ascii_matches_unicode_path() {
    for form in [Form::Exact, Form::Folded] {
      let input = (0..=127).map(char::from);
      let expected: String = if form.folded() {
        input.clone().nfkd().case_fold().collect()
      } else {
        input.clone().nfkd().collect()
      };
      assert_eq!(normalized(&input.collect::<String>(), form), expected);
    }
  }

  #[test]
  fn excerpt_limits_preserve_joins() {
    for ch in ['a', 'é', '日'] {
      for count in [79, 80, 81, 319, 320, 321] {
        let prefix: String = std::iter::repeat_n(ch, count).collect();
        let line = format!("{prefix}-");
        let text = sample(&[&line, "b"]);
        let index = Index::new(PageId(0), &text);
        let actual = index.excerpt(Position::default());
        let expected: String = format!("{prefix}b ").chars().take(80).collect();
        assert_eq!(actual, expected, "{ch} {count}");
      }
    }
    let text = sample(&["  A\tB ", "", " C"]);
    let index = Index::new(PageId(0), &text);
    assert_eq!(index.excerpt(Position::default()), " A B C ");
  }

  #[test]
  fn contains_finds_covering_hits() -> Result<()> {
    let text = sample(&["cat dog cat", "bird"]);
    let index = Index::new(PageId(0), &text);
    let hits = index.matches(&compile(r"\vcat|bird", false)?, 1000)?;
    let at = |line, glyph| Position { page: PageId(0), line, glyph };
    let cases = [
      (at(0, 0), true),
      (at(0, 2), true),
      (at(0, 3), false),
      (at(0, 8), true),
      (at(0, 11), false),
      (at(1, 3), true),
      (at(2, 0), false),
    ];
    for (pos, expected) in cases {
      assert_eq!(contains(&hits, pos), expected, "{pos:?}");
    }
    Ok(())
  }

  #[test]
  fn regex_and_literal_differ() -> Result<()> {
    let text = sample(&["cat cats cat"]);
    let index = Index::new(PageId(0), &text);
    assert_eq!(index.matches(&compile("cat", true)?, 1000)?.len(), 2);
    assert_eq!(index.matches(&compile(r"\vcat(?=s)", false)?, 1000)?.len(), 1);
    assert!(compile(r"\v[", false).is_err());
    assert!(index.matches(&compile("", false)?, 1000)?.is_empty());
    Ok(())
  }
}
