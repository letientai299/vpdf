//! A page printed into the output stream as a Kitty image with Unicode
//! placeholders, so pagers and `fzf --preview` can show it like text.

use crate::Result;
use crate::document::Document;
use crate::text::PageId;
use std::io::Write;

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

/// Fits a page into the requested cells.
pub fn print(
  doc: &mut Document,
  page: PageId,
  cells: (u16, u16),
  out: &mut impl Write,
) -> Result<()> {
  doc.validate(page)?;
  let fit = vpdf_terminal::inline::Fit::new(
    doc.pages[page.0].bounds,
    cells,
    cell_aspect(),
  )?;
  let transport = vpdf_terminal::terminal::Transport::live();
  let id = vpdf_terminal::inline::image_id(std::process::id(), transport.tmux);
  vpdf_pdf::raster::visit_page(
    doc,
    page,
    fit.scale(),
    |width, height, pixels| {
      fit.print(transport, id, (width, height), pixels, out)
    },
  )
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
}
