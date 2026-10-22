#[path = "../../support/region.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::snapshot;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::region::*;
#[test]
fn recorded_regions_keep_hits() -> Result<()> {
  use vpdf_model::scene::Scene;
  use vpdf_terminal::graphics::Presentation;
  use vpdf_terminal::payload::Encoder;
  let layout = fixture::layout();
  let canvas = fixture::canvas(&layout);
  let sweep = Some(fixture::sweep());
  for crop_quirk in [false, true] {
    let mut recorded = Region::open(snapshot(1));
    let mut scene = Scene::new();
    canvas.scene(Some(&mut recorded), sweep, crop_quirk, &mut scene);
    let area = recorded.cells().unwrap();
    let profile = Presentation { crop_quirk, ..Presentation::default() };
    let mut graphics = Graphics::new(profile, &Encoder::default());
    let mut replay = Graphics::new(profile, &Encoder::default());
    let mut direct = Region::open(snapshot(1));
    for _ in 0..2 {
      let mut expected = Vec::new();
      graphics.begin(&mut expected)?;
      canvas.draw(Some(&mut direct), sweep, &mut graphics, &mut expected)?;
      graphics.end(&mut expected)?;
      assert_eq!(direct.cells(), Some(area));
      assert_eq!(replay_frame(&mut replay, &scene)?, expected);
    }
  }
  Ok(())
}
