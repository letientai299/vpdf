use crate::Result;
use crate::document::Document;
use crate::raster::Capture;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use vpdf_model::text::{Bounds, PageId};

pub fn capture(
  path: &Path,
  password: &str,
  page: PageId,
  bounds: Bounds,
  rotation: u16,
  cancel: &AtomicBool,
) -> Result<Capture> {
  let mut doc = Document::open(path, password)?;
  let capture = crate::raster::capture(&mut doc, page, bounds, rotation)?;
  anyhow::ensure!(
    !cancel.load(Ordering::Relaxed),
    "Region operation cancelled"
  );
  Ok(capture)
}

pub fn recognize(
  png: Vec<u8>,
  language: &str,
  cancel: &AtomicBool,
) -> Result<String> {
  let output = super::ocr::tesseract(png, language, cancel)?;
  let text = super::ocr::parse_tsv(&output, Bounds::default(), 1.0)?;
  Ok(
    text
      .lines
      .iter()
      .map(vpdf_model::text::Line::text)
      .collect::<Vec<_>>()
      .join("\n"),
  )
}
