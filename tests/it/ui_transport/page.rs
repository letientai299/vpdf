#[path = "../../support/page.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::marks;
use vpdf_model::text::PageId;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::{page::Canvas, paint::Marks};

#[test]
fn recorded_overlays_keep_layers() -> Result<()> {
  let (mut source, layout, reader, scene) = fixture::overlays()?;
  let hovered = [(PageId(0), source.page.text.lines[0].glyphs[0].bounds)];
  let marks = Marks { hovered: &hovered, ..marks(&reader) };
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 2, menu: None };
  let mut expected = Vec::new();
  canvas.draw(&mut source, &marks, &mut Graphics::default(), &mut expected)?;
  drop(source);
  let mut actual = Vec::new();
  let mut graphics = Graphics::default();
  let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
    .with_graphics(&mut graphics);
  painter.paint(&scene);
  painter.finish()?;
  assert_eq!(actual, expected);
  Ok(())
}
