use crate::Result;
use crate::app::App;
use crate::document::{Document, Handle, Lists, TEXT_FLAGS};
use crate::layout::{Crop, clip, page_matrix, transform};
use crate::text::{Bounds, PageId};
use mupdf::text_page::TextBlockType;
use mupdf::{Colorspace, DisplayList, Matrix, TextPageFlags};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// Thumbnail width for margin detection, in pixels.
const WIDTH: f32 = 320.0;
/// Gray level below which a pixel counts as ink.
const INK: u8 = 235;
/// Margin kept around the detected content, in thousandths.
const PAD: u16 = 15;

fn stamps(
  list: &DisplayList,
  bounds: Bounds,
  matrix: &Matrix,
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
    .map(|block| transform(block.bounds().into(), matrix))
    .collect();
  let mut candidates = Vec::new();
  for block in raw.blocks() {
    for line in block.lines() {
      let text: String = line.chars().filter_map(|ch| ch.char()).collect();
      let rect = Bounds::from(line.bounds());
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
  let pixmap = list.to_pixmap(&matrix, &Colorspace::device_gray(), false)?;
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
      crate::render_worker::yield_to_views(|| stop.load(Ordering::Relaxed)),
      "Cropping canceled"
    );
    boxes.push(ink(doc, PageId(page))?);
  }
  Ok(trims(&boxes))
}

/// Sparse results and fixed calibration.
#[derive(Clone, Default)]
pub struct Margins {
  pages: BTreeMap<PageId, Crop>,
  calibration: Vec<Option<[f32; 4]>>,
}

impl Margins {
  pub fn len(&self) -> usize {
    self.pages.len()
  }
  pub fn is_empty(&self) -> bool {
    self.pages.is_empty()
  }
  pub fn get(&self, page: PageId) -> Crop {
    self.pages.get(&page).copied().unwrap_or_default()
  }
}

const SAMPLES: usize = 24;
const BATCH: usize = crate::prefetch::PAGES;

fn samples(count: usize, nearby: impl Iterator<Item = PageId>) -> Vec<PageId> {
  let mut pages: Vec<_> = nearby.take(BATCH).collect();
  if count <= SAMPLES {
    pages.extend((0..count).map(PageId));
  } else {
    for pair in 0..(SAMPLES - BATCH) / 2 {
      let first = pair * count.saturating_sub(2) / ((SAMPLES - BATCH) / 2 - 1);
      pages.extend([PageId(first), PageId((first + 1).min(count - 1))]);
    }
  }
  let mut unique = Vec::new();
  for page in pages {
    if !unique.contains(&page) {
      unique.push(page);
    }
  }
  unique
}

pub struct Job {
  result: mpsc::Receiver<std::result::Result<Arc<Margins>, String>>,
  stop: Arc<AtomicBool>,
  pages: Vec<PageId>,
}

impl Drop for Job {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
  }
}

impl App {
  pub(crate) fn start_crop(&mut self) {
    let pages = samples(self.doc.pages.len(), self.nearby_pages().into_iter());
    self.crop_batch(pages);
    self.layout.crop = Crop([1, 0, 0, 0]);
    self.reader.message = "Detecting margins".into();
  }

  fn crop_batch(&mut self, pages: Vec<PageId>) {
    let (send, result) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let canceled = Arc::clone(&stop);
    let (path, password) = (self.path.clone(), self.password.clone());
    let cached = self.doc.margins.clone();
    let lists = self.doc.lists();
    let wanted = pages.clone();
    let nearby = self.nearby_pages();
    std::thread::spawn(move || {
      let found = (|| -> Result<Arc<Margins>> {
        let handle = Handle::open(&path, &password)?;
        let mut margins = cached.as_deref().cloned().unwrap_or_default();
        let mut boxes = Vec::new();
        for page in &pages {
          anyhow::ensure!(
            crate::render_worker::yield_to_views(
              || canceled.load(Ordering::Relaxed)
            ),
            "Cropping canceled"
          );
          boxes.push(page_ink(
            &handle,
            *page,
            nearby.contains(page).then_some(lists.as_ref()),
          )?);
        }
        if margins.calibration.is_empty() {
          margins.calibration.clone_from(&boxes);
        }
        for (page, bounds) in pages.into_iter().zip(boxes) {
          let crop = calibrated(&[bounds], &margins.calibration)[0];
          margins.pages.insert(page, crop);
        }
        Ok(Arc::new(margins))
      })()
      .map_err(|error| error.to_string());
      let _ = crate::input::notify(send.send(found));
    });
    self.crop_job = Some(Job { result, stop, pages: wanted });
  }

  fn apply_crop(&mut self, crops: Arc<Margins>) -> Result<()> {
    let animation = self.animation.take().and_then(|animation| {
      let page = self.layout.page_at(animation.target);
      let point = self.layout.unproject(
        page,
        (self.layout.screen.width() / 2.0, 0.0),
        animation.target,
      )?;
      Some((animation, page, point))
    });
    self.layout.crop = Crop([1, 0, 0, 0]);
    self.layout.crops = crops;
    self.relayout()?;
    if let Some((mut animation, page, (x, y))) = animation {
      let bounds = Bounds { x0: x, y0: y, x1: x, y1: y };
      let rect = self.layout.project(page, bounds, 0.0);
      let shown = &self.layout.pages[page.0];
      animation.target = self
        .layout
        .clamp_scroll(rect.y0.clamp(shown.top, shown.top + shown.height));
      animation.updated = std::time::Instant::now();
      self.animation = Some(animation);
    }
    self.reader.message = "Margins cropped".into();
    Ok(())
  }

  pub(crate) fn request_crop(&mut self) {
    if self.layout.crop.is_none() {
      return;
    }
    let nearby = self.nearby_pages();
    if let Some(job) = &self.crop_job {
      let target_ready = nearby.first().is_some_and(|page| {
        job.pages.contains(page)
          || self
            .doc
            .margins
            .as_ref()
            .is_some_and(|cached| cached.pages.contains_key(page))
      });
      if target_ready && nearby.iter().any(|page| job.pages.contains(page)) {
        return;
      }
      job.stop.store(true, Ordering::Relaxed);
      return;
    }
    let Some(cached) = &self.doc.margins else {
      self.start_crop();
      return;
    };
    let pages: Vec<_> = self
      .nearby_pages()
      .into_iter()
      .filter(|page| !cached.pages.contains_key(page))
      .collect();
    if !pages.is_empty() {
      self.crop_batch(pages);
    }
  }

  pub fn crop_updates(&mut self) -> Result<bool> {
    self.request_crop();
    let Some(job) = &self.crop_job else { return Ok(false) };
    let result = match job.result.try_recv() {
      Ok(result) => result,
      Err(mpsc::TryRecvError::Empty) => return Ok(false),
      Err(mpsc::TryRecvError::Disconnected) => Err("Cropping stopped".into()),
    };
    let canceled = job.stop.load(Ordering::Relaxed);
    self.crop_job = None;
    if canceled {
      self.request_crop();
      return Ok(true);
    }
    let crops = match result {
      Ok(crops) => crops,
      Err(error) => {
        self.layout.crop = Crop::default();
        self.relayout()?;
        return Err(anyhow::Error::msg(error));
      }
    };
    self.doc.margins = Some(Arc::clone(&crops));
    self.apply_crop(crops)?;
    Ok(true)
  }

  pub(crate) fn cancel_crop(&mut self) -> Result<()> {
    if self.crop_job.take().is_some() {
      self.layout.crop = Crop::default();
      self.relayout()?;
    }
    Ok(())
  }

  /// `zc`: trims white margins, or restores them.
  pub fn toggle_crop(&mut self) -> Result<()> {
    if self.crop_job.take().is_some() || !self.layout.crop.is_none() {
      self.layout.crop = Crop::default();
      self.relayout()?;
      self.reader.message = "Margins shown".into();
    } else if let Some(crops) = &self.doc.margins {
      self.apply_crop(Arc::clone(crops))?;
    } else {
      self.start_crop();
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn sampling_is_bounded() {
    for count in [1, 2, 15, 17, 24, 244, 3329, 100_000] {
      let pages = samples(count, (0..count).map(PageId));
      assert!(pages.len() <= SAMPLES);
      if count <= SAMPLES {
        assert_eq!(pages.len(), count);
      }
      assert!(pages.iter().all(|page| page.0 < count));
      assert!(pages.contains(&PageId(0)));
      assert!(pages.contains(&PageId(count - 1)));
    }
    assert!(Margins::default().get(PageId(100)).is_none());
  }

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
