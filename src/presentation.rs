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
pub(crate) fn draw<W: Write, T>(
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
pub(crate) fn try_draw<W: Write, T>(
  out: &mut W,
  graphics: Option<&mut Graphics>,
  compose: impl FnOnce(&mut Painter<'_, W>) -> Result<T>,
) -> Result<T> {
  let mut painter = painter(out, graphics);
  let result = compose(&mut painter)?;
  painter.finish()?;
  Ok(result)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io;
  use vpdf_model::scene::Draw;

  struct Broken;
  impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
      Err(io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> io::Result<()> {
      Ok(())
    }
  }

  #[test]
  fn output_errors_take_priority() {
    let error = draw(&mut Broken, None, |scene| {
      scene.text("partial");
      Err::<(), _>(anyhow::anyhow!("Composition failed"))
    })
    .unwrap_err();
    assert_eq!(
      error.downcast_ref::<io::Error>().unwrap().kind(),
      io::ErrorKind::BrokenPipe
    );
  }

  #[test]
  fn composition_errors_take_priority() {
    let error = try_draw(&mut Broken, None, |scene| {
      scene.text("partial");
      Err::<(), _>(anyhow::anyhow!("Composition failed"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "Composition failed");
  }
}
