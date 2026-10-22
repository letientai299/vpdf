#[path = "../../support/tiles.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::{frame, layout};
use std::io::Write;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::layout::tiles::*;
use vpdf_ui::layout::visible;

#[test]
fn failed_placement_retains_requests() -> Result<()> {
  use vpdf_terminal::payload::{Encoder, Tile};
  struct Broken;
  impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
      Err(std::io::Error::other("Output unavailable"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
      Ok(())
    }
  }
  let layout = layout();
  let key = visible(&layout, 0.0)[0];
  let previous = visible(&layout, layout.pages()[2].top)[0];
  let mut tiles = Tiles::default();
  let old_view =
    View { layout: &layout, scroll: layout.pages()[2].top, target: None };
  tiles
    .scene(
      old_view,
      frame(&layout),
      &mut vpdf_model::scene::Recorder::new(|_| false),
    )
    .unwrap();
  let previous_wanted = tiles.wanted().to_vec();
  assert!(previous_wanted.contains(&previous));
  let mut graphics = Graphics::default();
  let pixels = vec![255; (key.width * key.height * 3) as usize];
  let tile =
    Tile { key, payload: Encoder::default().encode(&pixels)?, render_ms: 0.0 };
  graphics.upload(0, &tile, &mut std::io::sink())?;
  let view = View { layout: &layout, scroll: 0.0, target: None };
  assert!(
    draw(&mut Broken, Some(&mut graphics), |scene| tiles.scene(
      view,
      frame(&layout),
      scene
    ))
    .and_then(std::convert::identity)
    .is_err()
  );
  assert_eq!(tiles.wanted(), previous_wanted);
  Ok(())
}
