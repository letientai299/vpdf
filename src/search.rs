use crate::Result;
use crate::text::{PageId, Position, Text};
use fancy_regex::{Regex, RegexBuilder};
use serde::Serialize;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
  pub first: Position,
  pub last: Position,
  pub excerpt: String,
}

pub struct Index {
  text: String,
  anchors: Vec<Position>,
}

impl Index {
  pub fn new(page: PageId, text: &Text) -> Self {
    let mut index = Self { text: String::new(), anchors: Vec::new() };
    for (line, row) in text.lines.iter().enumerate() {
      for (glyph, ch) in row.glyphs.iter().enumerate() {
        let pos = Position { page, line, glyph };
        for ch in ch.ch.to_string().nfkd().case_fold() {
          index.push(ch, pos);
        }
      }
      if index.text.ends_with('-') {
        index.text.pop();
        index.anchors.pop();
      } else if let Some(pos) = index.anchors.last().copied() {
        index.push(' ', pos);
      }
    }
    index
  }

  fn push(&mut self, ch: char, pos: Position) {
    let ch = if ch.is_whitespace() { ' ' } else { ch };
    if ch == ' ' && self.text.ends_with(' ') {
      return;
    }
    self.text.push(ch);
    self.anchors.extend(std::iter::repeat_n(pos, ch.len_utf8()));
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
      let first = self.anchors[found.start()];
      let last = self.anchors[found.end() - 1];
      hits.push(Hit {
        first,
        last,
        excerpt: self.text[found.start()..].chars().take(80).collect(),
      });
      if hits.len() >= limit {
        break;
      }
    }
    Ok(hits)
  }

  pub fn weight(&self) -> usize {
    self.text.capacity() + self.anchors.capacity() * size_of::<Position>()
  }
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
