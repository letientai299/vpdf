use super::*;
use crate::bindings::{Bindings, Context, Target as Binding};
use crate::keys::Dispatch;
use crate::panel::recent::{Entry, Frame};
use vpdf_model::geometry::At;
use vpdf_model::input::{
  KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

fn entry(path: &str) -> Entry {
  Entry {
    path: path.into(),
    cursor: Position::default(),
    rotation: 0,
    used: 0,
    marks: 0,
  }
}

fn rows(entries: &[Entry]) -> Vec<Row> {
  entries
    .iter()
    .map(|entry| Row::recent(&entry.path, vpdf_model::text::PageId(0), None))
    .collect()
}

fn panel() -> Panel {
  let entries = [entry("/missing/a.pdf"), entry("/missing/b.pdf")];
  Panel::recent(rows(&entries), &entries)
}

fn action(panel: &mut Panel, action: Action) -> Event {
  panel.input(Input::plain(action), 24)
}

fn key(code: KeyCode) -> KeyEvent {
  KeyEvent::new(code, KeyModifiers::NONE)
}

fn text(panel: &mut Panel) -> String {
  crate::testing::text(&scene(panel))
}

fn scene(panel: &mut Panel) -> Vec<vpdf_model::scene::Op<'static>> {
  let mut scene = crate::testing::empty();
  panel
    .scene(
      &mut scene,
      Frame {
        at: At::default(),
        width: 100,
        screen: crate::testing::screen::cells(100, 24),
        now: 0,
      },
      false,
    )
    .unwrap();
  scene.into_scene()
}

#[test]
fn selected_checkboxes_keep_the_row_fill() {
  let mut panel = panel();
  let quiet = crate::testing::quiet_on_selected(&scene(&mut panel));
  assert!(quiet.is_empty(), "{quiet:?}");
}

#[test]
fn checkboxes_preserve_cursor_and_preview() {
  let mut panel = panel();
  let first = text(&mut panel);
  assert_eq!(first.matches("○").count(), 2);
  assert!(first.contains("0 selected"));
  panel.point(Pointing::Row(1));
  for count in [1, 0, 1] {
    assert_eq!(action(&mut panel, Action::RecentToggle), Event::Stay);
    assert_eq!(panel.selected(), 0);
    assert_eq!(panel.removal.checked.len(), count);
  }
  action(&mut panel, Action::Motion(Motion::Down));
  assert_eq!(panel.selected(), 1);
  assert_eq!(panel.removal.checked.len(), 1);
  assert_eq!(action(&mut panel, Action::Follow), Event::Follow { focus: true });
  assert_eq!(
    panel.follow(&crate::find::Search::default()),
    Some(Follow::File("/missing/b.pdf".into()))
  );
  assert_eq!(action(&mut panel, Action::Cancel), Event::Stay);
  assert!(panel.removal.checked.is_empty());
  action(&mut panel, Action::RecentToggle);
  panel.focus(false);
  assert!(panel.removal.checked.is_empty());
}

#[test]
fn confirmation_defaults_yes_and_cancels() {
  let cancel = [
    key(KeyCode::Char('n')),
    key(KeyCode::Esc),
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
  ];
  for cancel in cancel {
    let mut panel = panel();
    action(&mut panel, Action::RecentToggle);
    action(&mut panel, Action::RecentRemove);
    let before = text(&mut panel);
    assert!(before.contains("Remove \"a.pdf\" from recent?"));
    assert!(before.contains("[ Remove Y/Enter ] [ Cancel n/Esc ]"));
    assert!(
      before.contains("Saved position and local marks will be forgotten.")
    );
    assert_eq!(panel.confirm_key(cancel), Event::Stay);
    assert!(!panel.confirming());
    assert_eq!(panel.rows().len(), 2);
    assert_eq!(panel.selected(), 0);
    assert_eq!(panel.removal.checked.len(), 1);
  }
  for confirm in [KeyCode::Enter, KeyCode::Char('y')] {
    let mut panel = panel();
    action(&mut panel, Action::RecentRemove);
    assert_eq!(panel.confirm_key(key(KeyCode::Char(':'))), Event::Stay);
    assert!(panel.confirming());
    assert_eq!(
      panel.confirm_key(key(confirm)),
      Event::Forget(vec!["/missing/a.pdf".into()])
    );
  }
}

#[test]
fn batch_targets_survive_filter_and_refresh() {
  let mut panel = panel();
  action(&mut panel, Action::RecentToggle);
  action(&mut panel, Action::Motion(Motion::Down));
  action(&mut panel, Action::RecentToggle);
  panel.refill(rows(&[entry("/missing/b.pdf")]), "");
  assert!(text(&mut panel).contains("2 selected (1 hidden)"));
  action(&mut panel, Action::RecentRemove);
  assert!(text(&mut panel).contains("Remove 2 files from recent?"));
  panel.refresh_recent(
    rows(&[entry("/missing/c.pdf")]),
    &[entry("/missing/c.pdf")],
  );
  assert_eq!(
    panel.confirm_key(key(KeyCode::Enter)),
    Event::Forget(vec!["/missing/a.pdf".into(), "/missing/b.pdf".into()])
  );
  assert!(panel.removal.checked.is_empty());
}

#[test]
fn refresh_preserves_path_identity() {
  let mut panel = panel();
  action(&mut panel, Action::Motion(Motion::Down));
  action(&mut panel, Action::RecentToggle);
  let entries = [entry("/missing/b.pdf"), entry("/missing/a.pdf")];
  panel.refresh_recent(rows(&entries), &entries);
  assert_eq!(panel.selected(), 0);
  assert_eq!(
    panel.removal.checked.iter().collect::<Vec<_>>(),
    vec![&PathBuf::from("/missing/b.pdf")]
  );
  panel.refresh_recent(rows(&entries[1..]), &entries[1..]);
  assert!(panel.removal.checked.is_empty());
  assert_eq!(panel.selected(), 0);
  panel.refresh_recent(Vec::new(), &[]);
  assert!(text(&mut panel).contains(":edit <file>"));
  assert!(panel.shelf_mut().unwrap().take_request().is_none());
}

#[test]
fn headings_and_welcome_cannot_select() {
  for rows in [Vec::new(), vec![Row::parse("# Today")]] {
    let mut panel = Panel::recent(rows, &[]);
    action(&mut panel, Action::RecentToggle);
    action(&mut panel, Action::RecentRemove);
    assert!(panel.removal.checked.is_empty());
    assert!(!panel.confirming());
  }
}

#[test]
fn recent_keys_preserve_other_paging() {
  let bindings = Bindings::default();
  for (context, expected) in [
    (Context::Normal, Action::Screen(1)),
    (Context::Panel, Action::Screen(1)),
    (Context::Recent, Action::RecentToggle),
    (Context::HomeRecent, Action::RecentToggle),
  ] {
    let Some(Dispatch::Input(input)) = crate::keys::Keys::default().feed_with(
      key(KeyCode::Char(' ')),
      context,
      &bindings,
    ) else {
      panic!("missing binding")
    };
    assert_eq!(input.action, expected);
    assert_eq!(
      bindings
        .map(context)
        .get(&crate::bindings::Sequence::parse("<Space>").unwrap()),
      Some(&Binding::Reader(expected))
    );
  }
}

#[test]
fn recent_help_matches_dispatch() {
  use crate::topics::{Lookup, Registry, TopicId};
  let registry =
    Registry::new(crate::topics::authored(), Bindings::default().snapshot())
      .unwrap();
  for context in [Context::Recent, Context::HomeRecent] {
    for (key, action) in
      [("<Space>", Action::RecentToggle), ("x", Action::RecentRemove)]
    {
      let target = Binding::Reader(action);
      let Lookup::Found(destination) = registry.lookup(key, context) else {
        panic!("Missing key topic: {key}");
      };
      assert_eq!(
        destination.action(),
        Some(&TopicId::parse(&target.topic()).unwrap())
      );
    }
  }
}

fn press(column: u16, row: u16) -> MouseEvent {
  MouseEvent::new(MouseEventKind::Down(MouseButton::Left), column, row)
}

#[test]
fn confirm_buttons_answer_clicks() {
  // (column on the button row, answer)
  for (column, removed) in [(2, true), (21, false), (60, false)] {
    let mut panel = panel();
    action(&mut panel, Action::RecentRemove);
    text(&mut panel);
    let event = panel.confirm_press(press(column, 22));
    let want = if removed {
      Event::Forget(vec!["/missing/a.pdf".into()])
    } else {
      Event::Stay
    };
    assert_eq!(event, want, "{column}");
    // A miss keeps asking; Cancel stops.
    assert_eq!(panel.confirming(), column == 60, "{column}");
  }
}

fn marks() -> Panel {
  let position = Position::default();
  let local = vec![Row::local_mark('a', position, None, "1")];
  let global =
    vec![Row::global_mark('A', position, std::path::Path::new("/x/doc.pdf"))];
  let mut panel = Panel::new(PanelKind::Marks, Row::marks(local, global));
  panel.group(&[]);
  panel
}

#[test]
fn marks_confirm_names_the_mark() {
  // (row, question, detail, event)
  let cases = [
    (1, "Remove mark ’a?", None, Event::RemoveMark('a')),
    (
      3,
      "Remove global mark ’A to doc.pdf?",
      Some("Every document loses it."),
      Event::RemoveMark('A'),
    ),
  ];
  for (row, question, detail, event) in cases {
    let mut panel = marks();
    panel.choose(row);
    action(&mut panel, Action::RecentRemove);
    let shown = text(&mut panel);
    assert!(shown.contains(question), "{shown}");
    assert_eq!(
      detail.is_some_and(|detail| shown.contains(detail)),
      detail.is_some()
    );
    assert_eq!(panel.confirm_key(key(KeyCode::Enter)), event);
  }
  // Headings name no mark.
  let mut panel = marks();
  panel.choose(0);
  action(&mut panel, Action::RecentRemove);
  assert!(!panel.confirming());
}

#[test]
fn mark_rows_offer_removal() {
  let mut panel = marks();
  panel.choose(1);
  let destination = panel.rows()[1].destination.clone();
  let items = crate::context::panel_items(&panel, Some(&destination), false);
  let remove = items.iter().find(|item| item.label == "Remove mark");
  assert_eq!(remove.map(|item| item.key.as_str()), Some("x"));
  // A press below the rows names no mark, whatever is selected.
  let items = crate::context::panel_items(&panel, None, false);
  assert!(items.iter().all(|item| item.label != "Remove mark"));
  // Other panels ignore the key.
  let mut outline = Panel::new(PanelKind::Outline, rows(&[entry("/a.pdf")]));
  assert_eq!(action(&mut outline, Action::RecentRemove), Event::Stay);
  assert!(!outline.confirming());
}

#[test]
fn small_confirm_bars_name_their_keys() {
  // (width, height, buttons)
  for (width, height, buttons) in
    [(100, 24, true), (100, 2, false), (18, 24, false)]
  {
    let mut panel = panel();
    action(&mut panel, Action::RecentRemove);
    let mut scene = crate::testing::empty();
    panel.scene_confirmation(&mut scene, At::default(), width, height);
    let shown = crate::testing::text(&scene.into_scene());
    assert_eq!(shown.contains("[ Remove"), buttons, "{width}x{height}");
    // A narrow bar clips the question first.
    assert_eq!(
      shown.contains("[Y/n]"),
      !buttons && width > 40,
      "{width}x{height}"
    );
    assert_eq!(
      panel.confirm_press(press(2, height.saturating_sub(2))) == Event::Stay,
      !buttons,
      "{width}x{height}"
    );
  }
}

#[test]
fn marks_headers_hint_the_jump() {
  let position = Position::default();
  let local = vec![Row::local_mark('a', position, None, "1")];
  let mut panel = Panel::new(PanelKind::Marks, Row::marks(local, Vec::new()));
  panel.group(&[]);
  panel.choose(1);
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, At::default(), 40, 4);
  let header = &crate::testing::lines(&scene, 40)[&0];
  assert!(header.starts_with(" MARKS  1  ’a jump"), "{header:?}");
}

#[test]
fn directory_selection_keeps_hidden_members() {
  let entries =
    [entry("/books/a.pdf"), entry("/books/b.pdf"), entry("/other/a.pdf")];
  let mut grouped = vec![Row::new(
    "/books".into(),
    String::new(),
    Destination::Directory("/books".into()),
  )];
  grouped.extend(rows(&entries));
  let mut panel = Panel::recent(grouped, &entries);
  assert_eq!(panel.selection_mark(&panel.rows[0].destination), "○ ");
  panel.choose(1);
  action(&mut panel, Action::RecentToggle);
  assert_eq!(panel.selection_mark(&panel.rows[0].destination), "◐ ");
  panel.choose(0);
  assert_eq!(action(&mut panel, Action::Follow), Event::Stay);
  assert_eq!(panel.removal.checked.len(), 2);
  assert_eq!(panel.selection_mark(&panel.rows[0].destination), "● ");
  action(&mut panel, Action::RecentToggle);
  assert!(panel.removal.checked.is_empty());
  // A projection cannot redefine directory membership.
  let tree = tree::Tree::from_depths([0, 1, 1, 0].into_iter());
  panel.project(tree.project(|source| source == 0));
  panel.choose(0);
  action(&mut panel, Action::RecentRemove);
  assert_eq!(
    panel.confirm_key(key(KeyCode::Enter)),
    Event::Forget(vec!["/books/a.pdf".into(), "/books/b.pdf".into()])
  );
}

#[test]
fn recent_mouse_matches_group_keys() {
  let entries = [entry("/books/a.pdf"), entry("/books/b.pdf")];
  let mut grouped = vec![Row::new(
    "/books".into(),
    String::new(),
    Destination::Directory("/books".into()),
  )];
  grouped.extend(rows(&entries));
  let mut panel = Panel::recent(grouped, &entries);
  panel.scene_list(&mut Vec::new(), At::default(), 80, 12);
  let click = panel.press(press(4, 1));
  assert_eq!(click.activate(&mut panel, 1), Activation::Stay);
  assert_eq!(panel.removal.checked.len(), 2);
  assert_eq!(panel.selected(), 0);
  let click = panel.press(press(4, 2));
  assert_eq!(click.activate(&mut panel, 1), Activation::Stay);
  assert_eq!(panel.removal.checked.len(), 1);
  assert_eq!(panel.selection_mark(&panel.rows[0].destination), "◐ ");
}
