//! Decoded terminal bytes reach key actions and prompt text.
use super::common::events;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use vpdf::keys::{Action, Grammar, Keys, normalize};
use vpdf::prompt::{History, Prompt, PromptEvent, PromptKind};
use vpdf_protocol::decode::Event;

const NONE: KeyModifiers = KeyModifiers::NONE;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

fn key(bytes: &[u8]) -> KeyEvent {
  let [Event::Key(key)] = Vec::from(events(bytes))[..] else {
    panic!("not one key: {}", bytes.escape_ascii());
  };
  key
}

fn action(key: KeyEvent) -> Option<Action> {
  Keys::default().feed(key, Grammar::Normal).map(|input| input.action)
}

#[test]
fn shift_space_scrolls_up() {
  let cases: &[(&[u8], i32)] = &[
    (b" ", 1),
    (b"\x1b[32;2u", -1),
    (b"\x1b[32:32;2u", -1),
    (b"\x1b[32:32;2;32u", -1),
    (b"\x1b[32:32:32;2u", -1),
    (b"\x1b[32::32;2u", -1),
    (b"\x1b[27;2;32~", -1),
  ];
  for &(bytes, direction) in cases {
    for grammar in [Grammar::Normal, Grammar::Objects] {
      let input = Keys::default().feed(key(bytes), grammar).unwrap();
      assert_eq!(input.action, Action::Screen(direction), "{bytes:?}");
    }
  }
}

#[test]
fn associated_text_preserves_typing() {
  let history = History::default();
  let mut prompt = Prompt::new(PromptKind::Search(true));
  for event in events(b"\x1b[101;;233u\x1b[0;;20320:22909u") {
    match event {
      Event::Key(key) => prompt.feed(key, &history),
      Event::Paste(text) => prompt.paste(&text),
      _ => panic!("not text"),
    };
  }
  assert_eq!(prompt.text, "é你好");
}

/// U02: Tab, Ctrl-i, and Ctrl-o keep their identity from bytes through
/// normalization to actions; only prompt Tab completes.
#[test]
fn tab_and_ctrl_i_stay_apart() {
  let tab = key(b"\t");
  let ctrl_i = key(b"\x1b[105;5u");
  let ctrl_o = key(b"\x1b[111;5u");
  assert_ne!(normalize(tab), normalize(ctrl_i));
  assert_eq!(action(ctrl_i), Some(Action::History(true)));
  assert_eq!(action(ctrl_o), Some(Action::History(false)));
  assert_eq!(action(tab), Some(Action::LocalFocus));
  let history = History::default();
  let complete = |key| {
    let mut prompt = Prompt::new(PromptKind::Command);
    prompt.feed(KeyEvent::new(KeyCode::Char('t'), NONE), &history);
    prompt.feed(key, &history) == PromptEvent::Complete
  };
  let chord = |code, modifiers| KeyEvent::new(code, modifiers);
  assert!(complete(tab));
  assert!(!complete(ctrl_i), "Ctrl-i is not Tab");
  assert!(!complete(chord(KeyCode::Tab, CTRL)));
  assert!(!complete(chord(KeyCode::Char('i'), CTRL | ALT)));
}

/// tmux 3.8 extended keys name jump history like Kitty's.
#[test]
fn tmux_extended_keys_reach_history() {
  assert_eq!(action(key(b"\x1b[27;5;105~")), Some(Action::History(true)));
  assert_eq!(action(key(b"\x1b[27;5;111~")), Some(Action::History(false)));
}

#[test]
fn ctrl_shift_letters_keep_legacy_bindings() {
  // kitty with flag 4 sends Ctrl-Shift-v as `118:86;6`.
  let ctrl_shift_v = key(b"\x1b[118:86;6u");
  let mut keys = Keys::default();
  keys.feed(KeyEvent::new(KeyCode::Char('w'), CTRL), Grammar::Normal);
  let split =
    keys.feed(ctrl_shift_v, Grammar::Normal).map(|input| input.action);
  assert!(matches!(split, Some(Action::Window(_))), "{split:?}");
}
