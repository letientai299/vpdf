use crate::presentation::*;
use anyhow::Result;
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::{Panel, Row};

#[test]
fn recorded_panels_outlive_their_source() -> Result<()> {
  let panel = || {
    Panel::new(
      PanelKind::Help,
      ["# 日本語", "j: down", "k: up"].map(Row::parse).into(),
    )
  };
  let at = vpdf_model::geometry::At { column: 7, row: 3 };
  let scene = {
    let mut source = panel();
    let mut scene = vpdf_model::scene::Scene::new();
    source.scene_list(&mut scene, at, 20, 6);
    assert_eq!(source.row_at(0, 0), None);
    assert_eq!(source.row_at(0, 1), Some(0));
    assert_eq!(source.row_at(0, 2), Some(1));
    assert_eq!(source.row_at(19, 2), None);
    scene
  };
  let mut recorded = Vec::new();
  vpdf_terminal::painter::paint(&mut recorded, &scene)?;
  let mut streamed = Vec::new();
  panel().list(&mut streamed, at, 20, 6)?;
  assert_eq!(recorded, streamed);
  assert!(String::from_utf8(recorded)?.contains("日本語"));
  Ok(())
}
