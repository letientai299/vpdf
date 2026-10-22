use vpdf_model::scene::Op;
use vpdf_terminal::Result;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::painter::Painter;

pub fn replay_frame(
  graphics: &mut Graphics,
  scene: &[Op<'_>],
) -> Result<Vec<u8>> {
  let mut out = Vec::new();
  graphics.begin(&mut out)?;
  let mut painter = Painter::new(&mut out).with_graphics(graphics);
  painter.paint(scene);
  painter.finish()?;
  graphics.end(&mut out)?;
  Ok(out)
}
