use crate::Result;
use crate::app::App;
use crate::document::Document;
use crate::layout::{Crop, page_matrix};
use crate::text::PageId;
use mupdf::Colorspace;

/// Pages sampled for margins; enough to cover odd and even layouts.
const SAMPLES: usize = 24;
/// Thumbnail width for margin detection, in pixels.
const WIDTH: f32 = 160.0;
/// Gray level below which a pixel counts as ink.
const INK: u8 = 235;
/// Margin kept around the detected content, in thousandths.
const PAD: u16 = 15;

/// Ink box of one page as fractions of its width and height.
fn ink(doc: &mut Document, page: PageId) -> Result<Option<[f32; 4]>> {
  let bounds = doc.pages[page.0].bounds;
  let scale = (WIDTH / bounds.width() * 1024.0) as u32;
  let list = doc.display(page)?;
  let pixmap = list.to_pixmap(
    &page_matrix(bounds, scale, 0),
    &Colorspace::device_gray(),
    false,
  )?;
  let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
  let stride = usize::try_from(pixmap.stride())?;
  let samples = pixmap.samples();
  let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
  for y in 0..height {
    for (x, value) in samples[y * stride..][..width].iter().enumerate() {
      if *value < INK {
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

/// One trim for the whole document, so every page keeps the same scale.
pub fn margins(doc: &mut Document) -> Result<Crop> {
  let count = doc.pages.len();
  let step = count.div_ceil(SAMPLES).max(1);
  let mut union: Option<[f32; 4]> = None;
  for page in (0..count).step_by(step).map(PageId) {
    let Some([x0, y0, x1, y1]) = ink(doc, page)? else { continue };
    union = Some(union.map_or([x0, y0, x1, y1], |[a, b, c, d]| {
      [a.min(x0), b.min(y0), c.max(x1), d.max(y1)]
    }));
  }
  let Some([x0, y0, x1, y1]) = union else { return Ok(Crop::default()) };
  let trim = |fraction: f32| ((fraction * 1000.0) as u16).saturating_sub(PAD);
  Ok(Crop([trim(x0), trim(y0), trim(1.0 - x1), trim(1.0 - y1)]))
}

impl App {
  /// `zc`: trims white margins, or restores them.
  pub fn toggle_crop(&mut self) -> Result<()> {
    self.layout.crop = if self.layout.crop.is_none() {
      let crop = match self.doc.margins {
        Some(crop) => crop,
        None => margins(&mut self.doc)?,
      };
      self.doc.margins = Some(crop);
      crop
    } else {
      Crop::default()
    };
    self.layout.pan = 0.0;
    self.relayout()?;
    self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
    self.ensure_cursor()?;
    self.reader.message = if self.layout.crop.is_none() {
      "Margins shown"
    } else {
      "Margins cropped"
    }
    .into();
    Ok(())
  }
}
