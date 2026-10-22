use crate::input::event;
use crossterm::event as raw;
use vpdf_model::input as core;
use vpdf_terminal::decode as wire;

#[test]
fn keys_preserve_state() {
  for bits in 0..64 {
    for state in 0..8 {
      for (report, kind, want) in [
        (1, raw::KeyEventKind::Press, core::KeyEventKind::Press),
        (2, raw::KeyEventKind::Repeat, core::KeyEventKind::Repeat),
        (3, raw::KeyEventKind::Release, core::KeyEventKind::Release),
      ] {
        let (wire, code) =
          if state & 1 == 0 { (233, 'é') } else { (57404, '5') };
        let source = raw::KeyEvent::new_with_kind_and_state(
          raw::KeyCode::Char(code),
          raw::KeyModifiers::from_bits_retain(bits),
          kind,
          raw::KeyEventState::from_bits_retain(state),
        );
        let mask = ((bits & 2) << 1) | ((bits & 4) >> 1) | (bits & !6);
        let mask = u16::from(mask) | (u16::from(state & 6) << 5);
        let bytes = format!("\x1b[{wire};{}:{report}u", mask + 1);
        let key = decode_key(bytes.as_bytes());
        assert_eq!(key.code, core::KeyCode::Char(code));
        assert_eq!(key.kind, want);
        for (raw, neutral) in [
          (raw::KeyModifiers::SHIFT, core::KeyModifiers::SHIFT),
          (raw::KeyModifiers::CONTROL, core::KeyModifiers::CONTROL),
          (raw::KeyModifiers::ALT, core::KeyModifiers::ALT),
          (raw::KeyModifiers::SUPER, core::KeyModifiers::SUPER),
          (raw::KeyModifiers::HYPER, core::KeyModifiers::HYPER),
          (raw::KeyModifiers::META, core::KeyModifiers::META),
        ] {
          assert_eq!(
            key.modifiers.contains(neutral),
            source.modifiers.contains(raw)
          );
        }
        for (raw, neutral) in [
          (raw::KeyEventState::KEYPAD, core::KeyEventState::KEYPAD),
          (raw::KeyEventState::CAPS_LOCK, core::KeyEventState::CAPS_LOCK),
          (raw::KeyEventState::NUM_LOCK, core::KeyEventState::NUM_LOCK),
        ] {
          assert_eq!(key.state.contains(neutral), source.state.contains(raw));
        }
      }
    }
  }
}

#[test]
fn key_identity_preserves_case() {
  let decode = |ch: char, flags: raw::KeyModifiers| {
    let mask = u16::from(flags.bits() & 1) | (u16::from(flags.bits() & 2) << 1);
    let key =
      decode_key(format!("\x1b[{};{}u", u32::from(ch), mask + 1).as_bytes());
    assert_eq!(key.code, core::KeyCode::Char(ch));
    assert_eq!(
      key.modifiers.contains(core::KeyModifiers::SHIFT),
      flags.contains(raw::KeyModifiers::SHIFT)
    );
    assert_eq!(
      key.modifiers.contains(core::KeyModifiers::CONTROL),
      flags.contains(raw::KeyModifiers::CONTROL)
    );
    key
  };
  let codes = ['a', 'A', 'é', 'É', ' '];
  for left in codes {
    for right in codes {
      for a in [
        raw::KeyModifiers::NONE,
        raw::KeyModifiers::SHIFT,
        raw::KeyModifiers::CONTROL,
      ] {
        for b in [
          raw::KeyModifiers::NONE,
          raw::KeyModifiers::SHIFT,
          raw::KeyModifiers::CONTROL,
        ] {
          let raw_left = raw::KeyEvent::new(raw::KeyCode::Char(left), a);
          let raw_right = raw::KeyEvent::new(raw::KeyCode::Char(right), b);
          assert_eq!(
            decode(left, a) == decode(right, b),
            raw_left == raw_right
          );
        }
      }
    }
  }
}

#[test]
fn pointer_coordinates_and_history() {
  let mut parser = wire::Parser::default();
  parser.set_pixels(true);
  let mut reports = std::collections::VecDeque::new();
  parser.feed(
    b"\x1b[<0;419;233M\x1b[<128;419;233M\x1b[<129;419;233M",
    &mut reports,
  );
  let expected = [Some(false), Some(true)];
  let core::Event::Mouse(press) = event(reports.pop_front().unwrap()).unwrap()
  else {
    panic!("pointer")
  };
  assert_eq!(press.pixel(), Some((419, 233)));
  let cell = press.cells((8, 16)).unwrap();
  assert_eq!((cell.column, cell.row), (52, 14));
  assert_eq!(cell.kind, core::MouseEventKind::Down(core::MouseButton::Left));
  assert_eq!(reports.len(), 2);
  for (report, expected) in reports.into_iter().zip(expected) {
    let core::Event::Mouse(mouse) = event(report).unwrap() else {
      panic!("pointer")
    };
    assert_eq!(mouse.history(), expected);
    assert!(mouse.cells((8, 16)).is_none());
  }
  assert!(event(wire::Event::Resize(None)).is_none());
  assert!(event(wire::Event::Focus(false)).is_none());
}

fn decode_key(bytes: &[u8]) -> core::KeyEvent {
  let events = Vec::from(super::common::events(bytes));
  let [core::Event::Key(key)] = events.as_slice() else { panic!("key report") };
  *key
}
