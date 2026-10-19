//! Joins yanked PDF lines back into running text.

use crate::text::{Line, Text};
use std::ops::Range;

/// How a line attaches to the one before it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Join {
  /// A new text block, or the first line of a page.
  Paragraph,
  /// A deliberate break inside a block: code, lists, short lines.
  Line,
  /// The line wrapped at the block's right edge.
  Wrap,
}

/// Joins for the `wanted` lines of `text`.
pub fn joins(text: &Text, wanted: Range<usize>) -> Vec<Join> {
  let lines = &text.lines;
  let rights = block_rights(lines, &wanted);
  wanted
    .map(|index| {
      if index == 0 {
        Join::Paragraph
      } else if same_row(&lines[index - 1], &lines[index]) {
        Join::Wrap
      } else if lines[index].paragraph {
        Join::Paragraph
      } else if fits(&lines[index - 1], &lines[index], rights[index]) {
        Join::Line
      } else {
        Join::Wrap
      }
    })
    .collect()
}

/// `next` continues `line`'s row to its right, as OCR fragments do.
fn same_row(line: &Line, next: &Line) -> bool {
  let (Some(a), Some(b)) = (row_span(line), row_span(next)) else {
    return false;
  };
  let overlap = a.1.min(b.1) - a.0.max(b.0);
  overlap > 0.5 * (a.1 - a.0).min(b.1 - b.0)
    && left_edge(next) >= right_edge(line) - 1.0
}

/// Top and bottom of a line's ink.
fn row_span(line: &Line) -> Option<(f32, f32)> {
  let mut ink = line.glyphs.iter().filter(|g| !g.ch.is_whitespace());
  let first = ink.next()?;
  Some(ink.fold((first.bounds.y0, first.bounds.y1), |(top, bottom), g| {
    (top.min(g.bounds.y0), bottom.max(g.bounds.y1))
  }))
}

/// The line continuing the word `line` ends with: the next row down in
/// its column, past lines beside it such as margin line numbers, when
/// that word filled the row. Block breaks do not matter; some PDFs put
/// every line in a block of its own.
pub fn continuation(text: &Text, line: usize) -> Option<usize> {
  let lines = &text.lines;
  let (top, bottom) = row_span(&lines[line])?;
  let (left, right) = (left_edge(&lines[line]), right_edge(&lines[line]));
  let middle = f32::midpoint(top, bottom);
  let next = (line + 1..lines.len()).take(4).find(|&index| {
    let (next, other) = (&lines[index], row_span(&lines[index]));
    other.is_some_and(|(top, _)| top > middle)
      && left_edge(next) < right
      && right_edge(next) > left
  })?;
  let (next_top, _) = row_span(&lines[next])?;
  let spans: Vec<_> =
    lines.iter().map(|line| (left_edge(line), right_edge(line))).collect();
  // Without a shared margin, a short row looks full.
  let margin = column_margin(&spans, left, right)?.max(right);
  let adjacent = next_top - bottom < bottom - top;
  (adjacent && !fits(&lines[line], &lines[next], margin)).then_some(next)
}

/// Right edge each line's block wraps at: its widest line, or its
/// column's margin when that reaches further, as for code in prose.
/// Only blocks meeting `wanted` get one.
fn block_rights(lines: &[Line], wanted: &Range<usize>) -> Vec<f32> {
  let spans: Vec<_> =
    lines.iter().map(|line| (left_edge(line), right_edge(line))).collect();
  let mut rights = vec![0.0; lines.len()];
  let mut start = 0;
  for end in 1..=lines.len() {
    if end < lines.len() && !lines[end].paragraph {
      continue;
    }
    let block = &spans[start..end];
    // A block's first line never wraps, so one line needs no edge.
    if block.len() < 2 || end <= wanted.start || start >= wanted.end {
      start = end;
      continue;
    }
    let left = block.iter().map(|span| span.0).fold(f32::MAX, f32::min);
    let right = block.iter().map(|span| span.1).fold(f32::MIN, f32::max);
    let margin = column_margin(&spans, left, right).unwrap_or(right);
    rights[start..end].fill(right.max(margin));
    start = end;
  }
  rights
}

/// The margin of the lines overlapping `left..right`.
fn column_margin(spans: &[(f32, f32)], left: f32, right: f32) -> Option<f32> {
  let column = spans.iter().filter(|(l, r)| *l < right && *r > left);
  margin(column.map(|span| span.1))
}

/// The right edge at least three lines share, within 2 pt.
fn margin(rights: impl Iterator<Item = f32>) -> Option<f32> {
  let mut bins = std::collections::HashMap::new();
  for right in rights {
    *bins.entry((right / 2.0).round() as i32).or_insert(0) += 1;
  }
  let (bin, count) =
    bins.into_iter().max_by_key(|&(bin, count)| (count, bin))?;
  (count >= 3).then_some(bin as f32 * 2.0)
}

fn left_edge(line: &Line) -> f32 {
  let ink = line.glyphs.iter().filter(|g| !g.ch.is_whitespace());
  ink.map(|g| g.bounds.x0).fold(f32::MAX, f32::min)
}

fn right_edge(line: &Line) -> f32 {
  let ink = line.glyphs.iter().filter(|g| !g.ch.is_whitespace());
  ink.map(|g| g.bounds.x1).fold(f32::MIN, f32::max)
}

/// Whether `next`'s first word had room after `line`; wrapped prose
/// fills the line, so the break was deliberate.
fn fits(line: &Line, next: &Line, right: f32) -> bool {
  let mut word = next
    .glyphs
    .iter()
    .skip_while(|g| g.ch.is_whitespace())
    .take_while(|g| !g.ch.is_whitespace());
  let Some(first) = word.next() else { return false };
  let (mut x1, mut chars) = (first.bounds.x1, 1.0);
  for glyph in word {
    x1 = glyph.bounds.x1;
    chars += 1.0;
  }
  let width = x1 - first.bounds.x0;
  // An average glyph stands in for the space before the word.
  right - right_edge(line) > width + width / chars
}

/// Joins `lines` into running text. Wrapped lines meet with a space, or
/// none between CJK characters or inside a URL; split words lose their
/// hyphen unless `context` spells them hyphenated and never whole.
pub fn reflow(lines: &[(Join, String)], context: &str) -> String {
  let mut out = String::new();
  for (index, (join, line)) in lines.iter().enumerate() {
    let line = line.trim();
    if index > 0 {
      match join {
        Join::Paragraph => out.push_str("\n\n"),
        Join::Line => out.push('\n'),
        Join::Wrap if in_url(&out) => {}
        Join::Wrap => match hyphen(&out, line, context) {
          Hyphen::Split => {
            out.pop();
          }
          Hyphen::Compound => {}
          Hyphen::Absent if unspaced(&out, line) => {}
          Hyphen::Absent => out.push(' '),
        },
      }
    }
    out.push_str(line);
  }
  out
}

/// A hyphen ending a line before a word continues on the next.
enum Hyphen {
  Absent,
  /// Breaks one word; it goes.
  Split,
  /// Belongs to a compound like `self-attention`; it stays.
  Compound,
}

/// Classifies the hyphen ending `before` when `next` continues its word.
fn hyphen(before: &str, next: &str, context: &str) -> Hyphen {
  let mut tail = before.chars().rev();
  let Some(hyphen @ ('-' | '\u{ad}' | '\u{2010}')) = tail.next() else {
    return Hyphen::Absent;
  };
  if !tail.next().is_some_and(char::is_alphabetic)
    || !next.chars().next().is_some_and(char::is_lowercase)
  {
    return Hyphen::Absent;
  }
  if hyphen == '\u{ad}' {
    return Hyphen::Split;
  }
  let head = before.rsplit(char::is_whitespace).next().unwrap_or_default();
  let rest = next.split(char::is_whitespace).next().unwrap_or_default();
  let word = format!("{head}{rest}");
  let word = word.trim_matches(|ch: char| !ch.is_alphanumeric());
  let whole = word.replacen(hyphen, "", 1);
  if contains_word(context, word) && !contains_word(context, &whole) {
    Hyphen::Compound
  } else {
    Hyphen::Split
  }
}

/// `word` appears in `text` without letters on either side.
fn contains_word(text: &str, word: &str) -> bool {
  text.match_indices(word).any(|(at, _)| {
    let before = text[..at].chars().next_back();
    let after = text[at + word.len()..].chars().next();
    !before.is_some_and(char::is_alphanumeric)
      && !after.is_some_and(char::is_alphanumeric)
  })
}

/// `before` ends inside a URL, wrapped after one of its separators.
pub fn in_url(before: &str) -> bool {
  let word = before.rsplit(char::is_whitespace).next().unwrap_or_default();
  word.contains("://") && word.ends_with(['/', '-', '_', '?', '=', '&', '#'])
}

/// Scripts written without spaces between words.
fn unspaced(before: &str, next: &str) -> bool {
  before.chars().next_back().is_some_and(tight)
    || next.chars().next().is_some_and(tight)
}

/// A character of a script written without spaces.
pub fn tight(ch: char) -> bool {
  matches!(ch,
    '\u{0E00}'..='\u{0E7F}' // Thai
    | '\u{3000}'..='\u{30FF}' // CJK punctuation, kana
    | '\u{3400}'..='\u{4DBF}'
    | '\u{4E00}'..='\u{9FFF}'
    | '\u{F900}'..='\u{FAFF}'
    | '\u{FF00}'..='\u{FFEF}' // full-width forms
    | '\u{20000}'..='\u{2FFFF}')
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Bounds, Glyph};

  #[test]
  fn reflow_joins_wrapped_lines() {
    use Join::{Line, Paragraph, Wrap};
    let context =
      "self-attention and floating-point, also cooperate co-operate";
    let cases = [
      (vec![(Paragraph, "one")], "one"),
      (vec![(Paragraph, "a long "), (Wrap, " line")], "a long line"),
      (vec![(Paragraph, "distri-"), (Wrap, "bution")], "distribution"),
      (vec![(Paragraph, "distri\u{ad}"), (Wrap, "bution")], "distribution"),
      (
        vec![(Paragraph, "uses self-"), (Wrap, "attention.")],
        "uses self-attention.",
      ),
      (vec![(Paragraph, "(floating-"), (Wrap, "point)")], "(floating-point)"),
      (vec![(Paragraph, "co-"), (Wrap, "operate")], "cooperate"),
      (vec![(Paragraph, "Jean-"), (Wrap, "Paul")], "Jean- Paul"),
      (vec![(Paragraph, "1999-"), (Wrap, "now")], "1999- now"),
      (vec![(Paragraph, "end."), (Paragraph, "Next")], "end.\n\nNext"),
      (vec![(Paragraph, ">>> 1"), (Line, ">>> 2")], ">>> 1\n>>> 2"),
      (vec![(Paragraph, "日本語の"), (Wrap, "文章")], "日本語の文章"),
      (vec![(Paragraph, "한국어"), (Wrap, "문장")], "한국어 문장"),
      (
        vec![(Paragraph, "see https://a.jp/"), (Wrap, "b/c")],
        "see https://a.jp/b/c",
      ),
      (
        vec![(Paragraph, "at https://a-"), (Wrap, "b.org")],
        "at https://a-b.org",
      ),
      (
        vec![(Paragraph, "https://a.org."), (Wrap, "Next")],
        "https://a.org. Next",
      ),
    ];
    for (lines, want) in cases {
      let lines: Vec<_> =
        lines.iter().map(|(join, text)| (*join, (*text).to_owned())).collect();
      assert_eq!(reflow(&lines, context), want, "{lines:?}");
    }
  }

  #[test]
  fn continuations_sit_below_full_rows() {
    // Glyphs are 5 pt wide and 10 pt tall; rows 12 pt apart.
    let at = |x: f32, y: f32, text: &str| Line {
      paragraph: true,
      glyphs: text
        .chars()
        .zip(0..)
        .map(|(ch, index)| {
          let x0 = x + index as f32 * 5.0;
          Glyph { ch, bounds: Bounds { x0, y0: y, x1: x0 + 5.0, y1: y + 10.0 } }
        })
        .collect(),
    };
    let full = |y| at(0.0, y, &"x".repeat(20));
    let cases = [
      ("next row", vec![full(0.0), full(12.0), full(24.0)], Some(1)),
      (
        "past a margin number",
        vec![full(0.0), at(-30.0, 0.0, "520"), full(12.0), full(24.0)],
        Some(2),
      ),
      (
        "short row",
        vec![
          at(0.0, 0.0, "xxxx"),
          at(0.0, 12.0, "ab cd"),
          full(24.0),
          full(36.0),
          full(48.0),
        ],
        None,
      ),
      ("too far below", vec![full(0.0), full(40.0), full(52.0)], None),
      ("other column", vec![full(0.0), at(200.0, 24.0, "y"), full(36.0)], None),
      (
        "past a lower margin number",
        vec![full(0.0), at(-30.0, 12.0, "5"), full(12.0), full(24.0)],
        Some(2),
      ),
      ("no shared margin", vec![full(0.0), at(0.0, 12.0, "yy")], None),
    ];
    for (name, lines, want) in cases {
      let text = Text { lines, truncated: false };
      assert_eq!(continuation(&text, 0), want, "{name}");
    }
  }

  #[test]
  fn short_lines_break_deliberately() {
    // Each glyph is 5 pt wide; lines start at x = 0.
    let line = |paragraph, text: &str| Line {
      paragraph,
      glyphs: text
        .chars()
        .zip(0..)
        .map(|(ch, at)| {
          let x0 = at as f32 * 5.0;
          Glyph { ch, bounds: Bounds { x0, y0: 0.0, x1: x0 + 5.0, y1: 1.0 } }
        })
        .collect(),
    };
    let mut fragment = line(true, "row");
    for glyph in &mut fragment.glyphs {
      glyph.bounds.x0 += 70.0;
      glyph.bounds.x1 += 70.0;
    }
    let text = Text {
      lines: vec![
        line(true, "an OCR"),
        fragment,
        line(true, "a full line of prose ok"),
        line(false, "that wraps right at the"),
        line(false, "edge and wraps again ok"),
        line(false, "at the same right edge."),
        line(false, "then ends short."),
        line(false, "x = 1"),
        line(false, "y"),
        line(true, ">>> a = 1"),
        line(false, ">>> b = 2"),
        line(false, ">>> c = 3"),
      ],
      truncated: false,
    };
    let (paragraph, line, wrap) = (Join::Paragraph, Join::Line, Join::Wrap);
    let want = [paragraph, wrap, paragraph, wrap, wrap, wrap, wrap, line, line];
    let want = [&want[..], &[paragraph, line, line]].concat();
    assert_eq!(joins(&text, 0..text.lines.len()), want);
    assert_eq!(joins(&text, 3..5), want[3..5]);
  }
}
