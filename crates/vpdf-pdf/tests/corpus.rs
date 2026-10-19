use std::path::PathBuf;
use vpdf_pdf::{Result, document::Document};

/// Text read from a page's display list, which the reader shares with
/// its tiles, equals text read by interpreting the page.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn display_list_text_matches_page_text() -> Result<()> {
  use vpdf_pdf::document::{extract, extract_list};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for name in ["book.pdf", "paper.pdf", "form.pdf", "ocr.pdf"] {
    let doc = Document::open(&root.join(name), "")?;
    for index in 0..doc.pages.len() as i32 {
      let page = doc.native.load_page(index)?;
      let list = page.to_display_list(true)?;
      assert_eq!(
        format!("{:?}", extract(&page)?),
        format!("{:?}", extract_list(&list)?),
        "{name} page {}",
        index + 1
      );
    }
  }
  Ok(())
}

/// Page bounds read from page dictionaries equal those of loaded pages.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn dictionary_bounds_match_loaded_pages() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for name in ["book.pdf", "paper.pdf", "form.pdf", "ocr.pdf"] {
    let doc = Document::open(&root.join(name), "")?;
    for (index, page) in doc.pages.iter().enumerate() {
      let loaded: Bounds =
        vpdf_pdf::document::rect(doc.native.load_page(index as i32)?.bounds()?);
      assert_eq!(
        (page.bounds.x0, page.bounds.y0, page.bounds.x1, page.bounds.y1),
        (loaded.x0, loaded.y0, loaded.x1, loaded.y1),
        "{name} page {}",
        index + 1
      );
    }
  }
  Ok(())
}

use std::sync::Arc;
use vpdf_model::text::{Bounds, PageId};
use vpdf_pdf::raster::capture;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_figures_and_capture() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut doc = Document::open(&root.join("paper.pdf"), "")?;
  let page = PageId(2);
  let figures = doc.figures(page)?;
  let figure = figures
    .iter()
    .find(|figure| figure.caption.starts_with("Figure 1"))
    .ok_or_else(|| anyhow::anyhow!("Figure 1 missing: {figures:?}"))?;
  assert!(figure.bounds.height() > 100.0, "{figure:?}");
  assert!(Arc::ptr_eq(&figures, &doc.figures(page)?));
  let text = doc.text(page)?;
  doc.seed(page, text);
  let seeded = doc.figures(page)?;
  assert!(!Arc::ptr_eq(&figures, &seeded));
  doc.trim();
  assert!(!Arc::ptr_eq(&seeded, &doc.figures(page)?));
  assert!(doc.figures(PageId(doc.pages.len())).is_err());
  for rotation in [0, 90, 180, 270] {
    let capture = capture(&mut doc, page, figure.bounds, rotation)?;
    assert!(capture.png.starts_with(b"\x89PNG\r\n\x1a\n"));
    let decoded = mupdf::Image::from_bytes(&capture.png)?.to_pixmap()?;
    let list = doc.display(page)?;
    let full = list.to_pixmap(
      &capture.matrix,
      &mupdf::Colorspace::device_rgb(),
      false,
    )?;
    assert_eq!(decoded.width(), (capture.area.x1 - capture.area.x0) as u32);
    assert_eq!(decoded.height(), (capture.area.y1 - capture.area.y0) as u32);
    for row in 0..decoded.height() as usize {
      let offset = (capture.area.y0 - full.y()) as usize + row;
      let left = (capture.area.x0 - full.x()) as usize;
      let first = offset * full.stride() as usize + left * 3;
      let width = decoded.width() as usize * 3;
      let actual = row * decoded.stride() as usize;
      assert_eq!(
        &decoded.samples()[actual..actual + width],
        &full.samples()[first..first + width]
      );
    }
    assert!(
      u64::from(decoded.width()) * u64::from(decoded.height()) <= 8_000_000
    );
  }
  for region in [
    Bounds::default(),
    Bounds { x0: f32::NAN, x1: 100.0, y1: 100.0, ..Bounds::default() },
  ] {
    assert!(capture(&mut doc, page, region, 0).is_err());
  }
  let mut scan = Document::open(&root.join("ocr.pdf"), "")?;
  assert!(!scan.figures(PageId(0))?.is_empty());
  Ok(())
}
