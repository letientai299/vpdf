use crate::presentation::*;
use anyhow::Result;
use vpdf_model::geometry::Screen;
use vpdf_model::scene::Op;
use vpdf_model::text::{Bounds, PageId};
use vpdf_model::tile::TileKey;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::overview::*;
use vpdf_ui::source::Pages;

#[test]
fn recorded_overview_keeps_tiles_and_chrome() -> Result<()> {
  use vpdf_model::scene::Recorder;
  struct Source;
  impl Pages for Source {
    fn count(&self) -> usize {
      3
    }
    fn bounds(&self, _: PageId) -> Bounds {
      Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 }
    }
    fn label(&self, _: PageId) -> &'static str {
      "page"
    }
  }
  let screen = Screen {
    columns: 100,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (7, 3),
  };
  let frame = Frame {
    pane: 1,
    screen,
    clip: screen,
    at: vpdf_model::geometry::At { column: 7, row: 3 },
    left: 4,
    hovered: Some(PageId(2)),
    mask: None,
  };
  let mut overview = Overview::new(PageId(1));
  let mut recorder = Recorder::new(|_| false);
  let visible = overview.scene(&Source, frame, &mut recorder).unwrap();
  let expected_requests = requests(visible);
  assert_ne!(expected_requests.0, [] as [TileKey; 0]);
  let scene = recorder.into_scene();
  assert!(scene.iter().any(|op| matches!(op, Op::Tile(_))));
  assert!(scene.iter().any(|op| matches!(op, Op::Text(_))));
  assert!(scene.iter().any(|op| matches!(op, Op::Fill(_))));
  let mut graphics = Graphics::default();
  let mut replay = Graphics::default();
  for _ in 0..2 {
    let mut expected = Vec::new();
    graphics.begin(&mut expected)?;
    let direct = overview.draw(&Source, frame, &mut graphics, &mut expected)?;
    graphics.end(&mut expected)?;
    assert_eq!(requests(direct), expected_requests);
    let mut actual = Vec::new();
    replay.begin(&mut actual)?;
    let mut painter = vpdf_terminal::painter::Painter::new(&mut actual)
      .with_graphics(&mut replay);
    painter.paint(&scene);
    painter.finish()?;
    replay.end(&mut actual)?;
    assert_eq!(actual, expected);
  }
  Ok(())
}

fn requests(
  visible: vpdf_ui::layout::tiles::Visible,
) -> (Vec<TileKey>, Vec<TileKey>, usize) {
  let mut tiles = vpdf_ui::layout::tiles::Tiles::default();
  let update = tiles.update(visible);
  let request = update.request.unwrap();
  (tiles.wanted().to_vec(), request.keys().to_vec(), request.urgent())
}
