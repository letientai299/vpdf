use super::*;
use crossterm::event::{KeyCode, KeyModifiers};

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
