use crate::text::{Line, Text};
use unicode_normalization::char::{decompose_compatible, is_combining_mark};
use unicode_script::{Script, UnicodeScript};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Join {
  Space,
  Continue,
  Hyphen,
}

pub(super) fn cjk(ch: char) -> bool {
  !ch.is_ascii()
    && (matches!(
      ch.script(),
      Script::Han | Script::Hiragana | Script::Katakana
    ) || ch == 'ー')
}

pub(super) fn decisions(text: &Text) -> Vec<Join> {
  text
    .lines
    .iter()
    .enumerate()
    .map(|(at, line)| {
      if line.glyphs.last().is_some_and(|g| g.ch == '-') {
        return Join::Hyphen;
      }
      if text.lines.get(at + 1).is_some_and(|next| continuation(line, next)) {
        Join::Continue
      } else {
        Join::Space
      }
    })
    .collect()
}

fn continuation(line: &Line, next: &Line) -> bool {
  if next.paragraph {
    return false;
  }
  let Some(first) = next.glyphs.first() else {
    return false;
  };
  if !cjk(first.ch) || !cjk_tail(line) {
    return false;
  }
  let (Some(before), Some(after)) = (line.span(), next.span()) else {
    return false;
  };
  let height = before.height().max(after.height());
  let gap = after.y0 - before.y1;
  height > 0.0
    && gap >= -height * 0.25
    && gap <= height
    && (after.x0 - before.x0).abs() <= height * 2.0
    && after.x0 < before.x1
    && before.x0 < after.x1
}

fn cjk_tail(line: &Line) -> bool {
  for glyph in line.glyphs.iter().rev() {
    if cjk(glyph.ch) {
      return true;
    }
    let mut mark = true;
    decompose_compatible(glyph.ch, |ch| mark &= is_combining_mark(ch));
    if !mark {
      return false;
    }
  }
  false
}
