use mupdf::text_page::{TextChar, TextCharFlags};

/// Font facts of one extracted line, which glyph boxes lack.
#[derive(Clone, Copy, Debug, Default)]
pub struct LineStyle {
  /// Median font size in points.
  pub size: f32,
  pub bold: bool,
  pub mono: bool,
  /// Weight or size changes within the line, as in run-in labels:
  /// "**12** Prove that".
  pub mixed: bool,
}

/// Bold and monospace from the font, which names them when flags do not.
/// Each lookup is a native call, so callers sample a few letters.
fn typeface(glyph: &TextChar) -> (bool, bool) {
  let Some(font) = glyph.font() else {
    return (false, false);
  };
  let name = font.name();
  let named = |marks: &[&str]| marks.iter().any(|m| name.contains(m));
  let bold = font.is_bold()
    || named(&[
      "Bold", "Black", "Heavy", "Semibold", "SemiBold", "Demi", "CMBX",
      "CMMIB", "CMBSY",
    ]);
  let mono = font.is_monospaced()
    || named(&["Mono", "Courier", "Consol", "CMTT", "Code"]);
  (bold, mono)
}

/// Letters a line's weight is read from; each costs a native call.
const SAMPLES: usize = 5;
/// Line ends differing more in size mix styles.
const SIZE_STEP: f32 = 1.15;

pub fn style(line: &mupdf::TextLine) -> LineStyle {
  let visible: Vec<_> = line
    .chars()
    .filter(|g| g.char().is_some_and(|c| !c.is_whitespace() && !c.is_control()))
    .collect();
  let mut sizes: Vec<f32> = visible.iter().map(TextChar::size).collect();
  sizes.sort_by(f32::total_cmp);
  let flagged =
    visible.iter().filter(|g| g.flags().contains(TextCharFlags::BOLD)).count();
  let face = |glyph: &TextChar| {
    let (bold, mono) = typeface(glyph);
    (bold || glyph.flags().contains(TextCharFlags::BOLD), mono)
  };
  // Letters spread over the line; symbols use their own fonts.
  let letters: Vec<_> = visible
    .iter()
    .filter(|g| g.char().is_some_and(char::is_alphanumeric))
    .collect();
  let mut picks: Vec<usize> = (0..SAMPLES)
    .map(|at| at * letters.len().saturating_sub(1) / (SAMPLES - 1))
    .collect();
  picks.dedup();
  let faces: Vec<_> = picks
    .into_iter()
    .filter_map(|at| letters.get(at))
    .map(|glyph| face(glyph))
    .collect();
  let monos = faces.iter().filter(|(_, mono)| *mono).count();
  // Code words keep their own weight: "The `while` statement".
  let prose: Vec<bool> =
    faces.iter().filter(|(_, mono)| !mono).map(|(bold, _)| *bold).collect();
  let weights = if prose.is_empty() {
    faces.iter().map(|(bold, _)| *bold).collect()
  } else {
    prose
  };
  let ends = (visible.first(), visible.last());
  let (first, last) = (ends.0.map(TextChar::size), ends.1.map(TextChar::size));
  let (first, last) = (first.unwrap_or_default(), last.unwrap_or_default());
  LineStyle {
    size: sizes.get(sizes.len() / 2).copied().unwrap_or_default(),
    bold: flagged * 2 > visible.len()
      || flagged == 0 && !weights.is_empty() && weights.iter().all(|b| *b),
    mono: monos * 2 > faces.len(),
    // Footnote marks end in a smaller size: "Data11".
    mixed: weights.iter().any(|b| *b != weights[0])
      || first.max(last) > first.min(last) * SIZE_STEP,
  }
}
