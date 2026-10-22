use anyhow::Result;
use std::hint::black_box;
use vpdf_model::geometry::Crop;
use vpdf_ui::page::Viewport;

pub fn navigation() -> Result<()> {
  let prepared = super::fixtures::text()?;
  let screen = crate::screen::cells(120, 40);
  let bounds = || prepared.metadata.iter().map(|(_, bounds)| *bounds);
  let mut layout = super::layouts::tiles(bounds());
  for rotation in [0, 90] {
    layout.restore(2.0, rotation, Crop::default());
    layout.reflow(bounds());
    layout.pan_to(96.0);
    let mut scroll = 400.0;
    let mut view = Viewport::new(&mut layout, &mut scroll, &prepared, screen);
    super::measure(&format!("Viewport::first rotation {rotation}"), || {
      black_box(view.first().expect("prepared page text"));
    })?;
    super::measure(&format!("Viewport::nearest rotation {rotation}"), || {
      black_box(view.nearest((400.0, 300.0)).expect("prepared page text"));
    })?;
  }
  Ok(())
}
