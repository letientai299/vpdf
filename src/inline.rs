//! A page printed into the output stream as a Kitty image with Unicode
//! placeholders, so pagers and `fzf --preview` can show it like text.

use crate::Result;
use crate::diacritics::DIACRITICS;
use crate::document::Document;
use crate::text::PageId;
use crate::tiles::encode;
use mupdf::Colorspace;
use std::io::Write;
use vpdf_terminal::terminal::transmit;
use vpdf_ui::layout::page_matrix;

/// Asks the executable to print a page preview, and names the output
/// the reader chose, since its fzf owns the terminal and the preview
/// cannot query it.
pub const VAR: &str = "VPDF_PREVIEW";

/// How the picker's preview shows a page.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Graphics {
  /// An image with Unicode placeholders.
  Kitty,
  /// Extracted text.
  Text,
}

impl Graphics {
  /// What the reader's probed terminal draws.
  pub fn probed() -> Self {
    let terminal =
      vpdf_terminal::caps::get().and_then(vpdf_terminal::caps::Caps::terminal);
    if terminal.is_some_and(vpdf_terminal::caps::placeholders) {
      Self::Kitty
    } else {
      Self::Text
    }
  }

  pub fn name(self) -> &'static str {
    match self {
      Self::Kitty => "kitty",
      Self::Text => "text",
    }
  }

  /// The output `VAR` names; `None` outside a preview.
  pub fn exported() -> Option<Self> {
    Self::parse(std::env::var_os(VAR)?.to_str()?)
  }

  fn parse(name: &str) -> Option<Self> {
    match name {
      "kitty" => Some(Self::Kitty),
      "text" => Some(Self::Text),
      _ => None,
    }
  }
}

/// Cell height over width when the terminal does not say. Kitty fits
/// the image into its cells with the aspect ratio kept, so a wrong
/// value only adds margins.
const CELL_ASPECT: f32 = 2.0;

/// Cell height over width, from the source the reader sizes cells by.
fn cell_aspect() -> f32 {
  vpdf_terminal::terminal::cell()
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
    &vpdf_pdf::raster::matrix(&page_matrix(bounds, (scale * 1024.0) as u32, 0)),
    &Colorspace::device_rgb(),
    false,
  )?;
  let tmux = vpdf_terminal::terminal::tmux();
  let id = image_id(tmux);
  let (pixels_wide, pixels_high) = (pixmap.width(), pixmap.height());
  let encoded = encode(pixmap.samples())?;
  let control = format!(
    "a=T,U=1,q=2,f=24,o=z,i={id},s={pixels_wide},v={pixels_high},c={},r={}",
    width as u32, height as u32
  );
  transmit(out, &control, &encoded)?;
  placeholders(out, id, tmux, (width as usize, height as usize))
}

/// The placeholder cells of image `id`, which their foreground color
/// names: 24-bit, or a 256-color index under tmux, which turns a 24-bit
/// color into an index when the outer terminal lacks the RGB feature.
fn placeholders(
  out: &mut impl Write,
  id: u32,
  tmux: bool,
  (columns, rows): (usize, usize),
) -> Result<()> {
  let color = if tmux {
    format!("\x1b[38;5;{id}m")
  } else {
    let [_, r, g, b] = id.to_be_bytes();
    format!("\x1b[38;2;{r};{g};{b}m")
  };
  for (index, row) in DIACRITICS[..rows].iter().enumerate() {
    if index > 0 {
      writeln!(out)?;
    }
    out.write_all(color.as_bytes())?;
    for column in &DIACRITICS[..columns] {
      write!(out, "\u{10EEEE}{row}{column}")?;
    }
    write!(out, "\x1b[39m")?;
  }
  Ok(())
}

/// An id the placeholder color carries: 24 bits, or 1-255 under tmux.
fn image_id(tmux: bool) -> u32 {
  let mixed = std::process::id().wrapping_mul(2_654_435_761);
  if tmux { mixed % 255 + 1 } else { (mixed & 0x00FF_FFFF).max(1) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn preview_takes_only_reader_names() {
    let cases = [
      ("kitty", Some(Graphics::Kitty)),
      ("text", Some(Graphics::Text)),
      ("", None),
      ("0", None),
      ("Kitty", None),
    ];
    for (name, want) in cases {
      assert_eq!(Graphics::parse(name), want, "{name:?}");
    }
  }

  #[test]
  fn placeholder_color_carries_the_id() -> Result<()> {
    // (id, tmux) -> first row's color and cells
    let cases =
      [(0x01_02_03, false, "\x1b[38;2;1;2;3m"), (200, true, "\x1b[38;5;200m")];
    for (id, tmux, color) in cases {
      let mut out = Vec::new();
      placeholders(&mut out, id, tmux, (2, 2))?;
      let text = String::from_utf8(out)?;
      let first = format!("{color}\u{10EEEE}\u{305}\u{305}");
      assert!(text.starts_with(&first), "{text:?}");
      assert_eq!(text.matches(color).count(), 2, "{text:?}");
      assert_eq!(text.matches('\u{10EEEE}').count(), 4);
    }
    assert!((1..=255).contains(&image_id(true)));
    assert!((1..=0xFF_FFFF).contains(&image_id(false)));
    Ok(())
  }
}
