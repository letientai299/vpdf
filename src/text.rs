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

#[derive(Clone, Copy, Debug, Default, Serialize)]
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

  pub fn contains(self, x: f32, y: f32) -> bool {
    x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
  }
}

impl From<mupdf::Rect> for Bounds {
  fn from(r: mupdf::Rect) -> Self {
    Self { x0: r.x0, y0: r.y0, x1: r.x1, y1: r.y1 }
  }
}

#[derive(Clone, Debug, Serialize)]
pub struct Glyph {
  pub ch: char,
  pub bounds: Bounds,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Line {
  pub paragraph: bool,
  pub glyphs: Vec<Glyph>,
}

impl Line {
  pub fn text(&self) -> String {
    self.glyphs.iter().map(|g| g.ch).collect()
  }

  /// First and last glyph of the run that shares `glyph`'s class.
  pub fn run(&self, glyph: usize) -> Option<(usize, usize)> {
    self.run_by(glyph, class)
  }

  pub(crate) fn run_by(
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

#[derive(Clone, Debug, Default, Serialize)]
pub struct Text {
  pub lines: Vec<Line>,
  pub truncated: bool,
}

impl Text {
  pub fn plain(&self) -> String {
    self.lines.iter().map(Line::text).collect::<Vec<_>>().join("\n")
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
