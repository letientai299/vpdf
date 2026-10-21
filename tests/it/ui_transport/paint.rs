use vpdf_model::geometry::At;
use vpdf_model::scene::Scene;
use vpdf_ui::paint::*;

#[test]
fn overflowing_origins_return_errors() {
  let mut scene = Scene::new();
  move_to(&mut scene, At { column: 1, row: 1 }, usize::MAX, usize::MAX);
  let mut out = Vec::new();
  let error = vpdf_terminal::painter::paint(&mut out, &scene).unwrap_err();
  assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
  assert_eq!(out, [] as [u8; 0]);
}

#[test]
fn scene_lines_keep_width_and_origin() -> anyhow::Result<()> {
  let mut scene = Scene::new();
  line(&mut scene, At { column: 3, row: 2 }, 2, "界e\u{301}\x07", 4);
  let mut out = Vec::new();
  vpdf_terminal::painter::paint(&mut out, &scene)?;
  assert_eq!(out, "\x1b[5;4H界e\u{301} ".as_bytes());
  Ok(())
}
