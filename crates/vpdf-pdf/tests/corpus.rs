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
      &vpdf_pdf::raster::matrix(&capture.matrix),
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

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_figure_index() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for (name, captioned) in [("paper.pdf", true), ("ocr.pdf", false)] {
    let doc = Document::open(&root.join(name), "")?;
    let job = vpdf_pdf::jobs::figures::Job::start(
      &root.join(name),
      "",
      doc.pages.len(),
      Vec::new(),
      &vpdf_pdf::jobs::Wake::none(),
    );
    let entries = job
      .result
      .recv_timeout(std::time::Duration::from_secs(20))?
      .map_err(anyhow::Error::msg)?;
    assert!(!entries.is_empty(), "{name}");
    assert!(entries.iter().all(|(_, f)| f.caption.is_empty() != captioned));
  }
  Ok(())
}

#[test]
#[ignore = "requires PMPP PDF in VPDF_FIGURE_PMPP"]
fn pmpp_figure_index() -> Result<()> {
  let path = PathBuf::from(std::env::var("VPDF_FIGURE_PMPP")?);
  let mut doc = Document::open(&path, "")?;
  let job = vpdf_pdf::jobs::figures::Job::start(
    &path,
    "",
    doc.pages.len(),
    Vec::new(),
    &vpdf_pdf::jobs::Wake::none(),
  );
  let entries = job
    .result
    .recv_timeout(std::time::Duration::from_secs(30))?
    .map_err(anyhow::Error::msg)?;
  assert!(!entries.is_empty());
  assert!(entries.iter().all(|(_, f)| !f.caption.is_empty()));
  assert!(entries[0].1.caption.starts_with("FIGURE 1.1 CPUs and GPUs"));
  assert!(entries[0].1.caption.ends_with("throughput-oriented."));
  assert!(entries.iter().any(|(_, figure)| figure.caption
    == "FIGURE 4.1 Architecture of a CUDA-capable GPU."));
  for (index, caption, height) in [
    (49, "FIGURE 2.3", 130.0),
    (54, "FIGURE 2.6", 100.0),
    (55, "FIGURE 2.7", 85.0),
    (382, "FIGURE 16.8", 140.0),
  ] {
    let figures = doc.figures(PageId(index))?;
    assert_eq!(figures.len(), 1, "{figures:?}");
    assert!(figures[0].caption.starts_with(caption));
    assert!(figures[0].bounds.height() > height, "{figures:?}");
  }
  Ok(())
}

#[test]
#[ignore = "requires PMPP PDF in VPDF_FIGURE_PMPP"]
fn pmpp_figure_regions() -> Result<()> {
  let path = PathBuf::from(std::env::var("VPDF_FIGURE_PMPP")?);
  let mut doc = Document::open(&path, "")?;
  for (index, caption, height) in
    [(160, "FIGURE 6.12", 250.0), (161, "FIGURE 6.13", 380.0)]
  {
    let figures = doc.figures(PageId(index))?;
    assert_eq!(figures.len(), 1, "{figures:?}");
    let figure = &figures[0];
    assert!(figure.caption.starts_with(caption));
    assert_eq!(figure.source, vpdf_model::document::FigureSource::Graphic);
    assert!(figure.bounds.y0 > 75.0, "{figure:?}");
    assert!(figure.bounds.y0 < 85.0, "{figure:?}");
    assert!(figure.bounds.height() > height, "{figure:?}");
    assert!(figure.bounds.y1 < 500.0, "{figure:?}");
    let image = capture(&mut doc, PageId(index), figure.bounds, 0)?;
    assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
  }
  Ok(())
}

#[test]
#[ignore = "requires SDM PDF in VPDF_FIGURE_SDM"]
fn sdm_figure_regions() -> Result<()> {
  let path = PathBuf::from(std::env::var("VPDF_FIGURE_SDM")?);
  let mut doc = Document::open(&path, "")?;
  assert!(doc.figures(PageId(22))?.iter().all(|f| f.caption.is_empty()));
  let page = PageId(1149);
  let figures = doc.figures(page)?;
  assert_eq!(figures.len(), 2, "{figures:?}");
  for (figure, caption, top, bottom) in [
    (&figures[0], "Figure 1-16.", 100.0, 360.0),
    (&figures[1], "Figure 1-17.", 432.0, 568.0),
  ] {
    assert!(figure.caption.starts_with(caption), "{figure:?}");
    assert_eq!(figure.source, vpdf_model::document::FigureSource::Graphic);
    assert!(figure.bounds.y0 < top, "{figure:?}");
    assert!(figure.bounds.y0 > top - 2.0, "{figure:?}");
    assert!(figure.bounds.y1 > bottom, "{figure:?}");
    assert!(figure.bounds.y1 < bottom + 2.0, "{figure:?}");
    let image = capture(&mut doc, page, figure.bounds, 0)?;
    assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
  }
  assert!(figures[1].bounds.x0 < 158.0, "SRC labels clipped");
  Ok(())
}

#[test]
#[ignore = "requires PMLA PDF in VPDF_FIGURE_PMLA"]
fn pmla_figure_regions() -> Result<()> {
  let path = PathBuf::from(std::env::var("VPDF_FIGURE_PMLA")?);
  let mut doc = Document::open(&path, "")?;
  for (number, captions) in [
    (45, vec!["Figure 2.1:"]),
    (47, vec!["Figure 2.2:"]),
    (48, vec!["Figure 2.3:"]),
    (50, vec!["Figure 2.4:", "Figure 2.5:"]),
    (82, vec!["Figure 2.15:"]),
    (90, vec!["Figure 2.19:", "Figure 2.20:"]),
    (123, vec!["Figure 3.7:"]),
  ] {
    let figures = doc.figures(PageId(number - 1))?;
    assert_eq!(figures.len(), captions.len(), "page {number}: {figures:?}");
    for (figure, caption) in figures.iter().zip(captions) {
      assert!(figure.caption.starts_with(caption), "{figure:?}");
      assert_eq!(figure.source, vpdf_model::document::FigureSource::Graphic);
      assert!(figure.bounds.y0 > 50.0, "Header included: {figure:?}");
      assert!(figure.bounds.height() > 65.0, "Graphic missing: {figure:?}");
      assert!(figure.bounds.width() > 400.0, "Panel missing: {figure:?}");
    }
    if number == 50 {
      assert!(figures[0].bounds.y1 < 215.0, "Plots merged");
      assert!(figures[1].bounds.y0 > 210.0, "Plots merged");
    }
  }
  let figures = doc.figures(PageId(53))?;
  assert!(figures.iter().all(|f| !f.caption.starts_with("Figure 2.6 shows")));
  let figures = doc.figures(PageId(435))?;
  assert_eq!(figures.len(), 2, "Wrapped references remained: {figures:?}");
  assert!(figures[0].caption.starts_with("Figure 9.1:"));
  assert!(figures[1].caption.starts_with("Figure 9.2:"));
  let figures = doc.figures(PageId(201))?;
  let figure =
    figures.iter().find(|f| f.caption.starts_with("Figure 4.17:")).unwrap();
  assert!(
    figure
      .caption
      .starts_with("Figure 4.17: Samples from an associative Ising model"),
    "{figure:?}"
  );
  assert!(figure.caption.contains("gibbs_demo_ising.ipynb"), "{figure:?}");
  Ok(())
}

#[test]
#[ignore = "requires PMLA PDF in VPDF_FIGURE_PMLA"]
fn pmla_text_diagrams() -> Result<()> {
  let path = PathBuf::from(std::env::var("VPDF_FIGURE_PMLA")?);
  let mut doc = Document::open(&path, "")?;
  for (number, caption, top, bottom) in [
    (83, "Figure 2.16:", 60.0, 106.0),
    (250, "Table 4.6:", 61.0, 168.0),
    (250, "Figure 4.55:", 182.0, 370.0),
    (425, "Figure 8.6:", 60.0, 245.0),
    (440, "Figure 9.4:", 58.0, 200.0),
    (448, "Figure 9.9:", 241.0, 417.0),
  ] {
    let figures = doc.figures(PageId(number - 1))?;
    let figure = figures
      .iter()
      .find(|f| f.caption.starts_with(caption))
      .unwrap_or_else(|| panic!("page {number}: {figures:?}"));
    assert!(figure.bounds.y0 >= top - 5.0, "Header included: {figure:?}");
    assert!(figure.bounds.y0 < top + 5.0, "Diagram clipped: {figure:?}");
    assert!(figure.bounds.y1 < bottom, "Body included: {figure:?}");
    assert!(figure.bounds.y1 > bottom - 35.0, "Diagram clipped: {figure:?}");
  }
  Ok(())
}
