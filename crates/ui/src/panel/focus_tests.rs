use super::*;

#[test]
fn wheel_moves_viewport_and_keys_reveal_rows() {
  let mut panel = Panel::new(
    PanelKind::Version,
    (0..80).map(|row| Row::parse(&format!("row {row}"))).collect(),
  );
  panel.focused = false;
  panel.scene_list(
    &mut Vec::new(),
    vpdf_model::geometry::At::default(),
    40,
    12,
  );
  let wheel = MouseEvent::new(MouseEventKind::ScrollDown, 2, 4);
  panel.pointer(wheel);
  panel.scene_list(
    &mut Vec::new(),
    vpdf_model::geometry::At::default(),
    40,
    12,
  );
  assert_eq!((panel.selected, panel.top, panel.focused), (0, 3, false));
  panel.selected = 2;
  panel.scene_list(
    &mut Vec::new(),
    vpdf_model::geometry::At::default(),
    40,
    12,
  );
  assert_eq!(panel.top, 3, "tracking does not undo wheel scrolling");
  panel.navigate(Input::plain(Action::Motion(Motion::Down)), 12);
  assert_eq!((panel.selected, panel.top), (3, 3));
  panel.selected = 0;
  panel.begin_search(false);
  panel.preview("row 40");
  assert_eq!(panel.selected, 40);
  assert_eq!(panel.top, 31, "search reveals the matching row");
}
