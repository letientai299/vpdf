use super::{Fill, Layer};
use crate::geometry::{Screen, clip};
use crate::text::{Bounds, Quad};

/// Finite quad with clipped painting metadata.
///
/// ```compile_fail
/// let polygon = vpdf_model::scene::Polygon { corners: [(0, 0); 4] };
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Polygon {
  corners: [(u32, u32); 4],
  fill: Fill,
  clip: [u32; 4],
}

impl Polygon {
  pub fn new(
    quad: Quad,
    viewport: Bounds,
    color: [u8; 4],
    layer: Layer,
    screen: Screen,
    left: u16,
  ) -> Option<Self> {
    if !quad.0.iter().all(|&(x, y)| x.is_finite() && y.is_finite()) {
      return None;
    }
    let turns = std::array::from_fn::<_, 4, _>(|i| {
      let [a, b, c] = [quad.0[i], quad.0[(i + 1) % 4], quad.0[(i + 2) % 4]];
      (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0)
    });
    if !turns.iter().all(|&t| t.is_finite() && t.abs() > f32::EPSILON)
      || turns
        .iter()
        .any(|t| t.is_sign_positive() != turns[0].is_sign_positive())
    {
      return None;
    }
    let viewport = clip(
      viewport,
      Bounds { x0: 0.0, y0: 0.0, x1: screen.width(), y1: screen.height() },
    )?;
    let bounds = clip(quad.bounds(), viewport)?;
    Some(Self {
      corners: quad.0.map(|(x, y)| (x.to_bits(), y.to_bits())),
      fill: Fill::new(bounds, color, layer, screen, left)?,
      clip: [viewport.x0, viewport.y0, viewport.x1, viewport.y1]
        .map(f32::to_bits),
    })
  }

  pub fn quad(self) -> Quad {
    Quad(self.corners.map(|(x, y)| (f32::from_bits(x), f32::from_bits(y))))
  }

  pub fn fill(self) -> Fill {
    self.fill
  }

  pub fn clip(self) -> Bounds {
    let [x0, y0, x1, y1] = self.clip.map(f32::from_bits);
    Bounds { x0, y0, x1, y1 }
  }

  pub fn strips(self) -> Vec<Bounds> {
    strips(self.quad(), self.clip())
  }
}

fn strips(quad: Quad, viewport: Bounds) -> Vec<Bounds> {
  let Some(bounds) = clip(quad.bounds(), viewport) else {
    return Vec::new();
  };
  if quad.is_rect() {
    return vec![bounds];
  }
  let mut strips: Vec<Bounds> = Vec::new();
  for row in bounds.y0.floor() as i32..bounds.y1.ceil() as i32 {
    let y0 = (row as f32).max(bounds.y0);
    let y1 = (row as f32 + 1.0).min(bounds.y1);
    let y = f32::midpoint(y0, y1);
    let mut x0 = f32::INFINITY;
    let mut x1 = f32::NEG_INFINITY;
    for i in 0..4 {
      let (a, b) = (quad.0[i], quad.0[(i + 1) % 4]);
      if y < a.1.min(b.1) || y >= a.1.max(b.1) {
        continue;
      }
      let x = a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1);
      x0 = x0.min(x);
      x1 = x1.max(x);
    }
    let rect = Bounds {
      x0: x0.floor().max(bounds.x0),
      y0,
      x1: x1.ceil().min(bounds.x1),
      y1,
    };
    if rect.width() <= 0.0 {
      continue;
    }
    if let Some(last) = strips.last_mut()
      && last.x0.to_bits() == rect.x0.to_bits()
      && last.x1.to_bits() == rect.x1.to_bits()
      && last.y1.to_bits() == rect.y0.to_bits()
    {
      last.y1 = rect.y1;
    } else {
      strips.push(rect);
    }
  }
  strips
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn rotated_strips_follow_edges() {
    let quad = Quad([(10.0, 0.0), (30.0, 20.0), (20.0, 30.0), (0.0, 10.0)]);
    let viewport = Bounds { x0: 0.0, y0: 0.0, x1: 30.0, y1: 30.0 };
    let strips = strips(quad, viewport);
    assert!(strips.len() > 1);
    assert!(!strips.iter().any(|r| r.contains(1.0, 1.0)));
    let area: f32 = strips.iter().map(|r| r.width() * r.height()).sum();
    assert!((area - 400.0).abs() <= 30.0);
    let viewport = Bounds { x0: 5.0, y0: 5.0, x1: 25.0, y1: 25.0 };
    for strip in super::strips(quad, viewport) {
      assert!(strip.x0 >= 5.0 && strip.y0 >= 5.0);
      assert!(strip.x1 <= 25.0 && strip.y1 <= 25.0);
    }
  }

  #[test]
  fn orthogonal_quads_keep_rectangles() {
    let bounds = Bounds { x0: 10.0, y0: 20.0, x1: 30.0, y1: 40.0 };
    let mut quad = Quad::from(bounds);
    for _ in 0..4 {
      assert_eq!(strips(quad, bounds), [bounds]);
      quad.0.rotate_left(1);
    }
  }
}

#[cfg(test)]
mod validation_tests {
  use super::*;

  #[test]
  fn polygons_validate_and_clip() {
    let screen = Screen {
      columns: 10,
      rows: 8,
      cell_width: 10,
      cell_height: 10,
      origin: (2, 3),
    };
    let quad = Quad([(0.0, 20.0), (90.0, 10.0), (90.0, 40.0), (0.0, 50.0)]);
    let viewport = Bounds { x0: 10.25, y0: 5.25, x1: 80.75, y1: 60.5 };
    let polygon =
      Polygon::new(quad, viewport, [1, 2, 3, 4], Layer::AboveText, screen, 2)
        .unwrap();
    assert_eq!(polygon.clip(), viewport);
    assert_eq!(polygon.quad(), quad);
    for b in polygon.strips() {
      assert!(b.x0 >= viewport.x0 && b.x1 <= viewport.x1);
      assert!(b.y0 >= viewport.y0 && b.y1 <= viewport.y1);
    }
    for q in [
      Quad([(f32::NAN, 0.0); 4]),
      Quad([(10.0, 10.0); 4]),
      Quad([quad.0[0], quad.0[2], quad.0[1], quad.0[3]]),
    ] {
      assert!(
        Polygon::new(q, viewport, [1, 2, 3, 4], Layer::AboveText, screen, 2)
          .is_none()
      );
    }
  }
}
