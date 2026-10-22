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

#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
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
  pub quad: Option<Quad>,
}

/// Corners: upper-left, upper-right, lower-right, lower-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad(pub [(f32, f32); 4]);

impl From<Bounds> for Quad {
  fn from(b: Bounds) -> Self {
    Self([(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)])
  }
}

impl Glyph {
  pub fn quad(&self) -> Quad {
    self.quad.unwrap_or_else(|| self.bounds.into())
  }
}

impl Quad {
  pub fn is_rect(self) -> bool {
    (0..4).all(|i| {
      let (a, b) = (self.0[i], self.0[(i + 1) % 4]);
      (a.0 - b.0).abs() < 0.001 || (a.1 - b.1).abs() < 0.001
    })
  }

  pub fn bounds(self) -> Bounds {
    Bounds::enclosing(&self.0)
  }

  #[must_use]
  pub fn widen(self, minimum: f32) -> Self {
    let [start, end, lower_end, lower_start] = self.0;
    let mut direction = (end.0 - start.0, end.1 - start.1);
    let width = direction.0.hypot(direction.1);
    if width >= minimum {
      return self;
    }
    if width > f32::EPSILON {
      direction = (direction.0 / width, direction.1 / width);
    } else {
      direction = (lower_start.1 - start.1, start.0 - lower_start.0);
      let height = direction.0.hypot(direction.1);
      if height <= f32::EPSILON {
        return self;
      }
      direction = (direction.0 / height, direction.1 / height);
    }
    let extend = |p: (f32, f32)| {
      (
        p.0 + direction.0 * (minimum - width),
        p.1 + direction.1 * (minimum - width),
      )
    };
    Self([start, extend(end), extend(lower_end), lower_start])
  }

  #[must_use]
  pub fn band(self, page: Bounds) -> Self {
    let [start, end, lower_end, lower_start] = self.0;
    let u = (end.0 - start.0, end.1 - start.1);
    let length = u.0 * u.0 + u.1 * u.1;
    if length <= f32::EPSILON {
      return self;
    }
    let mut extent = Bounds::enclosing(&Self::from(page).0.map(|p| {
      (((p.0 - start.0) * u.0 + (p.1 - start.1) * u.1) / length, 0.0)
    }));
    let shear = ((lower_start.0 - start.0) * u.0
      + (lower_start.1 - start.1) * u.1)
      / length;
    extent.x0 -= shear.max(0.0);
    extent.x1 -= shear.min(0.0);
    let shift = |p: (f32, f32), t: f32| (p.0 + t * u.0, p.1 + t * u.1);
    Self([
      shift(start, extent.x0),
      shift(end, extent.x1 - 1.0),
      shift(lower_end, extent.x1 - 1.0),
      shift(lower_start, extent.x0),
    ])
  }

  pub fn span(quads: impl IntoIterator<Item = Self>) -> Option<Self> {
    let mut quads = quads.into_iter();
    let mut leading = Vec::new();
    let (origin, u, v, det) = loop {
      let Some(first) = quads.next() else {
        return leading
          .into_iter()
          .map(Self::bounds)
          .reduce(Bounds::union)
          .map(Self::from);
      };
      let [origin, right, _, bottom] = first.0;
      let u = (right.0 - origin.0, right.1 - origin.1);
      let v = (bottom.0 - origin.0, bottom.1 - origin.1);
      let det = u.0 * v.1 - u.1 * v.0;
      if det.abs() > f32::EPSILON {
        break (origin, u, v, det);
      }
      leading.push(first);
    };
    let local = |(x, y): (f32, f32)| {
      let (x, y) = (x - origin.0, y - origin.1);
      ((x * v.1 - y * v.0) / det, (y * u.0 - x * u.1) / det)
    };
    let bounds = leading
      .into_iter()
      .chain(quads)
      .fold(Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 }, |b, q| {
        b.union(Bounds::enclosing(&q.0.map(local)))
      });
    Some(Self(Self::from(bounds).0.map(|(x, y)| {
      (origin.0 + x * u.0 + y * v.0, origin.1 + x * u.1 + y * v.1)
    })))
  }
}

#[derive(Clone, Debug, Default)]
pub struct Line {
  pub paragraph: bool,
  pub glyphs: Vec<Glyph>,
  /// Cached OCR span; glyph boxes remain unchanged.
  pub ocr: Option<Quad>,
}

impl Line {
  pub fn quad(&self, range: std::ops::Range<usize>) -> Option<Quad> {
    let glyphs = self.glyphs.get(range)?;
    let (first, last) = (glyphs.first()?, glyphs.last()?);
    let Some(quad) = self.ocr else {
      if glyphs.len() == 1 {
        return Some(first.quad());
      }
      return Quad::span(glyphs.iter().map(Glyph::quad));
    };
    let [upper_left, upper_right, lower_right, lower_left] = quad.0;
    let width = upper_right.0 - upper_left.0;
    if width <= f32::EPSILON {
      return Some(quad);
    }
    let edge = |x: f32, start: (f32, f32), end: (f32, f32)| {
      let along = ((x - upper_left.0) / width).clamp(0.0, 1.0);
      (x, start.1 + along * (end.1 - start.1))
    };
    Some(Quad([
      edge(first.bounds.x0, upper_left, upper_right),
      edge(last.bounds.x1, upper_left, upper_right),
      edge(last.bounds.x1, lower_left, lower_right),
      edge(first.bounds.x0, lower_left, lower_right),
    ]))
  }

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
    self.lines.capacity() * size_of::<Line>()
      + self
        .lines
        .iter()
        .map(|l| l.glyphs.capacity() * size_of::<Glyph>())
        .sum::<usize>()
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
          ocr: None,
          paragraph: true,
          glyphs: value
            .chars()
            .enumerate()
            .map(|(index, ch)| Glyph {
              quad: None,
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
          ocr: None,
          paragraph: false,
          glyphs: text
            .chars()
            .enumerate()
            .map(|(glyph, ch)| Glyph {
              quad: None,
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
      ocr: None,
      paragraph,
      glyphs: vec![
        Glyph {
          quad: None,
          ch: 'a',
          bounds: Bounds { x0: 0.0, y0: top, x1: 10.0, y1: top + 10.0 },
        },
        Glyph {
          quad: None,
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

#[cfg(test)]
mod quad_tests {
  use super::*;

  #[test]
  fn cursors_keep_oriented_width() {
    let quad = Quad([(10.0, 10.0), (10.0, 11.0), (0.0, 11.0), (0.0, 10.0)]);
    assert_eq!(
      quad.widen(2.0),
      Quad([(10.0, 10.0), (10.0, 12.0), (0.0, 12.0), (0.0, 10.0)])
    );
    let zero = Quad([(10.0, 10.0), (10.0, 10.0), (0.0, 10.0), (0.0, 10.0)]);
    assert_eq!(
      zero.widen(2.0),
      Quad([(10.0, 10.0), (10.0, 12.0), (0.0, 12.0), (0.0, 10.0)])
    );
  }

  #[test]
  fn spans_preserve_skew_and_reverse_order() {
    let first = Quad([(10.0, 10.0), (20.0, 5.0), (22.0, 15.0), (12.0, 20.0)]);
    let last = Quad(first.0.map(|(x, y)| (x + 20.0, y - 10.0)));
    let expected = Quad([first.0[0], last.0[1], last.0[2], first.0[3]]);
    assert_eq!(Quad::span([first, last]), Some(expected));
    assert_eq!(Quad::span([last, first]), Some(expected));
    assert_eq!(Quad::span([]), None);
    let band = first.band(Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 100.0 });
    let [a, b, c, d] = band.0;
    assert!(((b.1 - a.1) / (b.0 - a.0) + 0.5).abs() < 0.001);
    assert!((d.0 - a.0 - 2.0).abs() < 0.001);
    assert!((c.1 - b.1 - 10.0).abs() < 0.001);
  }
  #[test]
  fn leading_marks_preserve_rotation() {
    let mark = Quad([(10.0, 10.0), (10.0, 10.0), (0.0, 20.0), (0.0, 20.0)]);
    let glyph = Quad([(10.0, 10.0), (20.0, 20.0), (10.0, 30.0), (0.0, 20.0)]);
    assert_eq!(Quad::span([mark, glyph]), Some(glyph));
  }

  #[test]
  fn sheared_bands_reach_edges() {
    let page = Bounds { x0: 0.0, y0: 0.0, x1: 100.0, y1: 100.0 };
    for shear in [-100.0, 100.0] {
      let quad = Quad([
        (20.0, 20.0),
        (30.0, 20.0),
        (30.0 + shear, 30.0),
        (20.0 + shear, 30.0),
      ]);
      let band = quad.band(page);
      for (start, end) in [(band.0[0], band.0[1]), (band.0[3], band.0[2])] {
        assert!(start.0 <= 0.0 && end.0 >= 100.0, "{band:?}");
      }
    }
  }
}

#[cfg(test)]
mod ocr_tests {
  use super::*;

  #[test]
  fn native_marks_keep_direction() {
    let quad = Quad([(10.0, 10.0), (10.0, 10.0), (0.0, 10.0), (0.0, 10.0)]);
    let line = Line {
      glyphs: vec![Glyph {
        ch: '\u{301}',
        bounds: quad.bounds(),
        quad: Some(quad),
      }],
      ..Line::default()
    };
    let cursor = line.quad(0..1).unwrap();
    assert_eq!(cursor, quad);
    assert_eq!(cursor.widen(2.0), quad.widen(2.0));
  }

  #[test]
  fn cached_selection_geometry() {
    let bounds = Bounds { x0: 10.0, y0: 20.0, x1: 50.0, y1: 30.0 };
    let q = Quad([(10.0, 20.0), (50.0, 16.0), (50.0, 26.0), (10.0, 30.0)]);
    let line = Line {
      ocr: Some(q),
      glyphs: (0..4)
        .map(|i| Glyph {
          ch: 'x',
          quad: None,
          bounds: Bounds {
            x0: 10.0 + i as f32 * 10.0,
            x1: 20.0 + i as f32 * 10.0,
            ..bounds
          },
        })
        .collect(),
      ..Line::default()
    };
    assert_eq!(line.quad(0..4), Some(q));
    assert_eq!(
      line.quad(1..3),
      Some(Quad([(20.0, 19.0), (40.0, 17.0), (40.0, 27.0), (20.0, 29.0)]))
    );
    assert_eq!(
      line.quad(2..3),
      Some(Quad([(30.0, 18.0), (40.0, 17.0), (40.0, 27.0), (30.0, 28.0)]))
    );
    assert_eq!(line.span(), Some(bounds));
    assert_eq!(line.quad(2..2), None);
    assert_eq!(line.quad(4..5), None);
    let text = Text { lines: vec![line], truncated: false };
    assert_eq!(
      text.weight(),
      text.lines.capacity() * size_of::<Line>()
        + text.lines[0].glyphs.capacity() * size_of::<Glyph>()
    );
  }
}
