//! A page printed into the output stream as a Kitty image with Unicode
//! placeholders, so pagers and `fzf --preview` can show it like text.

use crate::Result;
use crate::diacritics::DIACRITICS;
use crate::document::Document;
use crate::layout::page_matrix;
use crate::terminal::transmit;
use crate::text::PageId;
use crate::tiles::encode;
use mupdf::Colorspace;
use std::io::Write;

/// Cell height over width when the terminal does not say. Kitty fits
/// the image into its cells with the aspect ratio kept, so a wrong
/// value only adds margins.
const CELL_ASPECT: f32 = 2.0;

/// Cell height over width, from the source the reader sizes cells by.
fn cell_aspect() -> f32 {
  crate::terminal::cell()
    .map_or(CELL_ASPECT, |cell| f32::from(cell.height) / f32::from(cell.width))
}

/// Rendered pixels per cell column; Kitty scales to the real cell.
const PIXELS: f32 = 20.0;

/// Fits the page into `columns` x `rows` cells and prints it.
pub fn print(
  doc: &mut Document,
  page: PageId,
  (columns, rows): (u16, u16),
  out: &mut impl Write,
) -> Result<()> {
  doc.validate(page)?;
  let bounds = doc.pages[page.0].bounds;
  let aspect = bounds.height() / bounds.width();
  let cell = cell_aspect();
  let max_columns = DIACRITICS.len() as f32;
  let mut width = f32::from(columns.max(1)).min(max_columns);
  let mut height = (width * aspect / cell).ceil();
  if height > f32::from(rows.max(1)) {
    height = f32::from(rows.max(1)).min(max_columns);
    width = (height * cell / aspect).floor().max(1.0);
  }
  let scale = width * PIXELS / bounds.width();
  let pixmap = doc.display(page)?.to_pixmap(
    &page_matrix(bounds, (scale * 1024.0) as u32, 0),
    &Colorspace::device_rgb(),
    false,
  )?;
  let id = image_id();
  let (pixels_wide, pixels_high) = (pixmap.width(), pixmap.height());
  let encoded = encode(pixmap.samples())?;
  let control = format!(
    "a=T,U=1,q=2,f=24,o=z,i={id},s={pixels_wide},v={pixels_high},c={},r={}",
    width as u32, height as u32
  );
  transmit(out, &control, &encoded)?;
  let [_, r, g, b] = id.to_be_bytes();
  for (index, row) in DIACRITICS[..height as usize].iter().enumerate() {
    if index > 0 {
      writeln!(out)?;
    }
    write!(out, "\x1b[38;2;{r};{g};{b}m")?;
    for column in &DIACRITICS[..width as usize] {
      write!(out, "\u{10EEEE}{row}{column}")?;
    }
    write!(out, "\x1b[39m")?;
  }
  Ok(())
}

/// A 24-bit id carried in the placeholder foreground color.
fn image_id() -> u32 {
  let mixed = std::process::id().wrapping_mul(2_654_435_761);
  (mixed & 0x00FF_FFFF).max(1)
}
