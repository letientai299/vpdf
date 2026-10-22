use vpdf_model::geometry::clip;
use vpdf_model::text::{Bounds, Quad};

pub(super) fn strips(quad: Quad, viewport: Bounds) -> Vec<Bounds> {
  let Some(bounds) = clip(quad.bounds(), viewport) else {
    return Vec::new();
  };
  let axis_aligned = (0..4).all(|i| {
    let (a, b) = (quad.0[i], quad.0[(i + 1) % 4]);
    (a.0 - b.0).abs() < 0.001 || (a.1 - b.1).abs() < 0.001
  });
  if axis_aligned {
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
