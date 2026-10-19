use super::*;
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

fn layout() -> Layout {
  let page = crate::text::Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 1000.0 };
  Layout::new(
    [page; 5],
    Screen {
      columns: 100,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (7, 3),
    },
  )
}

fn frame(layout: &Layout) -> Frame {
  Frame { pane: 0, screen: layout.screen(), left: 4 }
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
fn animation_orders_urgent_before_prefetch() -> Result<()> {
  let layout = layout();
  let mut tiles = Tiles::default();
  let view = View {
    layout: &layout,
    scroll: layout.pages()[2].top,
    target: Some(layout.pages()[1].top),
  };
  let update = tiles.draw(
    view,
    frame(&layout),
    &mut Graphics::default(),
    &mut Vec::new(),
  )?;
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
  tiles.draw(
    stopped,
    frame(&layout),
    &mut Graphics::default(),
    &mut Vec::new(),
  )?;
  assert_eq!(tiles.nearby(stopped, 3), near);
  Ok(())
}

#[test]
fn failed_placement_retains_requests() -> Result<()> {
  use vpdf_terminal::payload::{Encoder, Tile};
  struct Broken;
  impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
      Err(std::io::Error::other("Output unavailable"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
      Ok(())
    }
  }
  let layout = layout();
  let key = visible(&layout, 0.0)[0];
  let previous = visible(&layout, layout.pages()[2].top)[0];
  let mut tiles = Tiles::default();
  tiles.update(Visible { wanted: vec![previous], missing: vec![previous] });
  let mut graphics = Graphics::default();
  let pixels = vec![255; (key.width * key.height * 3) as usize];
  let tile =
    Tile { key, payload: Encoder::default().encode(&pixels)?, render_ms: 0.0 };
  graphics.upload(0, &tile, &mut std::io::sink())?;
  let view = View { layout: &layout, scroll: 0.0, target: None };
  assert!(
    tiles.draw(view, frame(&layout), &mut graphics, &mut Broken).is_err()
  );
  assert_eq!(tiles.wanted(), &[previous]);
  Ok(())
}
