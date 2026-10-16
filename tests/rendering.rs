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
