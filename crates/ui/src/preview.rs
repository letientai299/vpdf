mod view;
pub use view::{Event, Frame, Origin, Press, Preview};

use crate::Result;
use crate::layout::Layout;
use crate::source::Raster;
use crate::text::PageId;
use vpdf_model::tile::TileKey;

pub fn on_anchor(
  anchor: Option<(PageId, usize, crate::text::Bounds)>,
  frame: crate::paint::Frame,
  mouse: vpdf_model::input::MouseEvent,
  hit: impl FnOnce(u16, u16) -> crate::pointer::Hit,
) -> bool {
  let Some((page, index, _)) = anchor else { return false };
  let column = mouse.column.wrapping_sub(frame.at.column);
  let row = mouse.row.wrapping_sub(frame.at.row);
  if column < frame.left || row + 1 >= frame.screen.rows {
    return false;
  }
  hit(column, row) == crate::pointer::Hit::Note { page, index, text: false }
}

pub fn image(
  layout: &Layout,
  key: TileKey,
  source: &mut impl Raster,
) -> Result<vpdf_model::image::Rgb> {
  let stride = usize::try_from(key.width)? * 4;
  let mut rgba = vec![0; stride * usize::try_from(key.height)?];
  let first = layout.pages()[key.page.0].top as i32;
  let end = key.y + key.height;
  for index in key.page.0..layout.pages().len() {
    let page = PageId(index);
    let placed = &layout.pages()[index];
    // Page rows in strip coordinates.
    let start = placed.top as i32 - first;
    let (from, to) = (key.y.max(start), end.min(start + placed.height as i32));
    if from >= end {
      break;
    }
    let pan = layout.shown_pan(page) as i32;
    // Wider fitting pages show their middle.
    let wide = placed.width as i32 - key.width;
    let left = match pan {
      _ if index == key.page.0 => key.x,
      0 => (wide / 2).max(0),
      pan => pan,
    };
    let width = key.width.min(placed.width as i32 - left);
    if from >= to || width <= 0 {
      continue;
    }
    let part = TileKey {
      page,
      crop: layout.crop_for(page),
      scale: placed.scale,
      x: left,
      y: from - start,
      width,
      height: to - from,
      ..key
    };
    let row = usize::try_from(width)? * 3;
    let offset = usize::try_from(from - key.y)?;
    // Fitting pages centre, as in the view.
    let indent = if pan == 0 { (key.width - width) / 2 } else { 0 };
    let indent = usize::try_from(indent)? * 4;
    let result = source.raster(part, |pixels, source_stride| {
      let rows = usize::try_from(part.height)?;
      anyhow::ensure!(
        source_stride >= row
          && source_stride.checked_mul(rows) == Some(pixels.len()),
        "Invalid preview raster rows"
      );
      for (line, pixels) in pixels.chunks(source_stride).enumerate() {
        let at = (offset + line) * stride + indent;
        let target = &mut rgba[at..at + row / 3 * 4];
        for (source, target) in pixels[..row]
          .as_chunks::<3>()
          .0
          .iter()
          .zip(target.as_chunks_mut::<4>().0)
        {
          target[..3].copy_from_slice(source);
          target[3] = 255;
        }
      }
      Ok(())
    });
    if index == key.page.0 {
      result?;
    }
  }
  key.recolor.apply_rgba(&mut rgba);
  vpdf_model::image::Rgb::rgba(key.width, key.height, rgba)
    .ok_or_else(|| anyhow::anyhow!("Invalid preview pixels"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::Bounds;

  #[derive(Default)]
  struct Source {
    failed: Option<PageId>,
    malformed: bool,
    requested: Vec<TileKey>,
  }

  impl Raster for Source {
    fn raster(
      &mut self,
      key: TileKey,
      visit: impl FnOnce(&[u8], usize) -> Result<()>,
    ) -> Result<()> {
      self.requested.push(key);
      anyhow::ensure!(self.failed != Some(key.page), "Raster unavailable");
      let stride = key.width as usize * 3 + 4;
      let rows = key.height as usize - usize::from(self.malformed);
      visit(&vec![key.page.0 as u8 + 1; stride * rows], stride)
    }
  }

  fn strip() -> (Layout, TileKey) {
    let layout = Layout::new(
      [100.0, 1.0].map(|size| Bounds { x0: 0.0, y0: 0.0, x1: size, y1: size }),
      crate::testing::screen::pixels(10, 10, 10, 10),
    );
    let key = TileKey {
      scale: 1024,
      y: 95,
      width: 120,
      height: 36,
      ..TileKey::default()
    };
    (layout, key)
  }

  #[test]
  fn composes_centered_rows_and_gaps() -> Result<()> {
    let (layout, key) = strip();
    let mut source = Source::default();
    let rgb = image(&layout, key, &mut source)?;
    assert_eq!(source.requested.len(), 2);
    assert_eq!((source.requested[0].y, source.requested[0].height), (95, 5));
    assert_eq!((source.requested[1].y, source.requested[1].height), (0, 16));
    for (y, row) in rgb.pixels().chunks(480).enumerate() {
      for (x, pixel) in row.chunks(4).enumerate() {
        let expected = if y < 5 && (10..110).contains(&x) {
          [1, 1, 1, 255]
        } else if (15..31).contains(&y) && (52..68).contains(&x) {
          [2, 2, 2, 255]
        } else {
          [0; 4]
        };
        assert_eq!(pixel, expected, "{x}, {y}");
      }
    }
    Ok(())
  }

  #[test]
  fn recolor_preserves_transparent_gaps() -> Result<()> {
    let (layout, key) = strip();
    let mut source = Source::default();
    let original = image(&layout, key, &mut source)?;
    let recolor = vpdf_model::recolor::Recolor::parse("dark 100").unwrap();
    let key = TileKey { recolor, ..key };
    let recolored = image(&layout, key, &mut source)?;
    for (before, after) in original
      .pixels()
      .as_chunks::<4>()
      .0
      .iter()
      .zip(recolored.pixels().as_chunks::<4>().0)
    {
      assert_eq!(before[3], after[3]);
      if before[3] != 0 {
        assert_eq!(
          *after,
          [255 - before[0], 255 - before[1], 255 - before[2], 255]
        );
      }
    }
    Ok(())
  }

  #[test]
  fn preserves_gaps_after_later_failure() -> Result<()> {
    let (layout, key) = strip();
    let mut source = Source { failed: Some(PageId(1)), ..Source::default() };
    let rgb = image(&layout, key, &mut source)?;
    assert!(rgb.pixels()[5 * 480..].iter().all(|&byte| byte == 0));
    source.failed = Some(PageId(0));
    assert!(image(&layout, key, &mut source).is_err());
    Ok(())
  }

  #[test]
  fn rejects_incomplete_raster_rows() {
    let (layout, key) = strip();
    let mut source = Source { malformed: true, ..Source::default() };
    assert!(image(&layout, key, &mut source).is_err());
  }
}
