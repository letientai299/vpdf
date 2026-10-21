use super::*;

use vpdf_model::input::KeyModifiers;

#[test]
fn scrolling_can_enter_wrapped_rows() {
  let rows = ["# Move", "j: one two three four five six seven eight nine ten"]
    .map(Row::parse)
    .into();
  let mut panel = Panel::new(PanelKind::Help, rows);
  panel.scene_list(&mut Vec::new(), vpdf_model::geometry::At::default(), 12, 4);
  panel.selected = 1;
  for top in 1..=3 {
    panel.navigate(Input::plain(Action::Rows(1)), 4);
    panel.scene_list(
      &mut Vec::new(),
      vpdf_model::geometry::At::default(),
      12,
      4,
    );
    assert_eq!((panel.top, panel.selected), (top, 1));
  }
}

#[test]
fn wrapped_rows_keep_their_source() {
  let rows =
    ["# Move", "j: one two three four five", "k: up"].map(Row::parse).into();
  let mut panel = Panel::new(PanelKind::Help, rows);
  panel.scene_list(&mut Vec::new(), vpdf_model::geometry::At::default(), 15, 6);
  let press = |row| MouseEvent {
    kind: MouseEventKind::Down(MouseButton::Left),
    column: 5,
    row,
    modifiers: KeyModifiers::NONE,
  };
  assert_eq!(panel.pointer(press(3)), Some(1));
  assert_eq!(panel.pointer(press(4)), Some(1));
  panel.begin_search(false);
  panel.preview("three four");
  assert_eq!(panel.selected, 1);
  assert_eq!(panel.count(), Some((1, 1)));
  assert_eq!(panel.rows[panel.selected].label, "j: one two three four five");
  panel.finish_search("three four");
  panel.selected = 2;
  panel.reveal();
  assert_eq!(panel.top, 1);
  panel.scene_list(&mut Vec::new(), vpdf_model::geometry::At::default(), 40, 6);
  assert_eq!(panel.selected, 2);
  assert_eq!(panel.top, 0);
  assert_eq!(panel.pointer(press(3)), Some(2));
  assert_eq!(panel.pointer(press(4)), None);
}
