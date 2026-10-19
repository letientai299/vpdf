use crate::Result;
use crate::diacritics::DIACRITICS;
use crate::payload::encode;
use crate::terminal::Transport;
use std::io::Write;
use vpdf_model::text::Bounds;

const PIXELS: f32 = 20.0;

pub struct Fit {
  columns: usize,
  rows: usize,
  scale: u32,
}

impl Fit {
  pub fn new(
    bounds: Bounds,
    (columns, rows): (u16, u16),
    cell: f32,
  ) -> Result<Self> {
    anyhow::ensure!(
      bounds.width() > 0.0
        && bounds.width().is_finite()
        && bounds.height() > 0.0
        && bounds.height().is_finite()
        && cell > 0.0
        && cell.is_finite(),
      "Invalid inline page geometry"
    );
    let aspect = bounds.height() / bounds.width();
    let max_columns = DIACRITICS.len() as f32;
    let mut width = f32::from(columns.max(1)).min(max_columns);
    let mut height = (width * aspect / cell).ceil();
    if height > f32::from(rows.max(1)) {
      height = f32::from(rows.max(1)).min(max_columns);
      width = (height * cell / aspect).floor().max(1.0);
    }
    let scale = width * PIXELS / bounds.width();
    anyhow::ensure!(
      width.is_finite()
        && height.is_finite()
        && width >= 1.0
        && height >= 1.0
        && height <= max_columns,
      "Invalid inline cell dimensions"
    );
    Ok(Self {
      columns: width as usize,
      rows: height as usize,
      scale: (scale * 1024.0) as u32,
    })
  }

  pub fn scale(&self) -> u32 {
    self.scale
  }

  pub fn print(
    &self,
    transport: Transport,
    id: u32,
    (width, height): (u32, u32),
    pixels: &[u8],
    out: &mut impl Write,
  ) -> Result<()> {
    let encoded = encode(pixels)?;
    let control = format!(
      "a=T,U=1,q=2,f=24,o=z,i={id},s={width},v={height},c={},r={}",
      self.columns, self.rows
    );
    transport.transmit(out, &control, &encoded)?;
    placeholders(out, id, transport.tmux, (self.columns, self.rows))
  }
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
pub fn image_id(process: u32, tmux: bool) -> u32 {
  let mixed = process.wrapping_mul(2_654_435_761);
  if tmux { mixed % 255 + 1 } else { (mixed & 0x00FF_FFFF).max(1) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn prints_with_explicit_transport() -> Result<()> {
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 };
    let fit = Fit::new(bounds, (40, 12), 2.0)?;
    assert_eq!((fit.columns, fit.rows, fit.scale()), (18, 12, 614));
    for tmux in [false, true] {
      let mut out = Vec::new();
      fit.print(Transport { tmux }, 123, (1, 1), &[255; 3], &mut out)?;
      let output = String::from_utf8(out)?;
      let start = if tmux { "\x1bPtmux;\x1b\x1b_G" } else { "\x1b_G" };
      assert!(output.starts_with(start));
      assert!(output.contains("i=123,s=1,v=1,c=18,r=12,m=0;"));
      assert_eq!(output.matches('\u{10EEEE}').count(), 18 * 12);
      assert_eq!(output.matches('\n').count(), 11);
      assert!(output.ends_with("\x1b[39m"));
    }
    Ok(())
  }

  #[test]
  fn rejects_unprintable_geometry() {
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 100.0 };
    assert!(Fit::new(bounds, (297, u16::MAX), 2.0).is_err());
    assert!(Fit::new(bounds, (40, 12), 0.0).is_err());
    assert!(Fit::new(bounds, (40, 12), f32::NAN).is_err());
    for invalid in [
      Bounds { x1: 0.0, ..bounds },
      Bounds { x1: f32::INFINITY, ..bounds },
      Bounds { y1: f32::INFINITY, ..bounds },
    ] {
      assert!(Fit::new(invalid, (40, 12), 2.0).is_err());
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
    assert!((1..=255).contains(&image_id(1234, true)));
    assert!((1..=0xFF_FFFF).contains(&image_id(1234, false)));
    Ok(())
  }
}
