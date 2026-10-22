#[path = "../../support/panel.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::panel;

#[test]
fn recorded_panels_outlive_their_source() -> Result<()> {
  let at = vpdf_model::geometry::At { column: 7, row: 3 };
  let scene = {
    let mut source = panel();
    let mut scene = vpdf_model::scene::Scene::new();
    source.scene_list(&mut scene, at, 20, 6);
    scene
  };
  let mut recorded = Vec::new();
  vpdf_terminal::painter::paint(&mut recorded, &scene)?;
  let mut streamed = Vec::new();
  draw(&mut streamed, None, |scene| panel().scene_list(scene, at, 20, 6))?;
  assert_eq!(recorded, streamed);
  assert!(String::from_utf8(recorded)?.contains("日本語"));
  Ok(())
}
