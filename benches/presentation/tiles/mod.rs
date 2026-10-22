use crate::presentation;
use anyhow::Result;
use std::hint::black_box;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::layout::tiles::{Frame, Tiles, View};

pub fn draw() -> Result<()> {
  let geometry: super::Geometry = super::fixture("geometry")?;
  let layout =
    super::layouts::tiles(geometry.pages.into_iter().map(super::bounds));
  let screen = layout.screen();
  let view =
    View { layout: &layout, scroll: 0.0, target: Some(layout.pages()[1].top) };
  for invalidate in [true, false] {
    let mut tiles = Tiles::default();
    let mut graphics = Graphics::default();
    let mut out = Vec::new();
    let mut draw = || {
      out.clear();
      if invalidate {
        tiles.invalidate();
      }
      let update = presentation::draw(&mut out, Some(&mut graphics), |scene| {
        tiles.scene(view, Frame { pane: 0, screen, left: 0 }, scene)
      })
      .and_then(std::convert::identity)
      .expect("prepared geometry");
      black_box(update);
    };
    draw();
    super::measure(
      &format!("Tiles::draw missing invalidate={invalidate}"),
      draw,
    )?;
  }
  Ok(())
}
