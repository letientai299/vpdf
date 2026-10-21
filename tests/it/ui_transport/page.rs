#[path = "../../support/page.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::{Source, layout, marks};
use vpdf_model::text::PageId;
use vpdf_reader::reader::{Reader, SelectionKind};
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::{page::Canvas, paint::Marks, theme};

#[test]
fn recorded_overlays_keep_layers() -> Result<()> {
  use vpdf_model::scene::{Layer, Op, Scene};
  let mut source = Source::new();
  let layout = layout(&source);
  let mut reader = Reader::default();
  reader.visual(SelectionKind::Character);
  let hovered = [(PageId(0), source.text.lines[0].glyphs[0].bounds)];
  let marks = Marks { hovered: &hovered, ..marks(&reader) };
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 2, menu: None };
  let mut scene = Scene::new();
  canvas.scene(&mut source, &marks, &mut scene)?;
  let fills: Vec<_> = scene
    .iter()
    .map(|op| {
      let Op::Fill(fill) = op else { panic!("Expected overlay fill") };
      **fill
    })
    .collect();
  assert_eq!(fills.len(), 4);
  assert_eq!(fills[0].layer(), Layer::BelowText);
  assert_eq!(fills[0].color(), theme::CURSORLINE);
  for fill in &fills[1..] {
    assert_eq!(fill.layer(), Layer::AboveText);
  }
  assert_eq!(fills[1].color(), theme::LINK);
  assert_eq!(fills[2].color(), theme::SELECTION);
  assert_eq!(fills[3].color(), theme::CURSOR);
  for fill in &fills {
    assert_eq!(fill.frame(), (layout.screen(), 2));
    let rect = fill.rect();
    assert!(rect.x0 >= 0.0 && rect.y0 >= 0.0);
    assert!(rect.x1 <= layout.screen().width());
    assert!(rect.y1 <= layout.screen().height());
  }
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
