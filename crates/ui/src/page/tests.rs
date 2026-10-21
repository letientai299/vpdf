use super::*;
#[path = "../../../../tests/support/page.rs"]
mod fixture;
use fixture::marks;
pub(super) use fixture::{Source, layout};
use vpdf_reader::reader::Reader;
use vpdf_search::search::Hit;

#[test]
fn text_requests_follow_visible_matches() -> Result<()> {
  let mut source = Source::new();
  let layout = layout(&source);
  let mut reader = Reader::default();
  reader.scroll = layout.pages()[1].top;
  let hits: Vec<_> = [0, 1, 2]
    .into_iter()
    .flat_map(|page| {
      let position = Position { page: PageId(page), ..Position::default() };
      std::iter::repeat_n(
        Hit { first: position, last: position, excerpt: String::new() },
        301,
      )
    })
    .collect();
  let marks = Marks { matches: &hits, ..marks(&reader) };
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  canvas.scene(&mut source, &marks, &mut Vec::new())?;
  assert_eq!(source.reads, vec![PageId(1); 300]);
  Ok(())
}

#[test]
fn idle_and_hover_need_no_text() -> Result<()> {
  let mut source = Source::new();
  source.fail = true;
  let layout = layout(&source);
  let reader = Reader::default();
  let hovered = [(PageId(0), source.text.lines[0].glyphs[0].bounds)];
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  for hovered in [&[][..], &hovered[..]] {
    let marks = Marks { hovered, ..marks(&reader) };
    canvas.scene(&mut source, &marks, &mut Vec::new())?;
  }
  assert_eq!(source.reads, [] as [PageId; 0]);
  Ok(())
}

#[test]
fn text_errors_stop_drawing() {
  let mut source = Source::new();
  source.fail = true;
  let layout = layout(&source);
  let mut reader = Reader::default();
  reader.visual(SelectionKind::Character);
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  let mut out = Vec::new();
  let error = canvas.scene(&mut source, &marks(&reader), &mut out).unwrap_err();
  assert_eq!(error.to_string(), "Text unavailable");
  assert_eq!(source.reads, [PageId(0)]);
  assert_eq!(out, [] as [vpdf_model::scene::Op<'_>; 0]);
}

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
  Ok(())
}
