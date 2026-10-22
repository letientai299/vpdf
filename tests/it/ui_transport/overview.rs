#[path = "../../support/overview.rs"]
mod fixture;
use crate::presentation::*;
use anyhow::Result;
use fixture::Source;
use vpdf_model::scene::Op;
use vpdf_model::text::PageId;
use vpdf_model::tile::TileKey;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::overview::*;

#[test]
fn recorded_overview_keeps_tiles_and_chrome() -> Result<()> {
  use vpdf_model::scene::Recorder;
  let frame = fixture::frame();
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
    assert_eq!(replay_frame(&mut replay, &scene)?, expected);
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
