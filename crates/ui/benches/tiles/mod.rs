use anyhow::Result;
use std::hint::black_box;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::layout::tiles::{Frame, Tiles, View};
use vpdf_ui::layout::{Layout, Screen};

pub fn draw() -> Result<()> {
  let geometry: super::Geometry = super::fixture("geometry")?;
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let layout =
    Layout::new(geometry.pages.into_iter().map(super::bounds), screen);
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
      let update = tiles
        .draw(view, Frame { pane: 0, screen, left: 0 }, &mut graphics, &mut out)
        .expect("prepared geometry");
      black_box(update);
    };
    draw();
    super::measure(
      &format!("Tiles::draw missing invalidate={invalidate}"),
      draw,
    )?;
  }
  let tiles = Tiles::default();
  super::measure("Tiles::nearby ten pages", || {
    black_box(tiles.nearby(black_box(view), 10));
  })?;
  Ok(())
}
