use serde::{Deserialize, Serialize};

#[derive(
  Clone,
  Copy,
  Debug,
  Default,
  Deserialize,
  Eq,
  Hash,
  Ord,
  PartialEq,
  PartialOrd,
  Serialize,
)]
pub struct PageId(pub usize);

#[derive(
  Clone,
  Copy,
  Debug,
  Default,
  Deserialize,
  Eq,
  Ord,
  PartialEq,
  PartialOrd,
  Serialize,
)]
pub struct Position {
  pub page: PageId,
  pub line: usize,
  pub glyph: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
  pub x0: f32,
  pub y0: f32,
  pub x1: f32,
  pub y1: f32,
}

impl Bounds {
  pub fn width(self) -> f32 {
    (self.x1 - self.x0).max(0.0)
  }

  pub fn height(self) -> f32 {
    (self.y1 - self.y0).max(0.0)
  }

  /// Smallest box holding every point.
  pub fn enclosing(points: &[(f32, f32)]) -> Self {
    let fold =
      |pick: fn(&(f32, f32)) -> f32, start: f32, f: fn(f32, f32) -> f32| {
        points.iter().map(pick).fold(start, f)
      };
    Self {
      x0: fold(|p| p.0, f32::INFINITY, f32::min),
      y0: fold(|p| p.1, f32::INFINITY, f32::min),
      x1: fold(|p| p.0, f32::NEG_INFINITY, f32::max),
      y1: fold(|p| p.1, f32::NEG_INFINITY, f32::max),
    }
  }

  /// Smallest box holding both.
  #[must_use]
  pub fn union(self, other: Self) -> Self {
    Self {
      x0: self.x0.min(other.x0),
      y0: self.y0.min(other.y0),
      x1: self.x1.max(other.x1),
      y1: self.y1.max(other.y1),
    }
  }

  pub fn centre(self) -> (f32, f32) {
    (f32::midpoint(self.x0, self.x1), f32::midpoint(self.y0, self.y1))
  }

  pub fn contains(self, x: f32, y: f32) -> bool {
    x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
  }
}

#[derive(Clone, Debug)]
pub struct Glyph {
  pub ch: char,
  pub bounds: Bounds,
}

#[derive(Clone, Debug, Default)]
pub struct Line {
  pub paragraph: bool,
  pub glyphs: Vec<Glyph>,
}

impl Line {
  /// Box around every glyph; None when empty.
  pub fn span(&self) -> Option<Bounds> {
    self.glyphs.iter().map(|glyph| glyph.bounds).reduce(Bounds::union)
  }

  pub fn text(&self) -> String {
    self.glyphs.iter().map(|g| g.ch).collect()
  }

  /// First and last glyph of the run that shares `glyph`'s class.
  pub fn run(&self, glyph: usize) -> Option<(usize, usize)> {
    self.run_by(glyph, class)
  }

  pub fn run_by(
    &self,
    glyph: usize,
    classify: impl Fn(char) -> u8,
  ) -> Option<(usize, usize)> {
    let kind = classify(self.glyphs.get(glyph)?.ch);
    let other = |g: &Glyph| classify(g.ch) != kind;
    let first =
      self.glyphs[..glyph].iter().rposition(other).map_or(0, |i| i + 1);
    let last = self.glyphs[glyph..]
      .iter()
      .position(other)
      .map_or(self.glyphs.len(), |i| glyph + i);
    Some((first, last - 1))
  }
}

#[derive(Clone, Debug, Default)]
pub struct Text {
  pub lines: Vec<Line>,
  pub truncated: bool,
}

impl Text {
  /// Selected glyphs, preserving lines and spacing.
  pub fn in_region(&self, bounds: Bounds) -> String {
    self
      .lines
      .iter()
      .map(|line| {
        line
          .glyphs
          .iter()
          .filter(|glyph| {
            let (x, y) = glyph.bounds.centre();
            bounds.contains(x, y)
          })
          .map(|glyph| glyph.ch)
          .collect::<String>()
      })
      .filter(|line| !line.trim().is_empty())
      .collect::<Vec<_>>()
      .join("\n")
  }

  pub fn plain(&self) -> String {
    self.lines.iter().map(Line::text).collect::<Vec<_>>().join("\n")
  }

  /// First position and text within areas.
  pub fn in_areas(
    &self,
    page: PageId,
    areas: &[Bounds],
  ) -> (Option<Position>, String) {
    let mut out = String::new();
    let mut first = None;
    for (line, row) in self.lines.iter().enumerate() {
      let before = out.len();
      for (glyph, ch) in row.glyphs.iter().enumerate() {
        let (x, y) = ch.bounds.centre();
        if areas.iter().any(|area| area.contains(x, y)) {
          first.get_or_insert(Position { page, line, glyph });
          out.push(ch.ch);
        }
      }
      if out.len() > before {
        out.push(' ');
      }
    }
    (first, out.split_whitespace().collect::<Vec<_>>().join(" "))
  }

  /// On a glyph or a line gap.
  pub fn covers(&self, x: f32, y: f32) -> bool {
    let mut glyphs = self.lines.iter().flat_map(|line| &line.glyphs);
    glyphs.any(|glyph| glyph.bounds.contains(x, y))
      || self
        .lines
        .windows(2)
        .any(|pair| !pair[1].paragraph && bridges(&pair[0], &pair[1], x, y))
  }

  pub fn weight(&self) -> usize {
    self.lines.iter().map(|l| l.glyphs.capacity() * size_of::<Glyph>()).sum()
  }

  pub fn glyph(&self, pos: Position) -> Option<&Glyph> {
    self.lines.get(pos.line)?.glyphs.get(pos.glyph)
  }

  pub fn clamp(&self, mut pos: Position) -> Position {
    pos.line = pos.line.min(self.lines.len().saturating_sub(1));
    pos.glyph = self
      .lines
      .get(pos.line)
      .map_or(0, |line| pos.glyph.min(line.glyphs.len().saturating_sub(1)));
    pos
  }

  pub fn nearest(&self, page: PageId, x: f32, y: f32) -> Position {
    let mut best = Position { page, ..Position::default() };
    let mut distance = f32::MAX;
    for (line, row) in self.lines.iter().enumerate() {
      for (glyph, ch) in row.glyphs.iter().enumerate() {
        let b = ch.bounds;
        let dx = (b.x0 - x).max(0.0).max(x - b.x1);
        let dy = (b.y0 - y).max(0.0).max(y - b.y1);
        let score = dx * dx + 4.0 * dy * dy;
        if score < distance {
          distance = score;
          best = Position { page, line, glyph };
        }
      }
    }
    best
  }
}

/// In a line gap, nearer a spanning line.
fn bridges(upper: &Line, lower: &Line, x: f32, y: f32) -> bool {
  // Spans hold first glyphs: a cheap bound.
  let (Some(top), Some(bottom)) = (upper.glyphs.first(), lower.glyphs.first())
  else {
    return false;
  };
  if y < top.bounds.y1 || y > bottom.bounds.y0 {
    return false;
  }
  let (Some(upper), Some(lower)) = (upper.span(), lower.span()) else {
    return false;
  };
  let middle = f32::midpoint(upper.y1, lower.y0);
  let within = |line: Bounds, from: f32, to: f32| {
    (line.x0..=line.x1).contains(&x) && (from..=to).contains(&y)
  };
  within(upper, upper.y1, middle) || within(lower, middle, lower.y0)
}

pub fn class(ch: char) -> u8 {
  if ch.is_whitespace() {
    0
  } else if ch.is_alphanumeric() || ch == '_' {
    1
  } else {
    2
  }
}

/// Drops controls; whitespace controls become spaces.
pub fn printable(raw: &str) -> impl Iterator<Item = char> + '_ {
  raw
    .chars()
    .filter(|ch| !ch.is_control() || ch.is_whitespace())
    .map(|ch| if ch.is_control() { ' ' } else { ch })
}

/// Printable text within cell columns.
pub fn clean(text: &str, columns: usize) -> String {
  use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
  let mut width = 0;
  let mut clean: String = printable(text)
    // Caps zero-width marks from hostile text.
    .take(columns.saturating_mul(8))
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= columns
    })
    .collect();
  // Variation selectors can widen whole sequences.
  while clean.width() > columns {
    // Marks follow their base.
    while clean.pop().is_some_and(|ch| ch.width() == Some(0)) {}
  }
  clean
}

/// Printable text with ellipsis when truncated.
pub fn clipped(text: &str, columns: usize) -> (String, bool) {
  use unicode_width::UnicodeWidthStr;
  let clean = clean(text, columns);
  if clean.width() < text.width() && columns > 0 {
    (format!("{}…", self::clean(text, columns - 1)), true)
  } else {
    (clean, false)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn clipping_preserves_cell_limits() {
    use unicode_width::UnicodeWidthStr;
    for (text, columns, expected, cut) in [
      ("", 4, "", false),
      ("long", 0, "", false),
      ("long", 1, "…", true),
      ("fit", 3, "fit", false),
      ("a\nb\x1b", 3, "a …", true),
      ("世界", 3, "世…", true),
      ("🦀x", 2, "…", true),
      ("e\u{301}x", 1, "…", true),
    ] {
      assert_eq!(clipped(text, columns), (expected.into(), cut));
      assert!(expected.width() <= columns);
    }
    let hostile = format!("a{}", "\u{301}".repeat(10_000));
    let (text, cut) = clipped(&hostile, 1);
    assert!(text.chars().count() <= 8);
    assert_eq!(text.width(), 1);
    assert!(!cut);
  }

  #[test]
  fn text_centers_preserve_order() {
    let text = Text {
      lines: ["abcd", "efgh"]
        .into_iter()
        .enumerate()
        .map(|(line, value)| Line {
          paragraph: true,
          glyphs: value
            .chars()
            .enumerate()
            .map(|(index, ch)| Glyph {
              ch,
              bounds: Bounds {
                x0: index as f32 * 10.0,
                x1: index as f32 * 10.0 + 10.0,
                y0: line as f32 * 10.0,
                y1: line as f32 * 10.0 + 10.0,
              },
            })
            .collect(),
        })
        .collect(),
      truncated: false,
    };
    let bounds = Bounds { x0: 10.0, y0: 0.0, x1: 30.0, y1: 20.0 };
    assert_eq!(text.in_region(bounds), "bc\nfg");
    assert_eq!(text.in_region(Bounds::default()), "");
    let mut spaced = text;
    spaced.lines[0].glyphs[0].ch = ' ';
    spaced.lines[0].glyphs[2].ch = ' ';
    for glyph in &mut spaced.lines[1].glyphs {
      glyph.ch = ' ';
    }
    let whole = Bounds { x0: 0.0, y0: 0.0, x1: 40.0, y1: 20.0 };
    assert_eq!(spaced.in_region(whole), " b d");
  }

  #[test]
  fn areas_extract_only_marked_glyphs() {
    let text = Text {
      lines: ["xCafé  ", "世界y"]
        .into_iter()
        .enumerate()
        .map(|(line, text)| Line {
          paragraph: false,
          glyphs: text
            .chars()
            .enumerate()
            .map(|(glyph, ch)| Glyph {
              ch,
              bounds: Bounds {
                x0: glyph as f32 * 10.0,
                y0: line as f32 * 20.0,
                x1: (glyph + 1) as f32 * 10.0,
                y1: line as f32 * 20.0 + 10.0,
              },
            })
            .collect(),
        })
        .collect(),
      truncated: false,
    };
    let first = Bounds { x0: 9.0, y0: 0.0, x1: 70.0, y1: 10.0 };
    let second = Bounds { x0: 0.0, y0: 20.0, x1: 21.0, y1: 30.0 };
    let page = PageId(3);
    for (areas, expected) in [
      (vec![first, second], "Café 世界"),
      (vec![first, first, second], "Café 世界"),
    ] {
      let (position, marked) = text.in_areas(page, &areas);
      assert_eq!(position, Some(Position { page, line: 0, glyph: 1 }));
      assert_eq!(marked, expected);
    }
    let blank = Bounds { x0: 0.0, y0: 40.0, x1: 50.0, y1: 50.0 };
    for areas in [vec![], vec![blank]] {
      assert_eq!(text.in_areas(page, &areas), (None, String::new()));
    }
  }

  #[test]
  fn text_bridges_line_gaps_within_a_block() {
    // Lines 10 tall, 10 apart; third starts a block.
    let line = |paragraph, top: f32, x1| Line {
      paragraph,
      glyphs: vec![
        Glyph {
          ch: 'a',
          bounds: Bounds { x0: 0.0, y0: top, x1: 10.0, y1: top + 10.0 },
        },
        Glyph {
          ch: 'b',
          bounds: Bounds { x0: 20.0, y0: top, x1, y1: top + 10.0 },
        },
      ],
    };
    let text = Text {
      lines: vec![
        line(true, 0.0, 100.0),
        line(false, 20.0, 50.0),
        line(true, 40.0, 100.0),
      ],
      truncated: false,
    };
    // (x, y) -> text
    let cases = [
      ((5.0, 5.0), true),
      // Between words on a line.
      ((15.0, 5.0), false),
      // Upper half: upper line's width.
      ((15.0, 12.0), true),
      ((90.0, 15.0), true),
      ((90.0, 16.0), false),
      ((40.0, 16.0), true),
      // Between blocks.
      ((5.0, 35.0), false),
      ((5.0, 45.0), true),
      ((5.0, 55.0), false),
      ((101.0, 5.0), false),
    ];
    for ((x, y), want) in cases {
      assert_eq!(text.covers(x, y), want, "{x},{y}");
    }
  }
}
