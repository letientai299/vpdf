include!("presentation/draw.rs");

pub(crate) fn inactive<'a>(
  scene: &mut impl vpdf_model::scene::Draw<'a>,
  screen: vpdf_ui::layout::Screen,
  left: u16,
  opacity: vpdf_model::theme::Opacity,
) {
  use vpdf_model::scene::{Layer, Stage};
  use vpdf_model::theme::{Colors, Overlay, Paint};
  let veil = Overlay::Inactive;
  if veil.resolve(Colors::Terminal, opacity)[3] == 0
    || screen.rows < 2
    || screen.columns == 0
  {
    return;
  }
  scene.stretch(
    Paint::Overlay(veil),
    Layer::AboveText,
    (0, screen.columns, screen.rows - 1),
    Stage { screen, left },
  );
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::{self, Write};
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
