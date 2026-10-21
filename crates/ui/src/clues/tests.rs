use super::*;
use vpdf_model::input::{KeyCode, KeyModifiers};

#[test]
fn empty_items_end_deadlines() {
  let now = Instant::now();
  let mut clues = Clues::default();
  assert_eq!(clues.wait(now), None);
  clues.restart(Some(now));
  assert_eq!(clues.wait(now), Some(Clues::DELAY));
  assert!(!clues.poll(now));
  let due = now + Clues::DELAY;
  assert!(clues.poll(due));
  assert_eq!(clues.wait(due), None);
  assert!(!clues.poll(due));
  assert!(!clues.show(Vec::new()));
  assert!(clues.shown().is_none());
  assert!(clues.show(vec![("g".into(), "prefix".into())]));
  assert_eq!(clues.wait(due), None);
  clues.restart(Some(due));
  assert!(clues.shown().is_none());
  assert_eq!(clues.wait(due), Some(Clues::DELAY));
  clues.restart(None);
  assert!(!clues.poll(due + Clues::DELAY));
}

#[test]
fn items_read_relevant_context() {
  fn unused<T>() -> Option<T> {
    panic!("Unrelated context read")
  }
  let keys = items("g", std::iter::from_fn(unused), std::iter::from_fn(unused));
  assert_ne!(keys, [] as [(String, String); 0]);
  let registers = [('z', "  last\nline "), ('a', "日本語\tword")].into_iter();
  let shown = items("\"", registers, std::iter::from_fn(unused));
  assert_eq!(
    &shown[..2],
    &[("a".into(), "日本語 word".into()), ("z".into(), "last line".into()),]
  );
  assert_eq!(shown[2].0, "+ *");
  assert_eq!(shown[3].0, "a-z");
  let mark = Position { page: crate::text::PageId(2), ..Position::default() };
  let shown = items("`", std::iter::from_fn(unused), [('b', mark)].into_iter());
  assert_eq!(
    shown,
    [
      ("b".into(), "page 3".into()),
      ("A-Z".into(), "mark in any document".into()),
      ("`".into(), "back to the last jump".into()),
    ]
  );
}

#[test]
fn clue_labels_name_keys() {
  let key = |ch, modifiers| KeyEvent::new(KeyCode::Char(ch), modifiers);
  let (none, ctrl) = (KeyModifiers::NONE, KeyModifiers::CONTROL);
  // (label) -> keys
  let cases = [
    ("v", Some(vec![key('v', none)])),
    ("gg", Some(vec![key('g', none), key('g', none)])),
    ("C-s", Some(vec![key('s', ctrl)])),
    ("-", Some(vec![key('-', none)])),
    ("a-z", None),
    ("A-Z", None),
    ("+ *", None),
  ];
  for (label, want) in cases {
    assert_eq!(clue_keys(label), want, "{label}");
  }
}

fn items<'a>(
  prefix: &str,
  registers: impl Iterator<Item = (char, &'a str)>,
  marks: impl Iterator<Item = (char, Position)>,
) -> Vec<(String, String)> {
  let mut keys = keys::Keys::default();
  for key in keys::parse(prefix).unwrap() {
    keys.feed(key, keys::Grammar::Objects);
  }
  super::items(&keys, keys::Grammar::Objects, |_| true, registers, marks)
}

#[test]
fn focused_panels_filter_ignored_continuations() {
  use crate::keys::{Grammar, Keys, PanelKind};
  use crate::panel::Panel;
  for recent in [false, true] {
    let mut panel = if recent {
      Panel::recent(Vec::new(), &[])
    } else {
      Panel::new(PanelKind::Outline, Vec::new())
    };
    let shown = |panel: &Panel, prefix| {
      let mut keys = Keys::default();
      for key in keys::parse(prefix).unwrap() {
        keys.feed(key, Grammar::Panel);
      }
      super::items(
        &keys,
        Grammar::Panel,
        |action| panel.allows(action),
        std::iter::empty(),
        std::iter::empty(),
      )
      .into_iter()
      .map(|(key, _)| key)
      .collect::<Vec<_>>()
    };
    let go = shown(&panel, "g");
    assert!(go.contains(&"g".into()));
    assert!(go.contains(&"q".into()));
    assert!(!go.contains(&"d".into()));
    assert!(!go.contains(&"v".into()));
    assert!(!go.contains(&"E".into()));
    let zoom = shown(&panel, "z");
    assert!(zoom.contains(&"z".into()));
    assert_eq!(zoom.contains(&"f".into()), !recent);
    let yank = shown(&panel, "y");
    assert!(yank.contains(&"y".into()));
    assert!(yank.contains(&"p".into()));
    assert_eq!(yank.contains(&"Y".into()), !recent);
    assert_eq!(shown(&panel, "m").is_empty(), recent);
    assert_eq!(shown(&panel, "'").is_empty(), recent);
    panel.focus(false);
    assert!(shown(&panel, "g").contains(&"d".into()));
  }
}
