use crate::presentation::*;
use anyhow::Result;
use vpdf_model::style::Role;
use vpdf_terminal::style::sgr;
use vpdf_ui::split::*;

#[test]
fn draws_only_column_dividers() -> Result<()> {
  let mut tree = Node::leaf(1);
  tree.split(1, Axis::Rows, 2, Place::After);
  tree.split(2, Axis::Columns, 3, Place::After);
  let area = Rect { column: 3, row: 2, columns: 21, rows: 8 };
  let mut out = Vec::new();
  draw(&mut out, None, |scene| tree.scene_dividers(scene, area))?;
  let want = [7, 8, 9, 10]
    .map(|row| {
      format!("\x1b[{row};14H{}│{}", sgr(Role::Muted), sgr(Role::Reset))
    })
    .concat();
  assert_eq!(String::from_utf8(out)?, want);
  let mut out = Vec::new();
  draw(&mut out, None, |scene| {
    tree.scene_dividers(scene, Rect { rows: 0, ..area });
  })?;
  assert_eq!(out, [] as [u8; 0]);
  Ok(())
}
