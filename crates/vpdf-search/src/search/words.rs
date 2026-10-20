use super::{Pattern, join};
use crate::Result;
use fancy_regex::{Regex, RegexBuilder};
use icu_segmenter::{WordSegmenter, options::WordBreakInvariantOptions};

pub(super) fn regex(source: &str) -> Result<Regex> {
  Ok(RegexBuilder::new(source).backtrack_limit(100_000).build()?)
}

pub(super) fn boundaries(text: &str) -> Option<Box<[u32]>> {
  if text.is_ascii() || !text.chars().any(join::cjk) {
    return None;
  }
  Some(
    WordSegmenter::new_dictionary(WordBreakInvariantOptions::default())
      .segment_str(text)
      .map(|byte| byte as u32)
      .collect(),
  )
}

pub(super) fn word_at(
  line: &crate::text::Line,
  glyph: usize,
) -> Option<String> {
  let ch = line.glyphs.get(glyph)?.ch;
  if !ch.is_alphanumeric()
    && ch != '_'
    && !unicode_normalization::char::is_combining_mark(ch)
  {
    return None;
  }
  if line.glyphs.iter().all(|glyph| glyph.ch.is_ascii()) {
    let (first, last) = line.run(glyph)?;
    return Some(line.glyphs[first..=last].iter().map(|g| g.ch).collect());
  }
  let text = line.text();
  let byte = text.char_indices().nth(glyph)?.0;
  let segmenter =
    WordSegmenter::new_dictionary(WordBreakInvariantOptions::default());
  let mut start = 0;
  for end in segmenter.segment_str(&text) {
    if byte < end {
      return Some(text[start..end].to_owned());
    }
    start = end;
  }
  None
}

impl Pattern {
  #[inline]
  pub(super) fn visit(
    &self,
    text: &str,
    boundaries: Option<&[u32]>,
    mut hit: impl FnMut(usize, usize) -> bool,
  ) -> Result<()> {
    let Some(boundaries) = boundaries else {
      for found in self.regex.find_iter(text) {
        let found = found?;
        if found.start() != found.end() && !hit(found.start(), found.end()) {
          break;
        }
      }
      return Ok(());
    };
    if self.loose.get().is_none() {
      let _ = self.loose.set(regex(&self.source)?);
    }
    let loose = self.loose.get().expect("compiled search pattern");
    let is_boundary = |at| boundaries.binary_search(&(at as u32)).is_ok();
    let mut at = 0;
    let mut guarded = None;
    while let Some(mut found) = loose.find_from_pos(text, at)? {
      if self.raw && is_boundary(found.start()) && !is_boundary(found.end()) {
        if guarded.is_none() {
          guarded = Some(self.guarded(text, boundaries)?);
        }
        let Some(retry) = guarded
          .as_ref()
          .expect("boundary pattern")
          .find_from_pos(text, found.start())?
        else {
          break;
        };
        found = retry;
      }
      if found.start() != found.end()
        && is_boundary(found.start())
        && is_boundary(found.end())
      {
        if !hit(found.start(), found.end()) {
          break;
        }
        at = found.end();
      } else {
        let Some(ch) = text[found.start()..].chars().next() else { break };
        at = found.start() + ch.len_utf8();
      }
    }
    Ok(())
  }

  #[cold]
  fn guarded(&self, text: &str, boundaries: &[u32]) -> Result<Regex> {
    let total = text.chars().count();
    let mut chars = text.char_indices().enumerate().peekable();
    let mut choices = Vec::with_capacity(boundaries.len());
    for &byte in boundaries {
      while chars.peek().is_some_and(|(_, (at, _))| *at < byte as usize) {
        chars.next();
      }
      let at = chars.peek().map_or(total, |(at, _)| *at);
      choices.push(total - at);
    }
    choices.reverse();
    let guard = format!("(?={})", tails(&choices, 0));
    regex(&format!("(?:{}){guard}", self.source))
  }
}

// Atomic units avoid oversized delegated NFAs.
fn tails(counts: &[usize], base: usize) -> String {
  let first = counts[0];
  let last = counts[counts.len() - 1];
  if last - first + 1 == counts.len() {
    return format!(r"{}(?s:.{{0,{}}})\z", skip(first - base), last - first);
  }
  let mid = counts.len() / 2;
  let left = tails(&counts[..mid], first);
  let right = tails(&counts[mid..], first);
  format!(r"{}(?:{left}|{right})", skip(first - base))
}

fn skip(count: usize) -> String {
  format!(r"(?>(?s:.{{64}})){{{}}}(?s:.{{{}}})", count / 64, count % 64)
}
