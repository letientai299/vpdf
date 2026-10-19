use crate::Result;
use crate::document::Document;
use crate::text::{Bounds, PageId};
use anyhow::Context;
use mupdf::{
  Colorspace, Device, DisplayList, IRect, ImageFormat, Matrix, Pixmap, Rect,
};
use vpdf_model::geometry::{Transform, clip, page_matrix, transform};
use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH, TileKey};

const CAPTURE_PIXELS: u64 = 8_000_000;
const CAPTURE_SCALE: u32 = 2048;

pub fn raster(doc: &mut Document, key: TileKey) -> Result<Vec<u8>> {
  anyhow::ensure!(
    key.width > 0
      && key.width <= TILE_WIDTH
      && key.height > 0
      && key.height <= TILE_HEIGHT,
    "Invalid tile dimensions"
  );
  let area = IRect::new(key.x, key.y, key.x + key.width, key.y + key.height);
  let pixels = raster_area(doc, &key, area)?;
  Ok(pixels.samples().to_vec())
}

/// RGB pixels of `area`, in the page space `key` describes.
pub fn raster_area(
  doc: &mut Document,
  key: &TileKey,
  area: IRect,
) -> Result<Pixmap> {
  doc.validate(key.page)?;
  let bounds = key.crop.apply(doc.pages[key.page.0].bounds);
  let list = doc.display(key.page)?;
  draw(&list, &page_matrix(bounds, key.scale, key.rotation), area)
}

/// RGB pixels of `area` with `list` drawn through `matrix`.
pub fn draw(
  list: &DisplayList,
  matrix: &Transform,
  area: IRect,
) -> Result<Pixmap> {
  let mut pixels =
    Pixmap::new_with_rect(&Colorspace::device_rgb(), area, false)?;
  pixels.clear_with(255)?;
  let device = Device::from_pixmap(&pixels)?;
  list.run(
    &device,
    &self::matrix(matrix),
    Rect::new(area.x0 as f32, area.y0 as f32, area.x1 as f32, area.y1 as f32),
  )?;
  drop(device);
  Ok(pixels)
}

/// Packed RGB rows of the tile `key` cut from `band`.
pub fn cut(band: &Pixmap, key: &TileKey) -> Result<Vec<u8>> {
  let stride = usize::try_from(band.stride())?;
  let left = usize::try_from(key.x - band.x())? * 3;
  let top = usize::try_from(key.y - band.y())?;
  let width = usize::try_from(key.width)? * 3;
  let height = usize::try_from(key.height)?;
  let mut pixels = Vec::with_capacity(width * height);
  for row in band.samples().chunks(stride).skip(top).take(height) {
    let row = row.get(left..left + width).context("Tile outside band")?;
    pixels.extend_from_slice(row);
  }
  anyhow::ensure!(pixels.len() == width * height, "Tile outside band");
  Ok(pixels)
}

pub struct Capture {
  pub png: Vec<u8>,
  pub matrix: Transform,
  pub area: IRect,
}

pub fn capture(
  doc: &mut Document,
  page: PageId,
  region: Bounds,
  rotation: u16,
) -> Result<Capture> {
  doc.validate(page)?;
  let bounds = doc.pages[page.0].bounds;
  let region = clip(region, bounds).context("Empty capture region")?;
  anyhow::ensure!(rotation.is_multiple_of(90), "Invalid capture rotation");
  let (matrix, area) = capture_area(bounds, region, rotation)?;
  let list = doc.display(page)?;
  let pixels = draw(&list, &matrix, area)?;
  let mut png = Vec::new();
  pixels.write_to(&mut png, ImageFormat::PNG)?;
  Ok(Capture { png, matrix, area })
}

fn capture_area(
  page: Bounds,
  region: Bounds,
  rotation: u16,
) -> Result<(Transform, IRect)> {
  let mut low = 1;
  let mut high = CAPTURE_SCALE;
  let mut best = None;
  while low <= high {
    let scale = low + (high - low) / 2;
    let matrix = page_matrix(page, scale, rotation);
    let area = rounded(transform(region, &matrix));
    if let Some(area) = area {
      best = Some((matrix, area));
      low = scale + 1;
    } else {
      high = scale - 1;
    }
  }
  best.context("Capture exceeds pixel limit")
}

fn rounded(bounds: Bounds) -> Option<IRect> {
  let coordinates =
    [bounds.x0.floor(), bounds.y0.floor(), bounds.x1.ceil(), bounds.y1.ceil()];
  if !coordinates.into_iter().all(|v| {
    v.is_finite()
      && f64::from(v) >= f64::from(i32::MIN)
      && f64::from(v) <= f64::from(i32::MAX)
  }) {
    return None;
  }
  let [x0, y0, x1, y1] = coordinates.map(|v| v as i32);
  let width = u64::try_from(i64::from(x1) - i64::from(x0)).ok()?;
  let height = u64::try_from(i64::from(y1) - i64::from(y0)).ok()?;
  (width > 0 && height > 0 && width.checked_mul(height)? <= CAPTURE_PIXELS)
    .then(|| IRect::new(x0, y0, x1, y1))
}

/// Native conversion preserves the shared transform.
pub fn matrix(transform: &Transform) -> Matrix {
  let [xx, xy, yx, yy, tx, ty] = transform.coefficients();
  Matrix::new(xx, xy, yx, yy, tx, ty)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn bounds(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
    Bounds { x0, y0, x1, y1 }
  }

  #[test]
  fn transforms_match_native() {
    for bounds in [
      bounds(0.0, 0.0, 612.0, 792.0),
      bounds(-11.25, 22.5, 123.75, 456.125),
      bounds(53.25, -99.5, 1000.0, 3500.0),
    ] {
      for rotation in [0, 45, 90, 180, 270, 360, 450] {
        for scale in [0, 10, 511, 1024, 2457, 16384] {
          let shared = page_matrix(bounds, scale, rotation);
          let mut native = Matrix::new_rotate(f32::from(rotation));
          native.scale(scale as f32 / 1024.0, scale as f32 / 1024.0);
          let points = [
            (bounds.x0, bounds.y0),
            (bounds.x1, bounds.y0),
            (bounds.x0, bounds.y1),
            (bounds.x1, bounds.y1),
          ];
          let rect =
            Bounds::enclosing(&points.map(|(x, y)| native.transform_xy(x, y)));
          native.e -= rect.x0;
          native.f -= rect.y0;
          assert_eq!(matrix(&shared), native);
          assert_eq!(shared.invert().map(|m| matrix(&m)), native.invert());
        }
      }
    }
  }

  #[test]
  fn captures_bound_rounded_pixels() -> Result<()> {
    let page = bounds(10.0, 20.0, 3010.0, 4020.0);
    for region in
      [page, bounds(30.1, 40.7, 31.0, 41.0), bounds(20.1, 30.2, 2000.7, 3000.3)]
    {
      for rotation in [0, 90, 180, 270] {
        let (matrix, area) = capture_area(page, region, rotation)?;
        let width = u64::try_from(area.x1 - area.x0)?;
        let height = u64::try_from(area.y1 - area.y0)?;
        assert!(width > 0 && height > 0 && width * height <= CAPTURE_PIXELS);
        let rect = transform(region, &matrix);
        assert!(area.x0 as f32 <= rect.x0 && area.y0 as f32 <= rect.y0);
        assert!(area.x1 as f32 >= rect.x1 && area.y1 as f32 >= rect.y1);
      }
    }
    for invalid in [
      bounds(f32::NAN, 0.0, 10.0, 10.0),
      bounds(0.0, 0.0, 0.0, 10.0),
      bounds(7000.0, 0.0, 8000.0, 10.0),
    ] {
      assert!(clip(invalid, page).is_none());
    }
    assert!(rounded(bounds(0.0, 0.0, 4000.1, 2000.0)).is_none());
    Ok(())
  }
}
