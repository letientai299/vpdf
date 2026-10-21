use crate::Result;
use crate::document::{Document, Handle, Lists};
use crate::text::{Bounds, PageId};
use mupdf::{Colorspace, DisplayList};
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

mod decoration;

#[derive(Clone, Default)]
struct Ink {
  raw: Option<[f32; 4]>,
  clean: Option<[f32; 4]>,
  bands: u8,
}

impl Ink {
  fn bounds(&self, confirmed: u8) -> Option<[f32; 4]> {
    if self.bands != 0 && self.bands & !confirmed == 0 {
      self.clean
    } else {
      self.raw
    }
  }
}

fn confirmed(samples: &[Ink]) -> u8 {
  [1, 2].into_iter().fold(0, |bands, band| {
    if samples.iter().filter(|ink| ink.bands & band != 0).count() >= 2 {
      bands | band
    } else {
      bands
    }
  })
}

/// Ink box of one page as fractions of its width and height.
fn ink(doc: &mut Document, page: PageId) -> Result<Ink> {
  let bounds = doc.pages[page.0].bounds;
  ink_list(doc.display(page)?.as_ref(), bounds)
}

fn ink_list(list: &DisplayList, bounds: Bounds) -> Result<Ink> {
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
  let mut masks = if peripheral {
    decoration::read(list, bounds, &matrix)?
  } else {
    decoration::Masks::default()
  };
  let origin = (pixmap.x() as f32, pixmap.y() as f32);
  let pixels = Bounds { x0: 0.0, y0: 0.0, x1: width as f32, y1: height as f32 };
  // Keep mask coordinates relative to the pixmap.
  let relative = |rect: Bounds| Bounds {
    x0: rect.x0 - origin.0,
    y0: rect.y0 - origin.1,
    x1: rect.x1 - origin.0,
    y1: rect.y1 - origin.1,
  };
  let absolute = |rect: Bounds| Bounds {
    x0: rect.x0 + origin.0,
    y0: rect.y0 + origin.1,
    x1: rect.x1 + origin.0,
    y1: rect.y1 + origin.1,
  };
  masks.decoration.iter_mut().for_each(|rect| *rect = relative(*rect));
  let rectangular = masks.rectangular(samples, stride, pixels);
  masks.decoration.iter_mut().for_each(|rect| *rect = absolute(*rect));
  if !rectangular {
    masks.bands = 0;
    masks.decoration.clear();
  }
  let mut raw = None;
  let mut clean = None;
  for y in 0..height {
    let py = y as f32 + origin.1;
    let row = &samples[y * stride..][..width];
    let stamped = |x| {
      masks.stamps.iter().any(|stamp| stamp.contains(x as f32 + origin.0, py))
    };
    let raw_row = extent(row, py, origin.0, |x| !stamped(x));
    if let Some(rect) = raw_row {
      include(&mut raw, rect);
    }
    if masks.bands == 0 {
      continue;
    }
    let decorated =
      masks.decoration.iter().any(|rect| py >= rect.y0 && py <= rect.y1);
    let clean_row = if decorated {
      extent(row, py, origin.0, |x| {
        !stamped(x) && !masks.hidden(x as f32 + origin.0, py)
      })
    } else {
      raw_row
    };
    if let Some(rect) = clean_row {
      include(&mut clean, rect);
    }
  }
  // White text still protects its authored bounds.
  if masks.bands != 0 {
    for rect in &masks.protected {
      if let Some(rect) = clip(*rect, absolute(pixels)) {
        include(&mut clean, rect);
      }
    }
  }
  let fraction = |rect: Bounds| {
    let rect = relative(rect);
    [
      rect.x0 / width as f32,
      rect.y0 / height as f32,
      rect.x1 / width as f32,
      rect.y1 / height as f32,
    ]
  };
  Ok(Ink {
    raw: raw.map(fraction),
    clean: clean.map(fraction),
    bands: masks.bands,
  })
}

fn extent(
  row: &[u8],
  y: f32,
  origin: f32,
  allowed: impl Fn(usize) -> bool,
) -> Option<Bounds> {
  let first =
    row.iter().enumerate().position(|(x, value)| *value < INK && allowed(x))?;
  let last = first
    + row[first..]
      .iter()
      .enumerate()
      .rposition(|(x, value)| *value < INK && allowed(x + first))?;
  Some(Bounds {
    x0: first as f32 + origin,
    y0: y,
    x1: last as f32 + origin + 1.0,
    y1: y + 1.0,
  })
}

fn include(bounds: &mut Option<Bounds>, rect: Bounds) {
  *bounds = Some(bounds.map_or(rect, |bounds| bounds.union(rect)));
}

fn page_ink(
  handle: &Handle,
  page: PageId,
  lists: Option<&Lists>,
) -> Result<Ink> {
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
  let ordinary: Vec<_> = reference
    .iter()
    .copied()
    .filter(|bounds| {
      bounds.is_some_and(|b| {
        b[0] > 0.01 && b[1] > 0.01 && b[2] < 0.99 && b[3] < 0.99
      })
    })
    .collect();
  let reference = if ordinary.is_empty() { reference } else { &ordinary };
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
  let top = reference.iter().flatten().map(|b| b[1]).fold(1.0, f32::min);
  let bottom = reference.iter().flatten().map(|b| b[3]).fold(0.0, f32::max);
  let trim = |fraction: f32| ((fraction * 1000.0) as u16).saturating_sub(PAD);
  boxes
    .iter()
    .map(|bounds| {
      let (center, span) = bounds.map_or((0.5, width), |b| {
        (f32::midpoint(b[0], b[2]), (b[2] - b[0]).max(width))
      });
      let left = (center - span / 2.0).clamp(0.0, (1.0 - span).max(0.0));
      let (top, bottom) =
        bounds.map_or((top, bottom), |b| (top.min(b[1]), bottom.max(b[3])));
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
  let confirmed = confirmed(&boxes);
  Ok(trims(&boxes.iter().map(|ink| ink.bounds(confirmed)).collect::<Vec<_>>()))
}

/// Sparse results and fixed calibration.
#[derive(Clone, Default)]
pub struct Margins {
  pages: Arc<Crops>,
  calibration: Vec<Option<[f32; 4]>>,
  decoration: u8,
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
      self.decoration = confirmed(&boxes);
      self.calibration =
        boxes.iter().map(|ink| ink.bounds(self.decoration)).collect();
    }
    for (page, bounds) in pages.iter().zip(boxes) {
      let crop =
        calibrated(&[bounds.bounds(self.decoration)], &self.calibration)[0];
      Arc::make_mut(&mut self.pages).insert(*page, crop);
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests;
