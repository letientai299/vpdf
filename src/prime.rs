//! Large images and their whole decodes.
//!
//! `MuPDF` decodes only the part of an image a clip covers and caches it
//! under that exact rectangle, so every tile over a scan decodes the
//! image again. A lookup that misses its rectangle falls back to a whole
//! decode at the same reduction (`fz_get_pixmap_from_image` in
//! `source/fitz/image.c`), so one whole decode serves every later tile.
//! The image scaler still differs by clip origin, so only a whole-page
//! render gives tiles the pixels of a full-page render.
use crate::Result;
use mupdf::{
  ColorParams, Colorspace, Device, DisplayList, IRect, Image, Matrix,
  NativeDevice, Rect,
};
use std::cell::Cell;
use std::rc::Rc;

/// Images below this many pixels decode quickly in parts.
const LARGE: u64 = 1 << 19;
/// Largest decoded image held for reuse, in bytes.
const DECODED: u64 = 128 << 20;

struct Scan {
  found: Rc<Cell<bool>>,
  decode: bool,
}

impl Scan {
  fn image(&self, image: &Image, ctm: &Matrix) {
    let (w, h) = (u64::from(image.width()), u64::from(image.height()));
    if w * h < LARGE || image.scalable() {
      return;
    }
    self.found.set(true);
    // A full-resolution whole decode is the key a tile looks up last;
    // reduced decodes come only from a draw device.
    if self.decode
      && w * h * u64::from(image.n()) <= DECODED
      && reduction(w, h, ctm) == 0
    {
      drop(image.to_pixmap());
    }
  }
}

/// `MuPDF`'s power-of-two reduction for an image drawn with `ctm`.
fn reduction(w: u64, h: u64, ctm: &Matrix) -> u32 {
  let dw = (ctm.a.hypot(ctm.b) as u64).min(w);
  let dh = (ctm.c.hypot(ctm.d) as u64).min(h);
  let mut factor = 0;
  while factor < 6 && w >> (factor + 1) >= dw + 2 && h >> (factor + 1) >= dh + 2
  {
    factor += 1;
  }
  factor
}

impl NativeDevice for Scan {
  fn fill_image(&mut self, image: &Image, ctm: Matrix, _: f32, _: ColorParams) {
    self.image(image, &ctm);
  }

  fn fill_image_mask(
    &mut self,
    image: &Image,
    ctm: Matrix,
    _: &Colorspace,
    _: &[f32],
    _: f32,
    _: ColorParams,
  ) {
    self.image(image, &ctm);
  }

  fn clip_image_mask(&mut self, image: &Image, ctm: Matrix, _: Rect) {
    self.image(image, &ctm);
  }
}

fn run(
  list: &DisplayList,
  matrix: &Matrix,
  area: IRect,
  decode: bool,
) -> Result<bool> {
  let rect =
    Rect::new(area.x0 as f32, area.y0 as f32, area.x1 as f32, area.y1 as f32);
  let found = Rc::new(Cell::new(false));
  let device = Device::from_native(Scan { found: Rc::clone(&found), decode })?;
  list.run(&device, matrix, rect)?;
  Ok(found.get())
}

/// Whether the page drawn with `matrix` over `area` has a large image.
pub fn large(list: &DisplayList, matrix: &Matrix, area: IRect) -> Result<bool> {
  run(list, matrix, area, false)
}

/// Decodes the page's large full-resolution images whole, so tiles
/// reuse one decode.
pub fn decode(list: &DisplayList, matrix: &Matrix, area: IRect) -> Result<()> {
  run(list, matrix, area, true).map(drop)
}
