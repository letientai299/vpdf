use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Validated, shared RGB pixels.
///
/// ```compile_fail
/// use vpdf_model::image::Rgb;
/// let image = Rgb { width: 1, height: 1, pixels: Vec::new() };
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rgb {
  width: i32,
  height: i32,
  pixels: Arc<Vec<u8>>,
}

impl Rgb {
  pub fn new(
    width: i32,
    height: i32,
    pixels: impl Into<Arc<Vec<u8>>>,
  ) -> Option<Self> {
    if width <= 0 || height <= 0 {
      return None;
    }
    let length = usize::try_from(width)
      .ok()?
      .checked_mul(usize::try_from(height).ok()?)?
      .checked_mul(3)?;
    let pixels = pixels.into();
    (pixels.len() == length).then_some(Self { width, height, pixels })
  }

  pub fn width(&self) -> i32 {
    self.width
  }
  pub fn height(&self) -> i32 {
    self.height
  }
  pub fn pixels(&self) -> &Arc<Vec<u8>> {
    &self.pixels
  }
}

/// Picture identities require explicit creation.
///
/// ```compile_fail
/// let id: vpdf_model::image::PictureId = 1;
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PictureId(u64);

impl PictureId {
  pub fn fresh() -> Self {
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    Self(SERIAL.fetch_add(1, Ordering::Relaxed))
  }
  pub fn get(self) -> u64 {
    self.0
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn validates_shape_and_shares_pixels() {
    for (width, height, bytes) in
      [(0, 1, 0), (-1, 1, 3), (1, 1, 2), (1, 1, 4), (i32::MAX, i32::MAX, 0)]
    {
      assert!(Rgb::new(width, height, vec![0; bytes]).is_none());
    }
    let pixels = vec![255; 6];
    let original = pixels.as_ptr();
    let image = Rgb::new(2, 1, pixels).unwrap();
    assert_eq!(image.pixels().as_ref().as_ptr(), original);
    let copy = image.clone();
    assert!(Arc::ptr_eq(image.pixels(), copy.pixels()));
    assert_eq!((copy.width(), copy.height()), (2, 1));
    assert_ne!(PictureId::fresh(), PictureId::fresh());
  }
}
