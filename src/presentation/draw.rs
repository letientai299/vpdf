use anyhow::Result;
use std::io::Write;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::painter::Painter;

fn painter<'a, W: Write>(
  out: &'a mut W,
  graphics: Option<&'a mut Graphics>,
) -> Painter<'a, W> {
  let painter = Painter::new(out);
  match graphics {
    Some(graphics) => painter.with_graphics(graphics),
    None => painter,
  }
}

/// Reports output errors before composed values.
pub fn draw<W: Write, T>(
  out: &mut W,
  graphics: Option<&mut Graphics>,
  compose: impl FnOnce(&mut Painter<'_, W>) -> T,
) -> Result<T> {
  let mut painter = painter(out, graphics);
  let result = compose(&mut painter);
  painter.finish()?;
  Ok(result)
}

/// Composition failures precede cached output failures.
pub fn try_draw<W: Write, T>(
  out: &mut W,
  graphics: Option<&mut Graphics>,
  compose: impl FnOnce(&mut Painter<'_, W>) -> Result<T>,
) -> Result<T> {
  let mut painter = painter(out, graphics);
  let result = compose(&mut painter)?;
  painter.finish()?;
  Ok(result)
}
