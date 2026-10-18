use crate::Result;
use crate::text::{PageId, Position, Text};
use fancy_regex::{Regex, RegexBuilder};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug)]
pub struct Hit {
  pub first: Position,
  pub last: Position,
  pub excerpt: String,
}

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

impl Run {
  fn anchor(self, byte: u32) -> (u32, u32) {
    let step = if self.stepped { byte - self.byte } else { 0 };
    (self.line, self.glyph + step)
  }
}

/// Whether a hyphen joins lines.
pub(crate) fn hyphenated(line: &str) -> bool {
  line.ends_with('-')
}

impl Index {
  pub fn new(page: PageId, text: &Text) -> Self {
    let lines = text.lines.iter().map(|line| line.text().into()).collect();
    let mut index = Self { page, text: String::new(), runs: Vec::new(), lines };
    let rows = text.lines.iter().map(|row| row.glyphs.iter().map(|g| g.ch));
    index.append(rows.enumerate(), 0, true, usize::MAX);
    index.text.shrink_to_fit();
    index.runs.shrink_to_fit();
    index
  }

  /// Joins `rows` as searched, from glyph `first`.
  fn append<'a>(
    &mut self,
    rows: impl Iterator<Item = (usize, impl Iterator<Item = char> + 'a)>,
    first: usize,
    fold: bool,
    limit: usize,
  ) {
    for (index, (line, row)) in rows.enumerate() {
      let skip = if index == 0 { first } else { 0 };
      for (glyph, ch) in row.enumerate().skip(skip) {
        if self.text.len() >= limit {
          return;
        }
        self.push_glyph(ch, (line as u32, glyph as u32), fold);
      }
      if hyphenated(&self.text) {
        self.text.pop();
        let len = self.text.len();
        if self.runs.last().is_some_and(|run| run.byte as usize == len) {
          self.runs.pop();
        }
      } else if let Some(anchor) = self.last_anchor() {
        self.push(' ', anchor);
      }
      if self.text.len() >= limit {
        break;
      }
    }
  }

  fn push_glyph(&mut self, ch: char, anchor: (u32, u32), fold: bool) {
    if !fold {
      return self.push(ch, anchor);
    }
    for ch in std::iter::once(ch).nfkd().case_fold() {
      self.push(ch, anchor);
    }
  }

  fn push(&mut self, ch: char, anchor: (u32, u32)) {
    let ch = if ch.is_whitespace() { ' ' } else { ch };
    if ch == ' ' && self.text.ends_with(' ') {
      return;
    }
    let byte = self.text.len() as u32;
    self.text.push(ch);
    let single = ch.len_utf8() == 1;
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

  pub fn matches(&self, pattern: &Regex, limit: usize) -> Result<Vec<Hit>> {
    let mut hits = Vec::new();
    if limit == 0 {
      return Ok(hits);
    }
    for found in pattern.find_iter(&self.text) {
      let found = found?;
      if found.start() == found.end() {
        continue;
      }
      hits.push(Hit {
        first: self.position(found.start()),
        last: self.position(found.end() - 1),
        excerpt: self.excerpt(self.position(found.start())),
      });
      if hits.len() >= limit {
        break;
      }
    }
    Ok(hits)
  }

  /// 80 printed characters from `from`.
  fn excerpt(&self, from: Position) -> String {
    let mut printed = Self {
      page: self.page,
      text: String::new(),
      runs: Vec::new(),
      lines: Vec::new(),
    };
    let rows = self.lines.iter().map(|line| line.chars()).enumerate();
    // Four bytes cover any character.
    printed.append(rows.skip(from.line), from.glyph, false, 80 * 4);
    printed.text.chars().take(80).collect()
  }

  pub fn weight(&self) -> usize {
    let lines = self.lines.iter().map(|line| line.len()).sum::<usize>();
    self.text.capacity()
      + self.runs.capacity() * size_of::<Run>()
      + lines
      + self.lines.capacity() * size_of::<Box<str>>()
  }
}

/// Whether a hit covers `pos`; `hits` are sorted and apart.
pub fn contains(hits: &[Hit], pos: Position) -> bool {
  let index = hits.partition_point(|hit| hit.first <= pos);
  index > 0 && pos <= hits[index - 1].last
}

pub fn compile(query: &str, whole_word: bool) -> Result<Regex> {
  let pattern = if let Some(raw) = query.strip_prefix("\\v") {
    raw.nfkd().collect::<String>()
  } else {
    let normalized: String = query.nfkd().case_fold().collect();
    let words = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    fancy_regex::escape(&words).to_string()
  };
  let pattern =
    if whole_word { format!(r"\b(?:{pattern})\b") } else { pattern };
  Ok(
    RegexBuilder::new(&format!("(?i){pattern}"))
      .backtrack_limit(100_000)
      .build()?,
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Bounds, Glyph, Line};

  fn sample(lines: &[&str]) -> Text {
    Text {
      lines: lines
        .iter()
        .map(|s| Line {
          paragraph: true,
          glyphs: s
            .chars()
            .map(|ch| Glyph { ch, bounds: Bounds::default() })
            .collect(),
        })
        .collect(),
      truncated: false,
    }
  }

  #[test]
  fn normalizes_and_maps_unicode() -> Result<()> {
    let text = sample(&["Straße ﬁsh inter-", "national café"]);
    let index = Index::new(PageId(0), &text);
    for query in ["STRASSE", "fish", "international", "café"] {
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
      ("INTERNATIONAL", "International Café Next "),
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
    let lines = ["Straße ﬁsh inter-", "national  café", "日本語 x\ty", "-"];
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
