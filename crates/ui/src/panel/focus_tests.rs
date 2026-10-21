use super::*;
#[cfg(test)]
use anyhow::Result;

use vpdf_model::input::KeyModifiers;

#[test]
fn wheel_moves_viewport_and_keys_reveal_rows() -> Result<()> {
  let mut panel = Panel::new(
    PanelKind::Help,
    (0..80).map(|row| Row::parse(&format!("row {row}"))).collect(),
  );
  panel.focused = false;
  panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 40, 12)?;
  let wheel = MouseEvent {
    kind: MouseEventKind::ScrollDown,
    column: 2,
    row: 4,
    modifiers: KeyModifiers::NONE,
  };
  panel.pointer(wheel);
  panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 40, 12)?;
  assert_eq!((panel.selected, panel.top, panel.focused), (0, 3, false));
  panel.selected = 2;
  panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 40, 12)?;
  assert_eq!(panel.top, 3, "tracking does not undo wheel scrolling");
  panel.navigate(Input::plain(Action::Motion(Motion::Down)), 12);
  assert_eq!((panel.selected, panel.top), (3, 3));
  panel.selected = 0;
  panel.begin_search(false);
  panel.preview("row 40");
  assert_eq!(panel.selected, 40);
  assert_eq!(panel.top, 31, "search reveals the matching row");
  Ok(())
}
