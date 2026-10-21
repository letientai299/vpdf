use crate::text::Bounds;
use serde::{Deserialize, Serialize};

/// Cell origin within a composed surface.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct At {
  pub column: u16,
  pub row: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Screen {
  pub columns: u16,
  pub rows: u16,
  pub cell_width: u16,
  pub cell_height: u16,
  pub origin: (u16, u16),
}

/// Pixel bounds require clipping.
///
/// ```compile_fail
/// let rect = vpdf_model::geometry::PixelRect { bits: [0; 4] };
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PixelRect {
  bits: [u32; 4],
}

impl PixelRect {
  pub fn clip(rect: Bounds, screen: Screen) -> Option<Self> {
    let clipped = Bounds {
      x0: rect.x0.max(0.0).floor(),
      y0: rect.y0.max(0.0).floor(),
      x1: rect.x1.min(screen.width()).ceil(),
      y1: rect.y1.min(screen.height()).ceil(),
    };
    if clipped.width() <= 0.0 || clipped.height() <= 0.0 {
      return None;
    }
    Some(Self {
      bits: [clipped.x0, clipped.y0, clipped.x1, clipped.y1].map(f32::to_bits),
    })
  }

  pub fn bounds(self) -> Bounds {
    let [x0, y0, x1, y1] = self.bits.map(f32::from_bits);
    Bounds { x0, y0, x1, y1 }
  }
}

/// Rows advanced while retaining reading context.
pub fn screen_step(rows: usize) -> usize {
  rows.saturating_sub(2).max(1)
}

impl Screen {
  /// Cell width and height in pixels.
  pub fn cell(self) -> (u16, u16) {
    (self.cell_width, self.cell_height)
  }

  pub fn width(self) -> f32 {
    f32::from(self.columns) * f32::from(self.cell_width)
  }
  pub fn height(self) -> f32 {
    f32::from(self.rows.saturating_sub(1)) * f32::from(self.cell_height)
  }
}

/// Margins trimmed from every page, in thousandths of its width and
/// height: left, top, right, bottom.
#[derive(
  Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize,
)]
pub struct Crop(pub [u16; 4]);

impl Crop {
  pub fn apply(self, bounds: Bounds) -> Bounds {
    let [left, top, right, bottom] =
      self.0.map(|v| f32::from(v.min(450)) / 1000.0);
    let (width, height) = (bounds.width(), bounds.height());
    Bounds {
      x0: bounds.x0 + width * left,
      y0: bounds.y0 + height * top,
      x1: bounds.x1 - width * right,
      y1: bounds.y1 - height * bottom,
    }
  }

  pub fn is_none(self) -> bool {
    self == Self::default()
  }
}

pub fn clip(bounds: Bounds, page: Bounds) -> Option<Bounds> {
  if ![
    bounds.x0, bounds.y0, bounds.x1, bounds.y1, page.x0, page.y0, page.x1,
    page.y1,
  ]
  .into_iter()
  .all(f32::is_finite)
  {
    return None;
  }
  let bounds = Bounds {
    x0: bounds.x0.max(page.x0),
    y0: bounds.y0.max(page.y0),
    x1: bounds.x1.min(page.x1),
    y1: bounds.y1.min(page.y1),
  };
  (bounds.x1 > bounds.x0 && bounds.y1 > bounds.y0).then_some(bounds)
}

/// Resolved crops shared with view layout.
#[derive(Clone, Default)]
pub struct Crops(std::collections::BTreeMap<crate::text::PageId, Crop>);

impl Crops {
  pub fn len(&self) -> usize {
    self.0.len()
  }
  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }
  pub fn get(&self, page: crate::text::PageId) -> Crop {
    self.0.get(&page).copied().unwrap_or_default()
  }
  pub fn contains(&self, page: crate::text::PageId) -> bool {
    self.0.contains_key(&page)
  }
  pub fn insert(&mut self, page: crate::text::PageId, crop: Crop) {
    self.0.insert(page, crop);
  }
}

/// Page coordinates to raster pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform([f32; 6]);

impl Default for Transform {
  fn default() -> Self {
    Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
  }
}

impl Transform {
  pub fn coefficients(self) -> [f32; 6] {
    self.0
  }

  pub fn transform_xy(self, x: f32, y: f32) -> (f32, f32) {
    let [xx, xy, yx, yy, tx, ty] = self.0;
    (x * xx + y * yx + tx, x * xy + y * yy + ty)
  }

  pub fn invert(self) -> Option<Self> {
    let [xx, xy, yx, yy, tx, ty] = self.0.map(f64::from);
    let det = xx * yy - xy * yx;
    if det.abs() <= f64::EPSILON {
      return None;
    }
    let inv = 1.0 / det;
    let (xx, xy, yx, yy) = (yy * inv, -xy * inv, -yx * inv, xx * inv);
    let values = [xx, xy, yx, yy, -tx * xx - ty * yx, -tx * xy - ty * yy]
      .map(|v| v as f32);
    values.into_iter().all(f32::is_finite).then_some(Self(values))
  }
}

pub fn page_matrix(bounds: Bounds, scale: u32, rotation: u16) -> Transform {
  let scale = scale as f32 / 1024.0;
  let (sin, cos) = match rotation % 360 {
    0 => (0.0, 1.0),
    90 => (1.0, 0.0),
    180 => (0.0, -1.0),
    270 => (-1.0, 0.0),
    degrees => {
      let radians = f32::from(degrees) * std::f32::consts::PI / 180.0;
      (radians.sin(), radians.cos())
    }
  };
  let mut matrix =
    Transform([cos * scale, sin * scale, -sin * scale, cos * scale, 0.0, 0.0]);
  let rect = transform(bounds, &matrix);
  matrix.0[4] -= rect.x0;
  matrix.0[5] -= rect.y0;
  matrix
}

pub fn transform(bounds: Bounds, matrix: &Transform) -> Bounds {
  let points = [
    (bounds.x0, bounds.y0),
    (bounds.x1, bounds.y0),
    (bounds.x0, bounds.y1),
    (bounds.x1, bounds.y1),
  ];
  Bounds::enclosing(&points.map(|(x, y)| matrix.transform_xy(x, y)))
}

/// Union area without counting overlapping rectangles twice.
pub fn union_area(rectangles: &[Bounds]) -> f32 {
  let mut edges: Vec<_> =
    rectangles.iter().flat_map(|r| [r.x0, r.x1]).collect();
  edges.sort_by(f32::total_cmp);
  edges.dedup();
  let mut area = 0.0;
  for edge in edges.windows(2) {
    let mut spans: Vec<_> = rectangles
      .iter()
      .filter(|r| r.x0 < edge[1] && r.x1 > edge[0])
      .map(|r| (r.y0, r.y1))
      .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut bottom = f32::NEG_INFINITY;
    let mut height = 0.0;
    for (top, end) in spans {
      height += (end - top.max(bottom)).max(0.0);
      bottom = bottom.max(end);
    }
    area += (edge[1] - edge[0]) * height;
  }
  area
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pixel_clips_round_edges() {
    let screen = Screen {
      columns: 10,
      rows: 5,
      cell_width: 2,
      cell_height: 4,
      origin: (3, 2),
    };
    let rect = Bounds { x0: -2.5, y0: 1.25, x1: 15.25, y1: 30.3 };
    let clipped = PixelRect::clip(rect, screen).unwrap().bounds();
    assert_eq!(
      (clipped.x0, clipped.y0, clipped.x1, clipped.y1),
      (0.0, 1.0, 16.0, 16.0)
    );
    assert_eq!(PixelRect::clip(clipped, screen).unwrap().bounds(), clipped);
    assert!(
      PixelRect::clip(Bounds { x0: 20.0, x1: 30.0, ..rect }, screen).is_none()
    );
    assert!(PixelRect::clip(rect, Screen { rows: 1, ..screen }).is_none());
  }
  #[test]
  fn rotations_preserve_origin_and_dimensions() {
    let bounds = Bounds { x0: 10.0, y0: 20.0, x1: 110.0, y1: 220.0 };
    for angle in [0, 90, 180, 270] {
      let rect = transform(bounds, &page_matrix(bounds, 1024, angle));
      assert!(rect.x0.abs() < 0.001 && rect.y0.abs() < 0.001);
      let width = if angle % 180 == 0 { 100.0 } else { 200.0 };
      assert!((rect.width() - width).abs() < 0.001);
    }
  }
}

#[cfg(test)]
mod coverage_tests {
  use super::*;

  #[test]
  fn overlapping_rectangles_count_once() {
    let a = Bounds { x0: 0.0, y0: 0.0, x1: 8.0, y1: 8.0 };
    let b = Bounds { x0: 4.0, y0: 4.0, x1: 12.0, y1: 12.0 };
    assert_eq!(union_area(&[]), 0.0);
    assert_eq!(union_area(&[a, a]), 64.0);
    assert_eq!(union_area(&[a, b]), 112.0);
  }
}
