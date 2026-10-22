include!("presentation/draw.rs");

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
