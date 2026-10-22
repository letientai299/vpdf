#[path = "../../../../../tests/support/tiles.rs"]
mod fixture;
use super::*;
use anyhow::Result;
use fixture::{frame, layout};
#[test]
fn bounded_neighborhood() {
  let limit = 10;
  for count in [1, 2, 10, 244, 3329] {
    for anchor in [0, count / 2, count - 1] {
      for forward in [false, true] {
        let pages = nearby(count, PageId(anchor), forward, limit);
        assert_eq!(pages.len(), count.min(limit));
        assert_eq!(pages[0], PageId(anchor));
        assert!(pages.iter().all(|page| page.0 < count));
        let mut unique = pages.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), pages.len());
      }
    }
  }
  assert_eq!(
    &nearby(100, PageId(50), true, limit)[..3],
    &[PageId(50), PageId(51), PageId(49)]
  );
  assert_eq!(
    &nearby(100, PageId(50), false, limit)[..3],
    &[PageId(50), PageId(49), PageId(51)]
  );
}

#[test]
fn requests_share_invalidation() {
  let layout = layout();
  let a = visible(&layout, 0.0)[0];
  let b = visible(&layout, layout.pages()[2].top)[0];
  let mut tiles = Tiles::default();
  let wanted = || Visible { wanted: vec![a], missing: vec![a] };
  let first = tiles.update(wanted());
  assert!(!first.complete);
  let request = first.request.unwrap();
  assert_eq!(request.keys(), &[a]);
  assert_eq!(request.urgent(), 1);
  assert!(tiles.update(wanted()).request.is_none());
  let ready = tiles.update(Visible { wanted: vec![a], missing: vec![] });
  assert!(ready.complete && ready.request.is_none());
  assert!(tiles.update(wanted()).request.is_none());
  tiles.invalidate();
  assert!(!tiles.wants(&a));
  assert!(tiles.update(wanted()).request.is_some());
  tiles.update(Visible { wanted: vec![b], missing: vec![b] });
  assert!(tiles.wants(&b) && !tiles.wants(&a));
}

#[test]
fn animation_orders_urgent_before_prefetch() {
  let layout = layout();
  let mut tiles = Tiles::default();
  let view = View {
    layout: &layout,
    scroll: layout.pages()[2].top,
    target: Some(layout.pages()[1].top),
  };
  let update =
    tiles.scene(view, frame(&layout), &mut crate::testing::empty()).unwrap();
  assert!(!update.complete);
  let request = update.request.unwrap();
  assert_eq!(request.urgent(), tiles.wanted().len());
  assert!(
    request.keys()[..request.urgent()].iter().all(|key| tiles.wants(key))
  );
  assert!(
    request.keys()[request.urgent()..].iter().all(|key| !tiles.wants(key))
  );
  let unique: std::collections::HashSet<_> = request.keys().iter().collect();
  assert_eq!(unique.len(), request.keys().len());
  for page in [1, 2] {
    assert!(tiles.wanted().iter().any(|key| key.page == PageId(page)));
  }
  let near = tiles.nearby(view, 3);
  assert_eq!(near, [PageId(1), PageId(0), PageId(2)]);
  tiles.invalidate();
  assert_eq!(tiles.nearby(view, 3), near);
  let stopped = View { scroll: layout.pages()[1].top, target: None, ..view };
  tiles.scene(stopped, frame(&layout), &mut crate::testing::empty()).unwrap();
  assert_eq!(tiles.nearby(stopped, 3), near);
}

#[test]
fn failed_placement_retains_requests() {
  struct Broken;
  impl<'a> vpdf_model::scene::Draw<'a> for Broken {
    fn push(&mut self, _: vpdf_model::scene::Op<'a>) {}
  }
  impl Surface<'_> for Broken {
    type Error = &'static str;
    fn has(&mut self, _: TileId) -> Result<bool, Self::Error> {
      Err("Readiness unavailable")
    }
  }
  let layout = layout();
  let previous = visible(&layout, layout.pages()[2].top)[0];
  let mut tiles = Tiles::default();
  tiles.update(Visible { wanted: vec![previous], missing: vec![previous] });
  let view = View { layout: &layout, scroll: 0.0, target: None };
  assert_eq!(
    tiles.scene(view, frame(&layout), &mut Broken).err(),
    Some("Readiness unavailable")
  );
  assert_eq!(tiles.wanted(), &[previous]);
}

#[test]
fn recorded_tiles_use_owner_residency() {
  use vpdf_model::scene::{Op, Recorder, Stage};
  let layout = layout();
  let view = View { layout: &layout, scroll: 0.0, target: None };
  let wanted = visible(&layout, 0.0);
  let ready = TileId { owner: TileOwner::new(0), key: wanted[0] };
  for pane in [0, 1] {
    let mut tiles = Tiles::default();
    let mut recorder = Recorder::new(|id| id == ready);
    let frame = Frame { pane, ..frame(&layout) };
    let update = tiles.scene(view, frame, &mut recorder).unwrap();
    assert!(!update.complete);
    let request = update.request.unwrap();
    let missing = wanted.len() - usize::from(pane == 0);
    assert_eq!(request.urgent(), missing);
    assert_eq!(tiles.wanted(), wanted);
    assert!(!request.keys()[..missing].contains(&ready.key) || pane != 0);
    let scene = recorder.into_scene();
    assert_eq!(scene.len(), wanted.len());
    for (op, key) in scene.iter().zip(&wanted) {
      let Op::Tile(placement) = op else { panic!("Expected tile placement") };
      assert_eq!(
        placement.id(),
        TileId { owner: TileOwner::new(pane), key: *key }
      );
      let (x, y) = layout.origin(key.page, 0.0);
      assert_eq!(placement.origin(), (x + key.x as f32, y + key.y as f32));
      let stage = Stage { screen: layout.screen(), left: 4 };
      assert_eq!(placement.stage(), stage);
    }
    let mut all_ready = Recorder::new(|_| true);
    let update = tiles.scene(view, frame, &mut all_ready).unwrap();
    assert!(update.complete && update.request.is_none());
  }
}
