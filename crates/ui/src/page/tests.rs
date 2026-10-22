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
  use vpdf_model::scene::{Layer, Op};
  let (_, layout, _, scene) = fixture::overlays()?;
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

#[test]
fn skewed_overlays_follow_projection() -> Result<()> {
  use vpdf_model::scene::{Layer, Op};
  let mut source = Source::new();
  let quad = Quad([(10.0, 30.0), (30.0, 20.0), (33.0, 30.0), (13.0, 40.0)]);
  let glyph =
    &mut std::sync::Arc::make_mut(&mut source.text).lines[0].glyphs[0];
  glyph.quad = Some(quad);
  glyph.bounds = quad.bounds();
  for rotation in [0, 90, 180, 270] {
    let mut layout = Layout::new(
      (0..3).map(|page| source.bounds(PageId(page))),
      Screen { columns: 100, rows: 100, ..layout(&source).screen() },
    );
    layout.restore(1.0, rotation, crate::layout::Crop::default());
    layout.reflow((0..3).map(|page| source.bounds(PageId(page))));
    let mut reader = Reader::default();
    reader.visual(SelectionKind::Line);
    let rect = layout.project(PageId(0), quad.bounds(), 0.0);
    reader.scroll = (rect.y0 - 10.0).max(0.0);
    let canvas =
      Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
    let mut scene = Vec::new();
    canvas.scene(&mut source, &marks(&reader), &mut scene)?;
    let polygons: Vec<_> = scene
      .iter()
      .filter_map(|op| {
        if let Op::Polygon(polygon) = op { Some(polygon) } else { None }
      })
      .collect();
    assert_eq!(polygons.len(), 3);
    for (color, layer) in [
      (theme::CURSORLINE, Layer::BelowText),
      (theme::SELECTION, Layer::AboveText),
      (theme::CURSOR, Layer::AboveText),
    ] {
      let colored: Vec<_> =
        polygons.iter().filter(|p| p.fill().color() == color).collect();
      assert_eq!(colored.len(), 1, "{rotation}: {color:?}");
      assert_eq!(colored[0].fill().layer(), layer);
      assert!(colored[0].strips().len() > 1);
    }
    assert_eq!(polygons[1].quad(), polygons[2].quad());
  }
  Ok(())
}

#[test]
fn ocr_line_shapes() -> Result<()> {
  use vpdf_model::scene::Op;
  use vpdf_model::text::{Glyph, Line};
  let mut source = Source::new();
  let text = std::sync::Arc::make_mut(&mut source.text);
  text.lines = (0..10)
    .map(|row| {
      let y = 10.0 + row as f32 * 10.0;
      Line {
        ocr: Some(Quad([
          (10.0, y),
          (90.0, y - 2.0),
          (90.0, y + 3.0),
          (10.0, y + 5.0),
        ])),
        glyphs: (0..16)
          .map(|i| Glyph {
            ch: 'x',
            quad: None,
            bounds: Bounds {
              x0: 10.0 + i as f32 * 5.0,
              y0: y,
              x1: 15.0 + i as f32 * 5.0,
              y1: y + 5.0,
            },
          })
          .collect(),
        ..Line::default()
      }
    })
    .collect();
  let layout = Layout::new(
    (0..3).map(|p| source.bounds(PageId(p))),
    Screen {
      columns: 100,
      rows: 100,
      cell_width: 1,
      cell_height: 2,
      origin: (0, 0),
    },
  );
  let mut reader = Reader::default();
  reader.visual(SelectionKind::Line);
  reader.cursor.line = 9;
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  let mut scene = Vec::new();
  canvas.scene(&mut source, &marks(&reader), &mut scene)?;
  assert_eq!(scene.len(), 12);
  assert!(scene.iter().all(|op| matches!(op, Op::Polygon(_))));
  let menu = Bounds { x0: 30.0, y0: 0.0, x1: 60.0, y1: 150.0 };
  let canvas = Canvas { menu: Some(menu), ..canvas };
  let mut scene = Vec::new();
  canvas.scene(&mut source, &marks(&reader), &mut scene)?;
  assert_ne!(scene, [] as [Op<'_>; 0]);
  for op in scene {
    let Op::Polygon(p) = op else { panic!("Expected polygon") };
    assert!(crate::layout::clip(p.clip(), menu).is_none());
  }
  Ok(())
}

#[test]
fn cursorline_follows_reader_mode() -> Result<()> {
  use vpdf_model::scene::Op;
  let mut source = Source::new();
  let layout = layout(&source);
  let canvas =
    Canvas { layout: &layout, screen: layout.screen(), left: 0, menu: None };
  for mode in [0, 1, 2] {
    let mut reader = Reader::default();
    match mode {
      1 => reader.enter_cursor(),
      2 => reader.visual(SelectionKind::Character),
      _ => {}
    }
    for enabled in [false, true] {
      let paint = Marks { cursorline: enabled, ..marks(&reader) };
      let mut scene = Vec::new();
      canvas.scene(&mut source, &paint, &mut scene)?;
      let bands = scene.iter().filter(|op| {
        matches!(op, Op::Fill(fill) if fill.color() == theme::CURSORLINE)
      }).count();
      assert_eq!(bands, usize::from(enabled && mode != 0));
    }
  }
  Ok(())
}
