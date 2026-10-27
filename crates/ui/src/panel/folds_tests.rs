use super::*;
use crate::keys::Fold;

fn grouped(kind: PanelKind) -> Panel {
  let depths = [0, 1, 1, 0, 1, 1];
  let rows = depths
    .into_iter()
    .enumerate()
    .map(|(line, depth)| Row {
      depth,
      ..Row::new(
        format!("row {line}"),
        String::new(),
        Destination::Position(Position { line, ..Position::default() }),
      )
    })
    .collect();
  let mut panel = Panel::new(kind, rows);
  panel.group(&[]);
  panel
}

#[test]
fn group_folds() {
  type Case = (usize, &'static [(Fold, usize)], usize, &'static [usize]);
  let all: &[usize] = &[0, 1, 2, 3, 4, 5];
  let cases: &[Case] = &[
    (1, &[(Fold::Close, 1)], 0, &[0, 1, 2, 3, 4, 5]),
    (1, &[(Fold::Close, 2)], 0, &[0, 3, 4, 5]),
    (4, &[(Fold::Close, 2)], 3, &[0, 1, 2, 3]),
    (0, &[(Fold::Open, 1)], 1, all),
    (2, &[(Fold::Open, 3)], 2, all),
    (0, &[(Fold::Close, 1), (Fold::Open, 1)], 0, all),
    (4, &[(Fold::CollapseAll, 1)], 3, &[0, 3]),
    (4, &[(Fold::CollapseAll, 1), (Fold::Open, 1)], 3, &[0, 3, 4, 5]),
    (0, &[(Fold::CollapseAll, 1), (Fold::ExpandAll, 1)], 0, all),
  ];
  for &(start, operations, selected, visible) in cases {
    let mut panel = grouped(PanelKind::Results);
    panel.choose(start);
    for &(operation, count) in operations {
      let input = Input { count, ..Input::plain(Action::Fold(operation)) };
      assert_eq!(panel.input(input, 20), Event::Fold { operation, count });
      assert!(panel.fold(operation, count));
    }
    let case = format!("{start} {operations:?}");
    assert_eq!(panel.selected(), selected, "{case}");
    assert_eq!(panel.projection.sources(), visible, "{case}");
    let folds = panel.folds.as_ref().unwrap();
    for &source in visible {
      assert_eq!(folds.visible(source), Some(source), "{case}");
    }
  }
}

#[test]
fn group_chevrons() {
  let mut panel = grouped(PanelKind::Annotations);
  let close = Some(Action::Fold(Fold::Close));
  assert_eq!(panel.control_action(Control::Branch(3)), close);
  panel.choose(3);
  panel.fold(Fold::Close, 1);
  let open = Some(Action::Fold(Fold::Open));
  assert_eq!(panel.control_action(Control::Branch(3)), open);
  assert_eq!(panel.control_action(Control::Branch(4)), None);
}

#[test]
fn groups_stay_with_their_rows() {
  let mut panel = grouped(PanelKind::Results);
  assert!(panel.foldable());
  panel.fold(Fold::CollapseAll, 1);
  panel.refill(vec![], "");
  assert!(!panel.foldable());
  assert!(!panel.fold(Fold::ExpandAll, 1));
  assert_eq!(
    panel.input(Input::plain(Action::Fold(Fold::ExpandAll)), 20),
    Event::Stay
  );
}

fn paint(panel: &mut Panel, width: u16) -> Vec<vpdf_model::scene::Op<'static>> {
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, vpdf_model::geometry::At::default(), width, 8);
  scene
}

#[test]
fn group_headers_and_badges() {
  for kind in [PanelKind::Results, PanelKind::Annotations, PanelKind::Marks] {
    for width in [21, 22, 28] {
      let mut panel = grouped(kind);
      panel.fold(Fold::CollapseAll, 1);
      let scene = paint(&mut panel, width);
      let text = crate::testing::text(&scene);
      assert_eq!(text.contains("▾▾"), width >= 22);
      assert_eq!(text.contains("+2"), width >= 28);
      assert!(!text.contains(['●', '○']));
      let quiet = crate::testing::quiet_on_selected(&scene);
      assert!(quiet.is_empty(), "{quiet:?}");
      let controls: Vec<_> =
        (0..width).filter_map(|column| panel.control_at(column, 0)).collect();
      assert_eq!(
        controls,
        if width >= 22 {
          vec![
            Control::ExpandAll,
            Control::ExpandAll,
            Control::CollapseAll,
            Control::CollapseAll,
          ]
        } else {
          vec![]
        }
      );
      assert_eq!(panel.control_action(Control::AutoExpand), None);
      assert_eq!(panel.control_tip(Control::AutoExpand), None);
      panel.point(Pointing::Control(Control::ExpandAll));
      let scene = paint(&mut panel, width);
      assert_eq!(
        scene.contains(&vpdf_model::scene::Op::Style(
          vpdf_model::style::Role::BarHover
        )),
        width >= 22
      );
    }
  }
}

#[test]
fn group_title_clicks() {
  for followable in [false, true] {
    let mut panel = grouped(PanelKind::Annotations);
    panel.rows[0].style = Style::Heading;
    if !followable {
      panel.rows[0].destination = Destination::None;
    }
    paint(&mut panel, 28);
    let press = panel.press(MouseEvent::new(
      MouseEventKind::Down(MouseButton::Left),
      5,
      1,
    ));
    let activation = press.activate(&mut panel, 1);
    assert_eq!(
      activation,
      if followable {
        Activation::Follow { focus: false }
      } else {
        Activation::Control(Control::Branch(0))
      }
    );
    if !followable {
      let Activation::Control(control) = activation else { unreachable!() };
      let Some(Action::Fold(operation)) = panel.control_action(control) else {
        unreachable!()
      };
      panel.fold(operation, 1);
      assert_eq!(panel.projection.sources(), &[0, 3, 4, 5]);
      assert_eq!(panel.clicked, None);
    }
  }
}

#[test]
fn group_menus() {
  for kind in [PanelKind::Results, PanelKind::Annotations, PanelKind::Marks] {
    let mut panel = grouped(kind);
    for closed in [false, true] {
      let items = crate::context::fold_items(&panel, Some(closed), "group");
      let labels: Vec<_> =
        items.iter().map(|item| item.label.as_ref()).collect();
      assert_eq!(
        labels,
        [
          if closed { "Expand group" } else { "Collapse group" },
          "Expand all",
          "Collapse all"
        ]
      );
    }
    assert_eq!(crate::context::fold_items(&panel, None, "group").len(), 2);
    panel.fold(Fold::CollapseAll, 1);
    assert_eq!(
      panel.control_tip(Control::Branch(0)).as_deref(),
      Some("Expand group")
    );
  }
}

#[test]
fn group_search_reveals() {
  let mut panel = grouped(PanelKind::Results);
  panel.resize(8);
  panel.fold(Fold::CollapseAll, 1);
  panel.begin_search(false);
  assert!(panel.preview("row 2"));
  assert_eq!(panel.selected(), 2);
  assert!(panel.is_visible(2));
  assert!(!panel.is_visible(4));
  assert!(panel.folds.as_ref().unwrap().closed(0));
  assert!(!panel.preview("missing"));
  assert_eq!(panel.projection.sources(), &[0, 3]);
  assert!(panel.preview("row 4"));
  panel.abort_search();
  assert_eq!(panel.selected(), 0);
  assert_eq!(panel.projection.sources(), &[0, 3]);
  panel.begin_search(false);
  assert!(panel.finish_search("row 2"));
  assert_eq!(panel.selected(), 2);
  assert!(panel.is_visible(2));
  assert!(!panel.folds.as_ref().unwrap().closed(0));
  panel.begin_search(false);
  assert!(panel.finish_search("row"));
  panel.fold(Fold::CollapseAll, 1);
  for reverse in [false, true] {
    assert!(panel.repeat(reverse, 2));
    assert!(panel.is_visible(panel.selected()));
  }
}

#[test]
fn group_heading_steps() {
  let mut panel = grouped(PanelKind::Marks);
  for operation in [Fold::ExpandAll, Fold::CollapseAll] {
    panel.fold(operation, 1);
    panel.choose(0);
    panel.heading(true, 1);
    assert_eq!(panel.selected(), 3);
    panel.heading(false, 1);
    assert_eq!(panel.selected(), 0);
  }
}

#[test]
fn group_note_menu() {
  let mut panel = grouped(PanelKind::Annotations);
  panel.rows[0].destination = Destination::None;
  panel.rows[1].destination = Destination::Annotation {
    position: Position::default(),
    annotation: Annotation {
      kind: "Note".into(),
      areas: vec![],
      contents: "A note".into(),
      author: String::new(),
      color: None,
    },
  };
  for (source, expected) in [(0, false), (1, true), (2, false)] {
    panel.choose(source);
    let items = crate::context::panel_items(
      &panel,
      Some(&panel.rows[source].destination),
      false,
    );
    assert_eq!(
      items.iter().any(|item| item.label == "Show note"),
      expected,
      "row {source}"
    );
  }
}
