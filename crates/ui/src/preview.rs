mod view;
pub use view::{Event, Frame, Origin, Press, Preview};

use crate::Result;
use crate::layout::Layout;
use crate::source::Raster;
use crate::text::PageId;
use crate::theme;
use vpdf_model::tile::TileKey;

pub fn on_anchor(
  anchor: Option<(PageId, usize, crate::text::Bounds)>,
  frame: crate::paint::Frame,
  mouse: crossterm::event::MouseEvent,
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
) -> Result<Vec<u8>> {
  let stride = usize::try_from(key.width)? * 3;
  let mut rgb = vec![theme::PREVIEW_GAP; stride * usize::try_from(key.height)?];
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
    let indent = usize::try_from(indent)? * 3;
    let result = source.raster(part, |pixels, source_stride| {
      let rows = usize::try_from(part.height)?;
      anyhow::ensure!(
        source_stride >= row
          && source_stride.checked_mul(rows) == Some(pixels.len()),
        "Invalid preview raster rows"
      );
      for (line, pixels) in pixels.chunks(source_stride).enumerate() {
        let at = (offset + line) * stride + indent;
        rgb[at..at + row].copy_from_slice(&pixels[..row]);
      }
      Ok(())
    });
    if index == key.page.0 {
      result?;
    }
  }
  Ok(rgb)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::layout::{Crop, Screen};
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
      Screen {
        columns: 10,
        rows: 10,
        cell_width: 10,
        cell_height: 10,
        origin: (0, 0),
      },
    );
    let key = TileKey {
      page: PageId(0),
      scale: 1024,
      rotation: 0,
      x: 0,
      y: 95,
      width: 120,
      height: 36,
      crop: Crop::default(),
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
    for (y, row) in rgb.chunks(360).enumerate() {
      for (x, pixel) in row.chunks(3).enumerate() {
        let expected = if y < 5 && (10..110).contains(&x) {
          1
        } else if (15..31).contains(&y) && (52..68).contains(&x) {
          2
        } else {
          theme::PREVIEW_GAP
        };
        assert_eq!(pixel, &[expected; 3], "{x}, {y}");
      }
    }
    Ok(())
  }

  #[test]
  fn preserves_gaps_after_later_failure() -> Result<()> {
    let (layout, key) = strip();
    let mut source = Source { failed: Some(PageId(1)), ..Source::default() };
    let rgb = image(&layout, key, &mut source)?;
    assert!(rgb[5 * 360..].iter().all(|&byte| byte == theme::PREVIEW_GAP));
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
