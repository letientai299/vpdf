use crate::presentation::*;
use anyhow::Result;
use vpdf_model::style::Role;
use vpdf_terminal::style::sgr;
use vpdf_ui::chrome::Notice;

#[test]
fn unmeasured_resize_shows_on_the_last_row() -> Result<()> {
  let error = vpdf_terminal::terminal::NO_CELL.to_owned();
  let unmeasured = Notice::new(10, 12, error);
  let mut out = Vec::new();
  unmeasured.draw(&mut out)?;
  let want = format!(
    "\x1b7{}\x1b[10;1HTerminal rep{}\x1b8",
    sgr(Role::Error),
    sgr(Role::Reset)
  );
  assert_eq!(String::from_utf8(out)?, want);
  Ok(())
}
