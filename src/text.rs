use serde::Serialize;

#[derive(
  Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct PageId(pub usize);

#[derive(
  Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize,
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
  pub glyphs: Vec<Glyph>,
}

impl Line {
  pub fn text(&self) -> String {
    self.glyphs.iter().map(|g| g.ch).collect()
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
