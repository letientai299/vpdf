use super::*;
use crate::page::tests::{Source, layout};
use crate::text::Glyph;
use std::sync::Arc;

#[test]
fn visibility_preserves_lazy_text_access() -> Result<()> {
  let source = Source::new();
  let mut layout = layout(&source);
  let screen = layout.screen();
  let mut scroll = 0.0;
  let mut view = Viewport::new(&mut layout, &mut scroll, source, screen);
  let offscreen = Position { page: PageId(2), ..Position::default() };
  assert!(!view.visible(offscreen)?);
  assert_eq!(view.source.reads, [] as [PageId; 0]);
  assert_eq!(view.first()?, Some(Position::default()));
  assert_eq!(view.source.reads, [PageId(0), PageId(0)]);
  view.source.fail = true;
  assert!(!view.visible(offscreen)?);
  assert!(view.point(Position::default()).is_err());
  assert!(view.nearest((0.0, 0.0)).is_err());
  assert!(view.reveal(Position::default(), 10.0).is_err());
  assert_eq!(*view.scroll, 0.0);
  Ok(())
}

#[test]
fn nearest_prefers_whole_visible_glyphs() -> Result<()> {
  let mut source = Source::new();
  let glyph = |ch, x0, y0, x1, y1| Glyph {
    quad: None,
    ch,
    bounds: Bounds { x0, y0, x1, y1 },
  };
  Arc::make_mut(&mut source.text).lines[0].glyphs = vec![
    glyph(' ', 10.0, 40.0, 20.0, 50.0),
    glyph('a', 10.0, 30.0, 20.0, 40.0),
    glyph('b', 10.0, 45.0, 20.0, 55.0),
    glyph('c', 100.0, 30.0, 110.0, 40.0),
    glyph('d', 30.0, 30.0, 40.0, 40.0),
  ];
  let mut layout = layout(&source);
  let screen = Screen { rows: 6, ..layout.screen() };
  let mut scroll = 0.0;
  let mut view = Viewport::new(&mut layout, &mut scroll, source, screen);
  let at = |glyph| Position { glyph, ..Position::default() };
  assert!(view.visible(at(2))?);
  assert!(!view.visible(at(3))?);
  assert_eq!(view.nearest((15.0, 49.0))?, Some(at(1)));
  assert_eq!(view.nearest((35.0, 49.0))?, Some(at(4)));
  assert_eq!(view.first()?, Some(at(1)));
  *view.scroll = 45.0;
  assert_eq!(view.nearest((15.0, 49.0))?, Some(at(2)));
  *view.scroll = 60.0;
  assert_eq!(view.first()?, None);
  assert_eq!(view.nearest((15.0, 49.0))?, None);
  Ok(())
}

#[test]
fn landing_keeps_context_after_reveal() -> Result<()> {
  let mut source = Source::new();
  Arc::make_mut(&mut source.text).lines[0].glyphs[0].bounds =
    Bounds { x0: 10.0, y0: 200.0, x1: 20.0, y1: 210.0 };
  let mut layout = layout(&source);
  let screen = layout.screen();
  let mut scroll = 0.0;
  let mut view = Viewport::new(&mut layout, &mut scroll, source, screen);
  let at = Position::default();
  view.reveal(at, 10.0)?;
  assert_eq!(*view.scroll, 130.0);
  *view.scroll = 0.0;
  view.source.reads.clear();
  view.land(at, 10.0)?;
  assert_eq!(*view.scroll, 190.0);
  assert_eq!(view.source.reads, [PageId(0), PageId(0)]);
  view.source.reads.clear();
  view.land(at, 10.0)?;
  assert_eq!(*view.scroll, 190.0);
  assert_eq!(view.source.reads, [PageId(0)]);
  view.align(at, 0.5)?;
  assert_eq!(*view.scroll, 160.0);
  let missing = Position { page: PageId(1), line: 10, ..at };
  assert_eq!(view.point(missing)?, (0.0, 0.0));
  view.reveal(missing, 10.0)?;
  assert_eq!(*view.scroll, view.layout.pages()[1].top);
  Ok(())
}
