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

/// Names the output the reader chose for the `vpdf preview` its fzf
/// runs, since fzf owns the terminal and the preview cannot query it.
pub const VAR: &str = "VPDF_GRAPHICS";

/// How `vpdf preview` shows a page.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Graphics {
  /// An image with Unicode placeholders.
  Kitty,
  /// Extracted text.
  Text,
}

impl Graphics {
  fn of(placeholders: bool) -> Self {
    if placeholders { Self::Kitty } else { Self::Text }
  }

  /// What the reader's probed terminal draws.
  pub fn probed() -> Self {
    let terminal = crate::caps::get().and_then(crate::caps::Caps::terminal);
    Self::of(terminal.is_some_and(crate::caps::placeholders))
  }

  pub fn name(self) -> &'static str {
    match self {
      Self::Kitty => "kitty",
      Self::Text => "text",
    }
  }

  /// `VAR`, then `flag`, then the terminal `TERM` names, else the one
  /// `TERM_PROGRAM` names; `env` reads a variable.
  pub fn choose(
    flag: Option<Self>,
    env: impl Fn(&str) -> Option<String>,
  ) -> Self {
    use clap::ValueEnum;
    let exported =
      env(VAR).and_then(|value| Self::from_str(&value, false).ok());
    if let Some(graphics) = exported.or(flag) {
      return graphics;
    }
    let program = env("TERM_PROGRAM")
      .map(|name| if name == "iTerm.app" { "iTerm2".into() } else { name });
    let version = env("TERM_PROGRAM_VERSION").unwrap_or_default();
    let term = env("TERM").unwrap_or_default();
    // A terminal launched from another's shell inherits its
    // `TERM_PROGRAM`, so a specific `TERM` wins, as XTVERSION would.
    let named = match (named_by(&term), program) {
      (Some(term), Some(program)) if term.eq_ignore_ascii_case(&program) => {
        format!("{term} {version}")
      }
      (Some(term), _) => term.to_owned(),
      (None, Some(program)) => format!("{program} {version}"),
      (None, None) => return Self::Text,
    };
    Self::of(crate::caps::placeholders(&named))
  }
}

/// The terminal a `TERM` value names: `xterm-kitty` names kitty and
/// `st-256color` st, while `xterm-direct`, `screen`, and `tmux` name
/// none.
fn named_by(term: &str) -> Option<&str> {
  let term = term.strip_prefix("xterm-").unwrap_or(term);
  let name = term.split('-').next().unwrap_or_default();
  let generic =
    ["xterm", "screen", "tmux"].iter().any(|prefix| name.starts_with(prefix))
      || name.is_empty()
      || name.ends_with("color")
      || name == "direct";
  (!generic).then_some(name)
}

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
  let tmux = crate::terminal::tmux();
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

  /// (label, flag, `NAME=value` variables, want)
  type Case<'a> = (&'a str, Option<Graphics>, &'a str, Graphics);

  #[test]
  fn preview_follows_export_then_flag_then_names() {
    use Graphics::{Kitty, Text};
    let cases: [Case; 19] = [
      ("nothing", None, "", Text),
      ("kitty", None, "TERM=xterm-kitty", Kitty),
      (
        "ghostty",
        None,
        "TERM_PROGRAM=ghostty TERM_PROGRAM_VERSION=1.3.1",
        Kitty,
      ),
      (
        "iTerm2",
        None,
        "TERM_PROGRAM=iTerm.app TERM_PROGRAM_VERSION=3.5.6",
        Kitty,
      ),
      (
        "old iTerm2",
        None,
        "TERM_PROGRAM=iTerm.app TERM_PROGRAM_VERSION=3.5.5",
        Text,
      ),
      ("rio", None, "TERM_PROGRAM=rio TERM_PROGRAM_VERSION=0.5.27", Kitty),
      (
        "old rio",
        None,
        "TERM_PROGRAM=rio TERM_PROGRAM_VERSION=0.5.26 TERM=xterm-rio",
        Text,
      ),
      ("rio by TERM", None, "TERM=xterm-rio", Kitty),
      (
        "inherited TERM_PROGRAM",
        None,
        "TERM_PROGRAM=ghostty TERM_PROGRAM_VERSION=1.3.1 TERM=alacritty",
        Text,
      ),
      (
        "st inherits TERM_PROGRAM",
        None,
        "TERM_PROGRAM=ghostty TERM_PROGRAM_VERSION=1.3.1 TERM=st-256color",
        Text,
      ),
      (
        "iTerm2 with xterm-direct",
        None,
        "TERM_PROGRAM=iTerm.app TERM_PROGRAM_VERSION=3.5.6 TERM=xterm-direct",
        Kitty,
      ),
      (
        "iTerm2 with a generic TERM",
        None,
        "TERM_PROGRAM=iTerm.app TERM_PROGRAM_VERSION=3.5.6 TERM=xterm-256color",
        Kitty,
      ),
      ("wezterm", None, "TERM_PROGRAM=WezTerm TERM=xterm-256color", Text),
      (
        "tmux hides the client",
        None,
        "TERM_PROGRAM=tmux TERM=tmux-256color",
        Text,
      ),
      ("flag", Some(Kitty), "TERM=xterm-256color", Kitty),
      ("flag over names", Some(Text), "TERM=xterm-kitty", Text),
      ("export over flag", Some(Text), "VPDF_GRAPHICS=kitty", Kitty),
      ("export over names", None, "VPDF_GRAPHICS=text TERM=xterm-kitty", Text),
      ("bad export", Some(Kitty), "VPDF_GRAPHICS=sixel", Kitty),
    ];
    for (label, flag, variables, want) in cases {
      let env = |name: &str| {
        variables.split(' ').find_map(|pair| {
          let (key, value) = pair.split_once('=')?;
          (key == name).then(|| value.to_owned())
        })
      };
      assert_eq!(Graphics::choose(flag, env), want, "{label}");
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
