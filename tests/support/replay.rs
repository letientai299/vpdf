use vpdf_model::scene::Op;
use vpdf_terminal::Result;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::painter::Painter;

/// One frame of `draw` through `graphics`.
pub fn framed(
  graphics: &mut Graphics,
  draw: impl FnOnce(&mut Painter<'_, Vec<u8>>) -> Result<()>,
) -> Result<Vec<u8>> {
  let mut out = Vec::new();
  graphics.begin(&mut out)?;
  let mut painter = Painter::new(&mut out).with_graphics(graphics);
  draw(&mut painter)?;
  painter.finish()?;
  graphics.end(&mut out)?;
  Ok(out)
}

pub fn replay_frame(
  graphics: &mut Graphics,
  scene: &[Op<'_>],
) -> Result<Vec<u8>> {
  framed(graphics, |painter| {
    painter.paint(scene);
    Ok(())
  })
}
