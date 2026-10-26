use super::*;
use crate::keys::Fold;
use crate::reading::Outline;
use std::collections::BTreeMap;
use vpdf_model::document::Heading;
use vpdf_model::geometry::At;
use vpdf_model::scene::Op;
use vpdf_model::style::Role;
use vpdf_model::text::PageId;

fn setup() -> (Outline, Panel) {
  let headings: Vec<_> = [0, 2, 40, 0]
    .into_iter()
    .enumerate()
    .map(|(source, depth)| Heading {
      title: format!("Chapter {source} 世界 e\u{301} with a long title"),
      depth,
      target: Target::Page(PageId(source)),
    })
    .collect();
  let mut panel = Panel::new(
    PanelKind::Outline,
    headings.iter().map(|heading| Row::outline(heading, "iv".into())).collect(),
  );
  let outline = Outline::new(&headings);
  outline.project(&mut panel);
  (outline, panel)
}

fn draw(panel: &mut Panel, width: u16) -> Vec<Op<'static>> {
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), width, 8);
  scene
}

fn lines(scene: &[Op<'_>], width: u16) -> BTreeMap<usize, String> {
  let mut lines = BTreeMap::<usize, String>::new();
  let (mut column, mut row) = (0, 0);
  for op in scene {
    if let Op::Move { column: x, row: y } = op {
      (column, row) = (*x, *y);
      continue;
    }
    let text = crate::testing::text(std::slice::from_ref(op));
    if text.is_empty() {
      continue;
    }
    assert!(column + text.width() <= usize::from(width), "row {row}: {text:?}");
    if column == 0 {
      lines.insert(row, String::new());
    }
    lines.entry(row).or_default().push_str(&text);
    column += text.width();
  }
  lines
}

fn activate(
  panel: &mut Panel,
  column: u16,
  row: u16,
  button: MouseButton,
  clicks: u8,
) -> Activation {
  let press =
    panel.press(MouseEvent::new(MouseEventKind::Down(button), column, row));
  press.activate(panel, clicks)
}

#[test]
fn width_budgets_and_labels() {
  for width in [8, 21, 22, 24, 27, 28, 30, 40] {
    let (mut outline, mut panel) = setup();
    let expanded = draw(&mut panel, width);
    let expanded = lines(&expanded, width);
    assert!(expanded[&1].contains("▾"));
    assert!(expanded[&2].contains("▾"));
    assert!(expanded[&3].contains('…'), "deep title at {width}");
    assert!(expanded[&3].ends_with(" iv"));
    outline.fold(&mut panel, Fold::CollapseAll, 1);
    let scene = draw(&mut panel, width);
    let lines = lines(&scene, width);
    assert_eq!(lines[&0].contains("▾▾"), width >= 22);
    assert_eq!(lines[&1].contains("+2"), width >= 28);
    assert!(lines[&1].contains("▸"));
    assert!(lines[&1].ends_with(" iv"));
    assert_eq!(panel.rows.len(), 4);
    for row in [1, 2] {
      assert!(lines[&row].width() < usize::from(width));
    }
  }
  let (_, mut panel) = setup();
  panel.rows[0].detail = "a page label longer than this panel".into();
  let narrow = draw(&mut panel, 8);
  let narrow = lines(&narrow, 8);
  assert!(narrow[&1].contains('…'));
  assert!(!narrow[&1].contains("longer"));
}

#[test]
fn chevron_edges_and_click_history() {
  let (mut outline, mut panel) = setup();
  draw(&mut panel, 30);
  for column in [0, 3, 28] {
    assert_eq!(panel.control_at(column, 1), None);
  }
  for clicks in [1, 2, 3] {
    for column in [1, 2] {
      assert_eq!(
        activate(&mut panel, column, 1, MouseButton::Left, clicks),
        Activation::Control(Control::Branch(0))
      );
      assert_eq!(panel.clicked, None);
      assert!(panel.focused());
    }
  }
  outline.fold(&mut panel, Fold::Close, 1);
  draw(&mut panel, 30);
  assert_eq!(panel.row_at(1, 2), Some(3));
  for column in [1, 2] {
    assert_eq!(panel.control_at(column, 2), None, "leaf follows");
    assert_eq!(
      activate(&mut panel, column, 2, MouseButton::Left, 1),
      Activation::Follow { focus: false }
    );
  }
  assert_eq!(panel.selected(), 3);
  assert_eq!(
    activate(&mut panel, 1, 1, MouseButton::Middle, 1),
    Activation::Split(0)
  );
  assert_eq!(
    activate(&mut panel, 1, 1, MouseButton::Right, 1),
    Activation::Context
  );
  assert_eq!(
    activate(&mut panel, 3, 1, MouseButton::Left, 2),
    Activation::Follow { focus: false }
  );
  assert_eq!(
    activate(&mut panel, 3, 1, MouseButton::Left, 2),
    Activation::Refocus
  );
}

#[test]
fn controls_and_stale_targets() {
  let (mut outline, mut panel) = setup();
  draw(&mut panel, 30);
  let mut controls = Vec::new();
  for column in 0..29 {
    if let Some(control) = panel.control_at(column, 0) {
      assert_eq!(
        activate(&mut panel, column, 0, MouseButton::Left, 3),
        Activation::Control(control)
      );
      controls.push(control);
    }
  }
  assert_eq!(
    controls,
    [
      Control::ExpandAll,
      Control::ExpandAll,
      Control::CollapseAll,
      Control::CollapseAll,
      Control::AutoExpand
    ]
  );
  assert_eq!(panel.control_at(29, 0), None);
  panel.point(Pointing::Control(Control::Branch(1)));
  panel.clicked = Some(1);
  panel.choose(0);
  outline.fold(&mut panel, Fold::Close, 1);
  assert_eq!(panel.pointing, Pointing::Away);
  assert_eq!(panel.clicked, None);
  assert_eq!(panel.control_at(5, 2), None);
  draw(&mut panel, 21);
  assert!((0..21).all(|column| panel.control_at(column, 0).is_none()));
  draw(&mut panel, 30);
  panel.pointer(MouseEvent::new(MouseEventKind::ScrollDown, 1, 1));
  assert!(outline.closed(0));
}

#[test]
fn marker_selection_and_plain_text() {
  let (outline, mut panel) = setup();
  panel.choose(3);
  panel.mark_outline(outline.tree(), Some(0));
  panel.point(Pointing::Row(1));
  let scene = draw(&mut panel, 28);
  let spans = crate::testing::spans(&scene);
  assert!(spans.contains(&(Role::Heading, "▌".into())));
  assert!(scene.contains(&Op::Style(Role::Selected)));
  assert!(lines(&scene, 28)[&4].contains("Chapter 3"));
  assert_eq!(panel.selected(), 3);
  assert_eq!(
    panel.hover_title(),
    Some("Chapter 1 世界 e\u{301} with a long title")
  );
  assert_eq!(
    panel.yank_text(),
    Some("Chapter 3 世界 e\u{301} with a long title\n".into())
  );
  let top = panel.top();
  panel.mark_outline(outline.tree(), Some(1));
  assert_eq!((panel.selected(), panel.top()), (3, top));
  panel.begin_search(false);
  panel.preview("▾");
  assert_eq!(panel.count(), None);
}

#[test]
fn auto_control_menu_and_action() {
  let (mut outline, mut panel) = setup();
  assert_eq!(
    panel.control_action(Control::AutoExpand),
    Some(Action::OutlineAuto)
  );
  for enabled in [true, false] {
    outline.toggle_auto(&mut panel);
    assert_eq!(panel.auto_expand(), enabled);
    let scene = draw(&mut panel, 30);
    assert!(crate::testing::text(&scene).contains(if enabled {
      "●"
    } else {
      "○"
    }));
    assert_eq!(
      panel.control_tip(Control::AutoExpand).as_deref(),
      Some(if enabled { "Auto-expand: on" } else { "Auto-expand: off" })
    );
    let items = crate::context::outline_items(&panel, None);
    assert_eq!(items.last().unwrap().label.starts_with('✓'), enabled);
    assert_eq!(
      panel.input(Input::plain(Action::OutlineAuto), 8),
      Event::AutoExpand
    );
  }
  outline.toggle_auto(&mut panel);
  outline.fold(&mut panel, Fold::Open, 1);
  assert!(!panel.auto_expand());
  panel.focus(false);
  assert_eq!(
    panel.input(Input::plain(Action::OutlineAuto), 8),
    Event::Unhandled
  );
  let mut other = Panel::new(PanelKind::Results, vec![]);
  assert_eq!(other.input(Input::plain(Action::OutlineAuto), 8), Event::Stay);
}

#[test]
fn tiny_panes_clear_controls() {
  let (_, mut panel) = setup();
  for width in [0, 1, 2, 3, 4] {
    draw(&mut panel, 30);
    let scene = draw(&mut panel, width);
    lines(&scene, width);
    assert!((0..30).all(|column| panel.control_at(column, 0).is_none()));
    if width < 4 {
      assert_eq!(panel.row_at(0, 1), None);
    }
  }
}

#[test]
fn chevron_keeps_drawn_source() {
  let (mut outline, mut panel) = setup();
  panel.scene_list(&mut Vec::new(), At::default(), 30, 3);
  panel.pointer(MouseEvent::new(MouseEventKind::ScrollDown, 1, 1));
  assert_eq!(panel.top(), 3);
  assert_eq!(
    activate(&mut panel, 1, 1, MouseButton::Left, 1),
    Activation::Control(Control::Branch(0))
  );
  assert_eq!(panel.selected(), 0);
  outline.fold(&mut panel, Fold::Close, 1);
  assert!(outline.closed(0));
}

#[test]
fn empty_contents_explains_state() {
  let outline = Outline::default();
  let mut panel = Panel::new(PanelKind::Outline, vec![]);
  outline.project(&mut panel);
  for (note, message) in [
    ("loading", "Preparing contents…"),
    ("", "No headings. Use page jumps or search."),
  ] {
    panel.set_note(note);
    for width in [2, 8, 21, 40, 50] {
      let scene = draw(&mut panel, width);
      let rows = lines(&scene, width);
      assert_eq!(
        rows[&1],
        vpdf_model::text::clean(message, usize::from(width - 1))
      );
      assert_eq!(panel.row_at(0, 1), None);
    }
  }
  panel.scene_list(&mut Vec::new(), At::default(), 40, 1);
  assert_eq!(panel.row_at(0, 1), None);
}
