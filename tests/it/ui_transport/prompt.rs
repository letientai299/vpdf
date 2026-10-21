use crate::presentation::*;
use anyhow::Result;
use vpdf_model::geometry::{At, Screen};
use vpdf_ui::prompt::{Prompt, PromptKind};

#[test]
fn drawing_positions_the_wide_text_cursor() -> Result<()> {
  let mut prompt = Prompt::new(PromptKind::Command);
  prompt.replace("界a");
  let screen = Screen {
    columns: 10,
    rows: 2,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let at = At { column: 7, row: 3 };
  let mut out = Vec::new();
  prompt.draw(&mut out, screen, at, None)?;
  let want = concat!(
    "\x1b[5;8H:界a\x1b[90m\x1b[0m      ",
    "\x1b[5;12H\x1b[2 q\x1b[?25h"
  );
  assert_eq!(out, want.as_bytes());
  let mut scene = vpdf_model::scene::Scene::new();
  prompt.scene(&mut scene, screen, at, None);
  out.clear();
  vpdf_terminal::painter::paint(&mut out, &scene)?;
  assert_eq!(out, want.as_bytes());
  for screen in [Screen { rows: 0, ..screen }, Screen { columns: 0, ..screen }]
  {
    out.clear();
    prompt.draw(&mut out, screen, at, None)?;
    assert_eq!(out, [] as [u8; 0]);
  }
  Ok(())
}
