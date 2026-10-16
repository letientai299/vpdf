use mupdf::{Colorspace, Matrix};
use std::path::PathBuf;
use vpdf::Result;
use vpdf::document::Document;
use vpdf::text::PageId;
use vpdf::tiles::{TileKey, raster};

#[test]
#[ignore = "requires verified public PDF corpus"]
fn tiles_equal_full_page_pixels() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for file in ["paper.pdf", "book.pdf", "form.pdf", "ocr.pdf"] {
    let mut doc = Document::open(&root.join(file), "")?;
    let page = doc.native.load_page(0)?;
    let full = page.to_pixmap(
      &Matrix::IDENTITY,
      &Colorspace::device_rgb(),
      false,
      true,
    )?;
    let key = TileKey {
      page: PageId(0),
      scale: 1024,
      rotation: 0,
      x: 32,
      y: 64,
      width: 256,
      height: 128,
      crop: vpdf::layout::Crop::default(),
    };
    let tile = raster(&mut doc, key)?;
    for row in 0..key.height as usize {
      let start =
        ((key.y as usize + row) * full.width() as usize + key.x as usize) * 3;
      let tile_start = row * key.width as usize * 3;
      assert_eq!(
        &tile[tile_start..tile_start + key.width as usize * 3],
        &full.samples()[start..start + key.width as usize * 3],
        "{file}"
      );
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cropped_margins_keep_all_ink() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for file in ["paper.pdf", "book.pdf"] {
    let mut doc = Document::open(&root.join(file), "")?;
    let crop = vpdf::crop::margins(&mut doc)?;
    assert!(crop.0.iter().all(|trim| *trim < 400), "{file}: {crop:?}");
    assert!(crop.0[0] + crop.0[2] > 100, "{file} side margins: {crop:?}");
    for page in 0..doc.pages.len().min(40) {
      let cropped = crop.apply(doc.pages[page].bounds);
      for line in &doc.text(PageId(page))?.lines {
        for glyph in line.glyphs.iter().filter(|g| !g.ch.is_whitespace()) {
          let (x, y) = (
            (glyph.bounds.x0 + glyph.bounds.x1) / 2.0,
            (glyph.bounds.y0 + glyph.bounds.y1) / 2.0,
          );
          assert!(
            cropped.contains(x, y),
            "{file} p{}: {:?}",
            page + 1,
            glyph.ch
          );
        }
      }
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn inline_page_fits_requested_cells() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut doc = Document::open(&root.join("paper.pdf"), "")?;
  let mut out = Vec::new();
  vpdf::inline::print(&mut doc, PageId(0), (40, 12), &mut out)?;
  let text = String::from_utf8(out)?;
  let rows: Vec<_> = text.split('\n').collect();
  assert!(rows.len() <= 12 && !text.ends_with('\n'));
  let cells = rows[0].chars().filter(|c| *c == '\u{10EEEE}').count();
  assert!(cells > 0 && cells <= 40);
  assert!(text.starts_with("\x1b_Ga=T,U=1"));
  Ok(())
}
