use super::*;
use crate::overlays::Overlays;
use crate::preview::Preview;
use crate::region::{CaptureId, Region, Snapshot};
use vpdf_model::document::{Annotation, Link, Target};

impl crate::source::Raster for &mut Source {
  fn raster(
    &mut self,
    _: vpdf_model::tile::TileKey,
    _: impl FnOnce(&[u8], usize) -> Result<()>,
  ) -> Result<()> {
    self.lookups.push("raster");
    anyhow::bail!("Raster unavailable")
  }
}

fn snapshot() -> Snapshot {
  Snapshot {
    id: CaptureId::new(1),
    page: PageId(0),
    bounds: Bounds { x0: 10.0, y0: 40.0, x1: 20.0, y1: 50.0 },
  }
}

fn with_region(region: Region) -> Overlays {
  let mut overlays = Overlays::default();
  overlays.region = Some(region);
  overlays
}

fn with_preview(preview: Option<Preview>) -> Overlays {
  let mut overlays = Overlays::default();
  overlays.preview = preview;
  overlays
}

fn note() -> Annotation {
  Annotation {
    kind: "Note".into(),
    areas: vec![snapshot().bounds],
    contents: "A note".into(),
    author: String::new(),
  }
}

#[test]
fn hints_project_visible_boxes() -> Result<()> {
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.at = At { column: 17, row: 9 };
    fixture.frame.pane.left = 3;
    fixture.frame.pane.text = text;
    fixture.frame.visible.rows = 1;
    let target = Target::Uri("https://example.org/".into());
    let mut link = Link::new(
      Bounds { x0: 10.0, y0: 900.0, x1: 20.0, y1: 910.0 },
      target.clone(),
    );
    link.append(Link::new(
      Bounds { x0: 10.0, y0: 35.0, x1: 20.0, y1: 65.0 },
      target.clone(),
    ));
    fixture.source.links.push(link);
    let hints = fixture.view().hints(true)?;
    assert_eq!(hints.items().len(), 1);
    let hint = &hints.items()[0];
    assert_eq!(hint.cell, if text { (3, 2) } else { (4, 5) });
    assert_eq!(hint.column, 3);
    assert_eq!(hint.target, target);
    assert_eq!(hints.into_target(0), Some((target, true)));
    assert_eq!(fixture.source.reads > 0, text);
    assert_eq!(fixture.source.lookups, ["links"]);
  }
  Ok(())
}

#[test]
fn hints_preserve_lazy_failures() {
  for text in [false, true] {
    let mut fixture = Fixture::new();
    fixture.frame.pane.text = text;
    fixture.source.fail = 1;
    fixture.source.fail_lookup = 1;
    let error = fixture.view().hints(false).err().unwrap().to_string();
    assert_eq!(
      error,
      if text { "Text unavailable" } else { "Links unavailable" }
    );
    assert_eq!(fixture.source.reads, usize::from(text));
    assert_eq!(fixture.source.lookups.len(), usize::from(!text));
  }
  let mut fixture = Fixture::new();
  assert_eq!(
    fixture.view().hints(false).err().unwrap().to_string(),
    "No links in view"
  );
  assert_eq!(fixture.source.reads, 0);
}

#[test]
fn overlays_use_distinct_visibility() {
  let mut fixture = Fixture::new();
  fixture.frame.visible.rows = 3;
  fixture.source.fail = 1;
  let mut region = with_region(Region::selected(snapshot()));
  fixture.view().check_region(&mut region);
  assert!(region.region.is_some());
  let mut preview = with_preview(Preview::note(
    Position::default(),
    &note(),
    true,
    &fixture.layout,
  ));
  let peek = preview.preview.as_mut().unwrap();
  peek.anchor(PageId(0), 0, snapshot().bounds);
  peek.keep(true);
  fixture.view().drop_stray_preview(&mut preview);
  assert!(preview.preview.is_none());
  assert_eq!(fixture.source.reads, 0);

  fixture.frame.pane.text = true;
  fixture.view().check_region(&mut region);
  assert!(region.region.is_none());
  assert_eq!(fixture.source.reads, 1);
  let mut invalid = snapshot();
  invalid.page = PageId(9);
  let mut bar = with_region(Region::open(invalid));
  fixture.view().check_region(&mut bar);
  assert!(bar.region.is_some());
  assert_eq!(fixture.source.reads, 1);
}

#[test]
fn figure_points_preserve_fallbacks() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.reader.enter_cursor();
  fixture.source.fail = 1;
  let region = Region::selected(snapshot());
  let point = fixture.view().figure_point(Some(&region))?;
  assert_eq!((point.page, point.x, point.y), (PageId(0), 10.0, 40.0));
  assert_eq!(fixture.source.reads, 0);
  assert!(fixture.view().figure_point(None).is_err());
  fixture.source.fail = 0;
  let point = fixture.view().figure_point(None)?;
  assert_eq!((point.x, point.y), (10.0, 10.0));
  fixture.source.text = Arc::new(Text::default());
  fixture.reader.scroll = 180.0;
  let point = fixture.view().figure_point(None)?;
  assert_eq!((point.page, point.x, point.y), (PageId(0), 0.0, 180.0));
  Ok(())
}

#[test]
fn positions_ignore_screen_origin() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.frame.pane.at = At { column: 17, row: 9 };
  fixture.frame.pane.left = 3;
  fixture.frame.pane.text = true;
  let position = Position { line: 1, ..Position::default() };
  let rect = fixture.view().position_rect(position)?.unwrap();
  assert_eq!(rect, Bounds { x0: 0.0, y0: 10.0, x1: 10.0, y1: 20.0 });
  fixture.frame.pane.text = false;
  fixture.reader.scroll = 5.0;
  let rect = fixture.view().position_rect(position)?.unwrap();
  assert_eq!(rect, Bounds { x0: 10.0, y0: 25.0, x1: 20.0, y1: 35.0 });
  Ok(())
}

#[test]
fn preview_failures_preserve_state() -> Result<()> {
  let mut fixture = Fixture::new();
  let mut preview = with_preview(Preview::note(
    Position::default(),
    &note(),
    true,
    &fixture.layout,
  ));
  let old = preview.preview.as_ref().unwrap().source();
  fixture.view().show_preview(
    &mut preview,
    Target::Uri("external".into()),
    old,
  )?;
  assert_eq!(fixture.reader.message, "external");
  assert_eq!(fixture.source.reads, 0);
  assert!(
    fixture
      .view()
      .show_preview(&mut preview, Target::Page(PageId(0)), old)
      .is_err()
  );
  assert_eq!(fixture.source.lookups, ["raster"]);
  assert_eq!(preview.preview.as_ref().unwrap().source(), old);

  fixture.frame.pane.text = true;
  fixture.source.fail = 1;
  let next = Position { line: 2, ..old };
  assert!(
    fixture
      .view()
      .show_preview(&mut preview, Target::Page(PageId(0)), next)
      .is_err()
  );
  assert_eq!(preview.preview.as_ref().unwrap().source(), old);
  let mut empty = note();
  empty.contents.clear();
  fixture.view().preview_annotation(&mut preview, next, &empty);
  assert_eq!(preview.preview.as_ref().unwrap().source(), old);
  assert_eq!(fixture.source.reads, 1);
  Ok(())
}

#[test]
fn hover_anchors_require_text() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  assert!(fixture.view().hover_target(PageId(0), 0)?.is_none());
  assert_eq!(fixture.source.reads, 0);
  let bounds = fixture.source.text.lines[0].glyphs[0].bounds;
  fixture.source.links.push(Link::new(bounds, Target::Page(PageId(0))));
  assert!(fixture.view().hover_target(PageId(0), 0).is_err());
  fixture.source.fail = 0;
  let (_, position) = fixture.view().hover_target(PageId(0), 0)?.unwrap();
  assert_eq!(position, Position::default());
  fixture.source.text = Arc::new(Text::default());
  assert!(fixture.view().hover_target(PageId(0), 0)?.is_none());
  Ok(())
}

#[test]
fn previews_wait_for_visibility() {
  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  let mut preview = with_preview(Preview::note(
    Position::default(),
    &note(),
    false,
    &fixture.layout,
  ));
  fixture.view().drop_stray_preview(&mut preview);
  assert!(preview.preview.is_some());
  assert_eq!(fixture.source.reads, 1);
  fixture.source.fail = 0;
  fixture.view().drop_stray_preview(&mut preview);
  assert!(preview.preview.is_some());
  fixture.reader.scroll = 200.0;
  fixture.view().drop_stray_preview(&mut preview);
  assert!(preview.preview.is_none());
}

#[test]
fn icon_drawing_avoids_text() -> Result<()> {
  let mut fixture = Fixture::new();
  fixture.source.fail = 1;
  let mut preview = with_preview(Preview::note(
    Position::default(),
    &note(),
    true,
    &fixture.layout,
  ));
  preview.preview.as_mut().unwrap().anchor(PageId(0), 0, snapshot().bounds);
  let mut graphics = crate::testing::empty();
  fixture.view().scene_preview(&mut preview, &mut graphics)?;
  assert!(preview.preview.as_ref().unwrap().cells().is_some());
  assert_ne!(graphics.into_scene(), [] as [vpdf_model::scene::Op<'_>; 0]);
  let mut graphics = crate::testing::empty();
  assert_eq!(fixture.source.reads, 0);
  assert_eq!(fixture.source.lookups, [] as [&str; 0]);
  fixture.frame.pane.text = true;
  assert!(fixture.view().scene_preview(&mut preview, &mut graphics).is_err());
  assert!(preview.preview.is_some());
  assert_eq!(graphics.into_scene(), [] as [vpdf_model::scene::Op<'_>; 0]);
  assert_eq!(fixture.source.reads, 1);
  Ok(())
}

#[test]
fn anchor_hits_skip_surroundings() {
  use vpdf_model::input::{MouseEvent, MouseEventKind};
  let mut frame = Fixture::new().frame.pane;
  frame.at = At { column: 5, row: 7 };
  frame.left = 3;
  let anchor = Some((PageId(0), 2, snapshot().bounds));
  for (column, row) in [(7, 8), (8, 16)] {
    let mouse = MouseEvent::new(MouseEventKind::Moved, column, row);
    assert!(!crate::preview::on_anchor(anchor, frame, mouse, |_, _| panic!(
      "Non-page hit lookup"
    )));
  }
  let mouse = MouseEvent::new(MouseEventKind::Moved, 8, 8);
  assert!(crate::preview::on_anchor(anchor, frame, mouse, |column, row| {
    assert_eq!((column, row), (3, 1));
    crate::pointer::Hit::Note { page: PageId(0), index: 2, text: false }
  }));
  assert!(!crate::preview::on_anchor(anchor, frame, mouse, |_, _| {
    crate::pointer::Hit::Scrollbar
  }));
}
