use crate::text::Bounds;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Screen {
  pub columns: u16,
  pub rows: u16,
  pub cell_width: u16,
  pub cell_height: u16,
  pub origin: (u16, u16),
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
