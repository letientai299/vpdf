use super::{Bounds, DisplayList, INK, Result, Transform, clip, transform};
use crate::document::TEXT_FLAGS;
use mupdf::TextPageFlags;
use mupdf::text_page::TextBlockType;

#[derive(Default)]
pub(super) struct Masks {
  pub stamps: Vec<Bounds>,
  pub decoration: Vec<Bounds>,
  pub protected: Vec<Bounds>,
  pub bands: u8,
}

fn expanded(mut rect: Bounds) -> Bounds {
  rect.x0 = rect.x0.floor() - 1.0;
  rect.y0 = rect.y0.floor() - 1.0;
  rect.x1 = rect.x1.ceil() + 1.0;
  rect.y1 = rect.y1.ceil() + 1.0;
  rect
}

fn band(rect: Bounds, page: Bounds) -> u8 {
  if rect.y1 < page.y0 + page.height() * 0.15 {
    1
  } else if rect.y0 > page.y1 - page.height() * 0.15 {
    2
  } else {
    0
  }
}

fn edge(rect: Bounds, page: Bounds) -> bool {
  rect.x0 <= page.x0 + page.width() * 0.005
    || rect.x1 >= page.x1 - page.width() * 0.005
}

fn adjoins(rule: Bounds, tab: Bounds, page: Bounds) -> bool {
  band(rule, page) == band(tab, page)
    && rule.x0 < tab.x1
    && rule.x1 > tab.x0
    && (rule.y0 - tab.y1).abs().min((tab.y0 - rule.y1).abs())
      <= page.height() * 0.005
}

pub(super) fn read(
  list: &DisplayList,
  page: Bounds,
  matrix: &Transform,
) -> Result<Masks> {
  let flags = TEXT_FLAGS
    | TextPageFlags::PRESERVE_IMAGES
    | TextPageFlags::COLLECT_VECTORS;
  let raw = list.to_text_page(flags)?;
  let mut masks = Masks::default();
  let mut vectors = Vec::new();
  let mut graphics = Vec::new();
  let mut text = Vec::new();
  let mut stamps = Vec::new();
  for block in raw.blocks() {
    let rect = crate::document::rect(block.bounds());
    match block.r#type() {
      TextBlockType::Vector => vectors.push(rect),
      TextBlockType::Image => graphics.push(rect),
      _ => {}
    }
    for line in block.lines() {
      let rect = crate::document::rect(line.bounds());
      if rect.height() > 4.0 * rect.width()
        && (rect.x1 < page.x0 + page.width() * 0.1
          || rect.x0 > page.x1 - page.width() * 0.1)
        && line
          .chars()
          .filter_map(|ch| ch.char())
          .collect::<String>()
          .trim_start()
          .starts_with("arXiv:")
      {
        stamps.push(rect);
      } else {
        text.push(rect);
      }
    }
  }
  let rules: Vec<_> = vectors
    .iter()
    .copied()
    .filter(|rect| {
      band(*rect, page) != 0
        && edge(*rect, page)
        && rect.width() >= page.width() * 0.5
        && rect.height() <= page.height() * 0.005
    })
    .collect();
  let tabs: Vec<_> = vectors
    .iter()
    .copied()
    .filter(|rect| {
      band(*rect, page) != 0
        && edge(*rect, page)
        && rect.height() > page.height() * 0.005
        && rect.height() <= page.height() * 0.05
        && rect.width() >= rect.height() * 4.0
        && rect.width() <= page.width() * 0.5
        && rules.iter().any(|rule| adjoins(*rule, *rect, page))
    })
    .collect();
  let mut candidates = Vec::new();
  if !text.is_empty() {
    candidates.extend(
      rules
        .into_iter()
        .filter(|rule| tabs.iter().any(|tab| adjoins(*rule, *tab, page))),
    );
    candidates.extend(tabs);
  }
  graphics.extend(vectors.iter().filter(|v| !candidates.contains(v)));
  for stamp in stamps {
    let stamp = expanded(transform(stamp, matrix));
    if graphics
      .iter()
      .chain(&vectors)
      .chain(&text)
      .any(|rect| clip(stamp, transform(*rect, matrix)).is_some())
    {
      masks.protected.push(stamp);
    } else {
      masks.stamps.push(stamp);
    }
  }
  masks
    .protected
    .extend(text.iter().chain(&graphics).map(|rect| transform(*rect, matrix)));
  for rect in candidates {
    masks.bands |= band(rect, page);
    masks.decoration.push(expanded(transform(rect, matrix)));
  }
  Ok(masks)
}

impl Masks {
  pub fn hidden(&self, x: f32, y: f32) -> bool {
    self.decoration.iter().any(|rect| rect.contains(x, y))
      && !self.protected.iter().any(|rect| rect.contains(x, y))
  }

  pub fn rectangular(
    &self,
    samples: &[u8],
    stride: usize,
    page: Bounds,
  ) -> bool {
    self.decoration.iter().all(|rect| {
      let Some(rect) = clip(*rect, page) else { return false };
      let x0 = ((rect.x0 + 1.0).ceil() as usize).min(stride);
      let x1 = ((rect.x1 - 1.0).floor() as usize).min(stride);
      let y0 = ((rect.y0 + 1.0).ceil() as usize).min(samples.len() / stride);
      let y1 = ((rect.y1 - 1.0).floor() as usize).min(samples.len() / stride);
      // Subpixel rules have no interior rows.
      if x0 >= x1 || y0 >= y1 {
        return true;
      }
      let mut ink = 0;
      for y in y0..y1 {
        ink += samples[y * stride + x0..y * stride + x1]
          .iter()
          .filter(|value| **value < INK)
          .count();
      }
      ink * 10 >= (x1 - x0) * (y1 - y0) * 8
    })
  }
}
