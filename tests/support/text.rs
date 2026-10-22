use vpdf_model::text::{Bounds, Glyph, Line, Text};

pub fn glyph(ch: char, bounds: Bounds) -> Glyph {
  Glyph { quad: None, ch, bounds }
}

pub fn line(glyphs: impl IntoIterator<Item = Glyph>) -> Line {
  Line { ocr: None, paragraph: false, glyphs: glyphs.into_iter().collect() }
}

/// `value` as glyphs without geometry.
pub fn plain(value: &str) -> Line {
  line(value.chars().map(|ch| glyph(ch, Bounds::default())))
}

pub fn text(lines: impl IntoIterator<Item = Line>) -> Text {
  Text { lines: lines.into_iter().collect(), truncated: false }
}
