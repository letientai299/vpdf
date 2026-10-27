//! Keeps a restored window reachable on current displays.

/// A rectangle in screen points: x, y, width, height.
pub type Rect = [f64; 4];

/// Grabbable title-bar strip, in points.
const GRIP: [f64; 2] = [64.0, 24.0];

/// Where `frame` goes when no display shows its title bar: the main
/// `area`, centered and shrunk to fit; None keeps it.
pub fn fit(frame: Rect, areas: &[Rect], main: Rect) -> Option<Rect> {
  let [x, y, width, height] = frame;
  let bar = [x, y + height - GRIP[1], width, GRIP[1]];
  let reachable = areas.iter().any(|area| {
    let [w, h] = overlap(bar, *area);
    w >= GRIP[0].min(width) && h > 0.0
  });
  if reachable {
    return None;
  }
  let [ax, ay, aw, ah] = main;
  let (width, height) = (width.min(aw), height.min(ah));
  Some([ax + (aw - width) / 2.0, ay + (ah - height) / 2.0, width, height])
}

fn overlap(a: Rect, b: Rect) -> [f64; 2] {
  let span = |start: f64, size: f64, other: f64, other_size: f64| {
    ((start + size).min(other + other_size) - start.max(other)).max(0.0)
  };
  [span(a[0], a[2], b[0], b[2]), span(a[1], a[3], b[1], b[3])]
}

#[cfg(test)]
mod tests {
  use super::*;

  const MAIN: Rect = [0.0, 0.0, 1440.0, 875.0];
  const RIGHT: Rect = [1440.0, 0.0, 2560.0, 1415.0];

  #[test]
  fn fits_unreachable_frames() {
    for (frame, areas, expected) in [
      ([100.0, 100.0, 800.0, 600.0], vec![MAIN], None),
      ([1600.0, 200.0, 1000.0, 800.0], vec![MAIN, RIGHT], None),
      // The right display was removed.
      (
        [1600.0, 200.0, 1000.0, 800.0],
        vec![MAIN],
        Some([220.0, 37.5, 1000.0, 800.0]),
      ),
      // Too large for the remaining display.
      ([1500.0, 0.0, 2400.0, 1400.0], vec![MAIN], Some(MAIN)),
      // Entirely below the display.
      (
        [100.0, -700.0, 800.0, 600.0],
        vec![MAIN],
        Some([320.0, 137.5, 800.0, 600.0]),
      ),
      // A sliver of title bar cannot be dragged.
      (
        [1400.0, 100.0, 800.0, 600.0],
        vec![MAIN],
        Some([320.0, 137.5, 800.0, 600.0]),
      ),
    ] {
      assert_eq!(fit(frame, &areas, MAIN), expected, "{frame:?}");
    }
  }
}
