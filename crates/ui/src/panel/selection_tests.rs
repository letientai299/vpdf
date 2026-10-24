use super::*;
use crate::bindings::{Bindings, Context, Target as Binding};
use crate::keys::Dispatch;
use crate::panel::recent::{Entry, Frame};
use vpdf_model::geometry::At;
use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};

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
  crate::testing::text(&scene.into_scene())
}

#[test]
fn checkboxes_preserve_cursor_and_preview() {
  let mut panel = panel();
  let first = text(&mut panel);
  assert_eq!(first.matches("[ ]").count(), 2);
  assert!(first.contains("0 selected"));
  panel.point(Pointing::Row(1));
  for count in [1, 0, 1] {
    assert_eq!(action(&mut panel, Action::RecentToggle), Event::Stay);
    assert_eq!(panel.selected(), 0);
    assert_eq!(panel.recent.checked.len(), count);
  }
  action(&mut panel, Action::Motion(Motion::Down));
  assert_eq!(panel.selected(), 1);
  assert_eq!(panel.recent.checked.len(), 1);
  assert_eq!(action(&mut panel, Action::Follow), Event::Follow { focus: true });
  assert_eq!(
    panel.follow(&crate::find::Search::default()),
    Some(Follow::File("/missing/b.pdf".into()))
  );
  assert_eq!(action(&mut panel, Action::Cancel), Event::Stay);
  assert!(panel.recent.checked.is_empty());
  action(&mut panel, Action::RecentToggle);
  panel.focus(false);
  assert!(panel.recent.checked.is_empty());
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
    assert!(before.contains("Remove \"a.pdf\" from recent? [Y/n]"));
    assert!(
      before.contains("Saved position and local marks will be forgotten.")
    );
    assert_eq!(panel.confirm_key(cancel), Event::Stay);
    assert!(!panel.confirming());
    assert_eq!(panel.rows().len(), 2);
    assert_eq!(panel.selected(), 0);
    assert_eq!(panel.recent.checked.len(), 1);
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
  assert!(text(&mut panel).contains("Remove 2 files from recent? [Y/n]"));
  panel.refresh_recent(
    rows(&[entry("/missing/c.pdf")]),
    &[entry("/missing/c.pdf")],
  );
  assert_eq!(
    panel.confirm_key(key(KeyCode::Enter)),
    Event::Forget(vec!["/missing/a.pdf".into(), "/missing/b.pdf".into()])
  );
  assert!(panel.recent.checked.is_empty());
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
    panel.recent.checked.iter().collect::<Vec<_>>(),
    vec![&PathBuf::from("/missing/b.pdf")]
  );
  panel.refresh_recent(rows(&entries[1..]), &entries[1..]);
  assert!(panel.recent.checked.is_empty());
  assert_eq!(panel.selected(), 0);
  panel.refresh_recent(Vec::new(), &[]);
  assert!(text(&mut panel).contains("A PDF reader with Vim keys."));
  assert!(panel.shelf_mut().unwrap().take_request().is_none());
}

#[test]
fn headings_and_welcome_cannot_select() {
  for rows in [Vec::new(), vec![Row::parse("# Today")]] {
    let mut panel = Panel::recent(rows, &[]);
    action(&mut panel, Action::RecentToggle);
    action(&mut panel, Action::RecentRemove);
    assert!(panel.recent.checked.is_empty());
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
  use crate::topics::{Destination, Lookup, Registry, TopicId};
  let registry =
    Registry::new(crate::topics::authored(), Bindings::default().snapshot())
      .unwrap();
  for context in [Context::Recent, Context::HomeRecent] {
    for (key, action) in
      [("<Space>", Action::RecentToggle), ("x", Action::RecentRemove)]
    {
      let target = Binding::Reader(action);
      assert_eq!(
        registry.lookup(key, context),
        Lookup::Found(Destination::Action(
          TopicId::parse(&target.topic()).unwrap()
        ))
      );
    }
  }
}
