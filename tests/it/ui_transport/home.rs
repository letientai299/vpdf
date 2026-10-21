use crate::presentation::HomeDraw;
use anyhow::Result;
use vpdf_model::geometry::{At, Screen};
use vpdf_model::style::Role;
use vpdf_terminal::{graphics::Graphics, style::sgr};
use vpdf_ui::home::Home;
#[test]
fn panel_ends_at_the_last_column() -> Result<()> {
  let screen = Screen {
    columns: 40,
    rows: 10,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut out = Vec::new();
  Home::new(Vec::new(), &[], screen).draw(
    &mut out,
    &mut Graphics::default(),
    0,
    |_| {},
  )?;
  let out = String::from_utf8(out)?;
  assert!(out.contains(" Vpdf "));
  let columns = out.split("\x1b[").filter_map(|code| {
    let (row, column) = code.split_once('H')?.0.split_once(';')?;
    row.parse::<u16>().ok()?;
    column.parse::<u16>().ok()
  });
  assert_eq!(columns.max(), Some(screen.columns));
  for row in 0..screen.rows - 1 {
    let goto =
      vpdf_terminal::terminal::goto(At::default(), row, screen.columns - 1);
    let border = format!("{goto}{}│", sgr(Role::Muted));
    assert!(out.contains(&border), "border on row {row}");
  }
  Ok(())
}
