use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::At;
use vpdf_model::scene::{Recorder, Scene};
use vpdf_ui::keys::PanelKind;
use vpdf_ui::layout::tiles::{Frame, Tiles, View};
use vpdf_ui::panel::{Panel, Row, help};

pub fn compose() -> Result<()> {
  let geometry: super::Geometry = super::fixture("geometry")?;
  let layout =
    super::layouts::tiles(geometry.pages.into_iter().map(super::bounds));
  let screen = layout.screen();
  let view =
    View { layout: &layout, scroll: 0.0, target: Some(layout.pages()[1].top) };
  for invalidate in [true, false] {
    let mut tiles = Tiles::default();
    super::measure(
      &format!("Tiles::scene missing invalidate={invalidate}"),
      || {
        if invalidate {
          tiles.invalidate();
        }
        let mut recorder = Recorder::new(|_| false);
        black_box(
          tiles
            .scene(view, Frame { pane: 0, screen, left: 0 }, &mut recorder)
            .unwrap(),
        );
        black_box(recorder.into_scene());
      },
    )?;
  }
  let rows = help(|_| false).iter().map(|line| Row::parse(line)).collect();
  let mut panel = Panel::new(PanelKind::Help, rows);
  let mut scene = Scene::new();
  super::measure("Panel::scene_list", || {
    scene.clear();
    panel.scene_list(&mut scene, At::default(), 50, 40);
    black_box(&scene);
  })
}
