use super::*;
use crate::find::Search;
use crate::text::PageId;
use tree::Tree;
use vpdf_model::document::Heading;
use vpdf_model::geometry::At;

fn outline() -> (Tree, Panel) {
  let headings: Vec<_> = [0, 1, 2, 0, 1, 0, 1, 0, 0, 0, 0, 0]
    .into_iter()
    .enumerate()
    .map(|(source, depth)| Heading {
      title: format!("Entry {source} with a long title"),
      depth,
      target: Target::Page(PageId(source)),
    })
    .collect();
  let rows =
    headings.iter().map(|heading| Row::outline(heading, "iv".into())).collect();
  let tree = Tree::new(&headings);
  let mut panel = Panel::new(PanelKind::Outline, rows);
  panel.project(tree.project(|source| [0, 3, 5].contains(&source)));
  (tree, panel)
}

#[test]
fn projected_navigation() {
  let (_, mut panel) = outline();
  assert_eq!(panel.rows().len(), 12);
  for (action, count, explicit, want) in [
    (Action::Motion(Motion::Down), 1, false, 3),
    (Action::Motion(Motion::Down), 2, false, 7),
    (Action::Motion(Motion::Up), 1, false, 5),
    (Action::Motion(Motion::Last), 3, true, 5),
    (Action::Motion(Motion::First), 2, true, 3),
    (Action::Motion(Motion::Last), usize::MAX, true, 11),
    (Action::Motion(Motion::First), 1, false, 0),
    (Action::Screen(1), 1, false, 8),
    (Action::Rows(-1), 1, false, 7),
  ] {
    assert!(
      panel.navigate(Input { count, explicit, ..Input::plain(action) }, 8)
    );
    assert_eq!(panel.selected(), want);
  }
}

#[test]
fn projected_actions() {
  let (_, mut panel) = outline();
  panel.scene_list(&mut Vec::new(), At::default(), 20, 6);
  assert_eq!(panel.row_at(2, 2), Some(3));
  assert_eq!(panel.row_at(19, 2), None);
  assert_eq!(panel.row_at(2, 5), None);
  let click = MouseEvent::new(MouseEventKind::Down(MouseButton::Left), 2, 2);
  let press = panel.press(click);
  assert_eq!(
    press.activate(&mut panel, 1),
    Activation::Follow { focus: false }
  );
  assert_eq!(panel.selected(), 3);
  assert_eq!(panel.yank_text().as_deref(), Some("Entry 3 with a long title\n"));
  assert!(matches!(
    panel.follow(&Search::default()),
    Some(Follow::Link { target: Target::Page(PageId(3)), .. })
  ));
  assert_eq!(panel.refocus(), Some(Follow::Focus(Focus::Content)));
  let middle = MouseEvent::new(MouseEventKind::Down(MouseButton::Middle), 2, 3);
  let press = panel.press(middle);
  assert_eq!(press.activate(&mut panel, 1), Activation::Split(5));
  panel.choose(panel.row_at(2, 4).unwrap());
  assert_eq!(panel.selected(), 7, "context menus address source rows");
  panel.point(Pointing::Row(5));
  panel.scene_list(&mut Vec::new(), At::default(), 20, 6);
  assert_eq!(panel.hover_title(), Some("Entry 5 with a long title"));
}

#[test]
fn projected_scrolling() {
  let (_, mut panel) = outline();
  panel.scene_list(&mut Vec::new(), At::default(), 30, 6);
  panel.select(8);
  assert_eq!(panel.top(), 1);
  assert_eq!(panel.row_at(2, 4), Some(8));
  panel.pointer(MouseEvent::new(MouseEventKind::ScrollDown, 2, 2));
  assert_eq!(panel.top(), 4);
  assert_eq!(panel.row_at(2, 1), Some(8));
  panel.scene_list(&mut Vec::new(), At::default(), 20, 6);
  assert_eq!(panel.top(), 4, "resize preserves the scrolled source");
  panel.select(3);
  assert_eq!(panel.top(), 1);
  assert_eq!(panel.row_at(2, 1), Some(3));
}

#[test]
fn hidden_targets_clear() {
  let (tree, mut panel) = outline();
  panel.project(tree.project(|_| false));
  panel.choose(2);
  panel.clicked = Some(2);
  panel.point(Pointing::Row(2));
  panel.hover_title = Some("old title".into());
  panel.project(tree.project(|source| source == 0));
  assert_eq!(panel.selected(), 0);
  assert_eq!(panel.clicked, None);
  assert_eq!(panel.pointing, Pointing::Away);
  assert_eq!(panel.hover_title(), None);
}

#[test]
fn projected_search() {
  let (_, mut panel) = outline();
  panel.begin_search(false);
  panel.preview("Entry 2 with");
  assert_eq!(panel.selected(), 2);
  assert_eq!(panel.count(), Some((1, 1)));
  assert!(matches!(
    panel.follow(&Search::default()),
    Some(Follow::Link { target: Target::Page(PageId(2)), .. })
  ));
  panel.abort_search();
  assert_eq!(panel.selected(), 0);
  panel.begin_search(false);
  panel.preview("iv");
  assert_eq!(panel.count(), Some((2, 12)));
}

#[test]
fn outline_generation_reset() {
  let (_, mut panel) = outline();
  panel.choose(5);
  panel.clicked = Some(5);
  panel.point(Pointing::Row(5));
  panel.begin_search(true);
  panel.preview("iv");
  panel.top = 4;
  panel.focus(false);
  let rows = panel.rows.clone();
  assert!(!panel.refill(rows, "inferred"));
  assert_eq!((panel.selected(), panel.top()), (0, 0));
  assert_eq!((panel.clicked, panel.pointing), (None, Pointing::Away));
  assert!(!panel.searching());
  assert_eq!(panel.query(), "");
  assert!(!panel.focused());
  assert_eq!(panel.note(), "inferred");
  panel.abort_search();
  assert_eq!(panel.selected(), 0);
  panel.navigate(Input::plain(Action::Motion(Motion::Down)), 6);
  assert_eq!(panel.selected(), 1, "old projection was discarded");
}

#[test]
fn wrapped_projection() {
  let (tree, mut panel) = outline();
  panel.kind = PanelKind::Jumps;
  panel.rows[3].label = "one two three four five six seven eight".into();
  panel.choose(3);
  panel.reflow(12);
  let start = panel.line_start(3);
  let end = panel.line_start(4);
  assert!(end - start > 1);
  for line in start..end {
    assert_eq!(panel.layout.as_ref().unwrap().lines[line].source, 3);
  }
  assert_eq!(panel.line_start(1), start);
  assert_eq!(panel.line_start(2), start);
  panel.project(tree.project(|_| false));
  panel.reflow(12);
  assert_eq!(
    panel.layout.as_ref().unwrap().lines[panel.line_start(2)].source,
    2
  );
}

#[test]
fn empty_projection() {
  let mut panel = Panel::new(PanelKind::Outline, Vec::new());
  panel.project(Tree::default().project(|_| false));
  panel.scene_list(&mut Vec::new(), At::default(), 20, 6);
  panel.navigate(Input::plain(Action::Motion(Motion::Down)), 6);
  assert_eq!(panel.row_at(2, 1), None);
  assert_eq!(panel.follow(&Search::default()), None);
  assert_eq!(panel.yank_text(), None);
}

#[test]
#[should_panic(expected = "assertion `left == right` failed")]
fn mismatched_projection() {
  let (_, mut panel) = outline();
  panel.project(tree::Projection::all(1));
}
