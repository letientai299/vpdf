use crate::Result;
use crate::document::{Document, Handle, Lists, TEXT_FLAGS};
use crate::text::{Bounds, PageId};
use mupdf::text_page::TextBlockType;
use mupdf::{Colorspace, DisplayList, TextPageFlags};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use vpdf_model::geometry::{
  Crop, Crops, Transform, clip, page_matrix, transform,
};

/// Thumbnail width for margin detection, in pixels.
const WIDTH: f32 = 320.0;
/// Gray level below which a pixel counts as ink.
const INK: u8 = 235;
/// Margin kept around the detected content, in thousandths.
const PAD: u16 = 15;

fn stamps(
  list: &DisplayList,
  bounds: Bounds,
  matrix: &Transform,
) -> Result<Vec<Bounds>> {
  let flags = TEXT_FLAGS
    | TextPageFlags::PRESERVE_IMAGES
    | TextPageFlags::COLLECT_VECTORS;
  let raw = list.to_text_page(flags)?;
  let mut content: Vec<Bounds> = raw
    .blocks()
    .filter(|block| {
      matches!(block.r#type(), TextBlockType::Image | TextBlockType::Vector)
    })
    .map(|block| transform(crate::document::rect(block.bounds()), matrix))
    .collect();
  let mut candidates = Vec::new();
  for block in raw.blocks() {
    for line in block.lines() {
      let text: String = line.chars().filter_map(|ch| ch.char()).collect();
      let rect = crate::document::rect(line.bounds());
      if text.trim_start().starts_with("arXiv:")
        && rect.height() > 4.0 * rect.width()
        && (rect.x1 < bounds.x0 + bounds.width() * 0.1
          || rect.x0 > bounds.x1 - bounds.width() * 0.1)
      {
        candidates.push(rect);
      } else {
        content.push(transform(rect, matrix));
      }
    }
  }
  Ok(
    candidates
      .into_iter()
      .map(|stamp| {
        let mut rect = transform(stamp, matrix);
        // Include antialiased edges in the thumbnail.
        rect.x0 = rect.x0.floor() - 1.0;
        rect.y0 = rect.y0.floor() - 1.0;
        rect.x1 = rect.x1.ceil() + 1.0;
        rect.y1 = rect.y1.ceil() + 1.0;
        rect
      })
      .filter(|stamp| {
        !content.iter().any(|bounds| clip(*stamp, *bounds).is_some())
      })
      .collect(),
  )
}

/// Ink box of one page as fractions of its width and height.
fn ink(doc: &mut Document, page: PageId) -> Result<Option<[f32; 4]>> {
  let bounds = doc.pages[page.0].bounds;
  ink_list(doc.display(page)?.as_ref(), bounds)
}

fn ink_list(list: &DisplayList, bounds: Bounds) -> Result<Option<[f32; 4]>> {
  let scale = (WIDTH / bounds.width() * 1024.0) as u32;
  let matrix = page_matrix(bounds, scale, 0);
  let pixmap = list.to_pixmap(
    &crate::raster::matrix(&matrix),
    &Colorspace::device_gray(),
    false,
  )?;
  let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
  let stride = usize::try_from(pixmap.stride())?;
  let samples = pixmap.samples();
  let edge = width.div_ceil(10);
  let peripheral = samples.chunks(stride).take(height).any(|row| {
    row[..edge]
      .iter()
      .chain(&row[width - edge..width])
      .any(|value| *value < INK)
  });
  let stamps =
    if peripheral { stamps(list, bounds, &matrix)? } else { Vec::new() };
  let origin = (pixmap.x() as f32, pixmap.y() as f32);
  let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
  for y in 0..height {
    for (x, value) in samples[y * stride..][..width].iter().enumerate() {
      let (px, py) = (x as f32 + origin.0, y as f32 + origin.1);
      if *value < INK && !stamps.iter().any(|stamp| stamp.contains(px, py)) {
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
      }
    }
  }
  if x1 <= x0 || y1 <= y0 {
    return Ok(None);
  }
  let (w, h) = (width as f32, height as f32);
  Ok(Some([x0 as f32 / w, y0 as f32 / h, x1 as f32 / w, y1 as f32 / h]))
}

fn page_ink(
  handle: &Handle,
  page: PageId,
  lists: Option<&Lists>,
) -> Result<Option<[f32; 4]>> {
  let list = match lists {
    Some(lists) => handle.display(page, lists)?,
    None => {
      Arc::new(handle.native.load_page(page.0 as i32)?.to_display_list(true)?)
    }
  };
  ink_list(&list, handle.bounds(page)?)
}

fn trims(boxes: &[Option<[f32; 4]>]) -> Vec<Crop> {
  calibrated(boxes, boxes)
}

fn calibrated(
  boxes: &[Option<[f32; 4]>],
  reference: &[Option<[f32; 4]>],
) -> Vec<Crop> {
  let mut widths: Vec<_> =
    reference.iter().flatten().map(|b| b[2] - b[0]).collect();
  if widths.is_empty() {
    if boxes.iter().any(Option::is_some) {
      return trims(boxes);
    }
    return vec![Crop::default(); boxes.len()];
  }
  widths.sort_by(f32::total_cmp);
  let width = (widths[widths.len() / 2] + 2.0 / WIDTH).min(1.0);
  let top =
    reference.iter().chain(boxes).flatten().map(|b| b[1]).fold(1.0, f32::min);
  let bottom =
    reference.iter().chain(boxes).flatten().map(|b| b[3]).fold(0.0, f32::max);
  let trim = |fraction: f32| ((fraction * 1000.0) as u16).saturating_sub(PAD);
  boxes
    .iter()
    .map(|bounds| {
      let (center, span) = bounds.map_or((0.5, width), |b| {
        (f32::midpoint(b[0], b[2]), (b[2] - b[0]).max(width))
      });
      let left = (center - span / 2.0).clamp(0.0, (1.0 - span).max(0.0));
      Crop([trim(left), trim(top), trim(1.0 - left - span), trim(1.0 - bottom)])
    })
    .collect()
}

pub fn margins(doc: &mut Document) -> Result<Vec<Crop>> {
  detect(doc, &AtomicBool::new(false))
}

fn detect(doc: &mut Document, stop: &AtomicBool) -> Result<Vec<Crop>> {
  let mut boxes = Vec::with_capacity(doc.pages.len());
  for page in 0..doc.pages.len() {
    anyhow::ensure!(
      crate::scheduling::yield_to_views(|| stop.load(Ordering::Relaxed)),
      "Cropping canceled"
    );
    boxes.push(ink(doc, PageId(page))?);
  }
  Ok(trims(&boxes))
}

/// Sparse results and fixed calibration.
#[derive(Clone, Default)]
pub struct Margins {
  pages: Arc<Crops>,
  calibration: Vec<Option<[f32; 4]>>,
}

impl Margins {
  pub fn resolved(&self) -> Arc<Crops> {
    Arc::clone(&self.pages)
  }

  pub fn len(&self) -> usize {
    self.pages.len()
  }
  pub fn is_empty(&self) -> bool {
    self.pages.is_empty()
  }
  pub fn get(&self, page: PageId) -> Crop {
    self.pages.get(page)
  }
}

impl Margins {
  pub fn contains(&self, page: PageId) -> bool {
    self.pages.contains(page)
  }

  pub fn update(
    &mut self,
    handle: &Handle,
    pages: &[PageId],
    nearby: &[PageId],
    lists: &Lists,
    stop: &AtomicBool,
  ) -> Result<()> {
    let mut boxes = Vec::new();
    for page in pages {
      anyhow::ensure!(
        crate::scheduling::yield_to_views(|| stop.load(Ordering::Relaxed)),
        "Cropping canceled"
      );
      boxes.push(page_ink(
        handle,
        *page,
        nearby.contains(page).then_some(lists),
      )?);
    }
    if self.calibration.is_empty() {
      self.calibration.clone_from(&boxes);
    }
    for (page, bounds) in pages.iter().zip(boxes) {
      let crop = calibrated(&[bounds], &self.calibration)[0];
      Arc::make_mut(&mut self.pages).insert(*page, crop);
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn page_centers_keep_width() {
    let boxes = [
      Some([0.21, 0.2, 0.86, 0.9]),
      Some([0.14, 0.1, 0.79, 0.9]),
      Some([0.21, 0.3, 0.86, 0.9]),
      Some([0.45, 0.4, 0.55, 0.6]),
      Some([0.02, 0.1, 0.98, 0.9]),
      None,
    ];
    let crops = trims(&boxes);
    let page = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
    for (index, bounds) in boxes.iter().enumerate() {
      let shown = crops[index].apply(page);
      if let Some([left, top, right, bottom]) = bounds {
        assert!(shown.contains(*left, *top) && shown.contains(*right, *bottom));
        assert!(
          (shown.centre().0 - f32::midpoint(*left, *right)).abs() < 0.002
        );
      }
      if index != 4 {
        assert!((shown.width() - (0.68 + 2.0 / WIDTH)).abs() < 0.003);
      }
    }
    assert_eq!(trims(&[None, None]), vec![Crop::default(); 2]);
  }
}
