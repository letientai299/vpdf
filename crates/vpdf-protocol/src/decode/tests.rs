//! Byte fixtures. "Recorded" ones are reads `scripts/record.py` logged:
//! kitty 0.49.1 driven by `kitten @ send-key` and `send-text`, real
//! pointer motion in that window, and `WezTerm` 20240203-110809-5046fc22
//! replies from a headless mux. "Source" ones follow the cited protocol
//! code or docs where no driveable terminal produced them.
use super::*;

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

/// Decodes `reads`, each followed by a quiet pause.
fn decode(reads: &[&[u8]]) -> Vec<Event> {
  let mut parser = Parser::default();
  let mut events = VecDeque::new();
  for read in reads {
    parser.feed(read, &mut events);
    parser.end(&mut events);
  }
  events.into()
}

fn char(ch: char, modifiers: KeyModifiers) -> Event {
  key(KeyCode::Char(ch), modifiers)
}

fn cell(kind: MouseKind, column: u16, row: u16) -> Event {
  Event::Mouse(Mouse { kind, at: Spot::Cell { column, row }, modifiers: NONE })
}

fn reply(reply: Reply) -> Event {
  Event::Reply(reply)
}

fn graphics(id: u32, message: &str) -> Event {
  let message = message.into();
  reply(Reply::Graphics { id: Some(id), placement: None, message })
}

fn check(cases: &[(&[u8], Event)]) {
  for (bytes, want) in cases {
    assert_eq!(
      decode(&[bytes]),
      std::slice::from_ref(want),
      "{}",
      bytes.escape_ascii()
    );
  }
}

#[test]
fn kitty_keys_recorded() {
  let keypad = |code| {
    Event::Key(KeyEvent::new_with_kind_and_state(
      code,
      NONE,
      KeyEventKind::Press,
      KeyEventState::KEYPAD,
    ))
  };
  check(&[
    (b"\t", key(KeyCode::Tab, NONE)),
    (b"\x1b[105;5u", char('i', CTRL)),
    (b"\x1b[111;5u", char('o', CTRL)),
    (b"\x1b[9;2u", key(KeyCode::BackTab, SHIFT)),
    // send-key carries no shifted key; a real keyboard does, below.
    (b"\x1b[45;6u", char('-', CTRL | SHIFT)),
    (b"\x1b[27u", key(KeyCode::Esc, NONE)),
    (b"\x1b[120;3u", char('x', ALT)),
    (b"\x1b[119;5u", char('w', CTRL)),
    (b"a", char('a', NONE)),
    (b"\x1b[97;2u", char('a', SHIFT)),
    (b"\x1b[97;6u", char('a', CTRL | SHIFT)),
    (b"\x1b[97;7u", char('a', CTRL | ALT)),
    (b"\r", key(KeyCode::Enter, NONE)),
    (b"\x1b[13;5u", key(KeyCode::Enter, CTRL)),
    (b"\x7f", key(KeyCode::Backspace, NONE)),
    (b"\x1b[P", key(KeyCode::F(1), NONE)),
    (b"\x1b[13~", key(KeyCode::F(3), NONE)),
    (b"\x1b[15~", key(KeyCode::F(5), NONE)),
    (b"\x1b[24~", key(KeyCode::F(12), NONE)),
    (b"\x1b[D", key(KeyCode::Left, NONE)),
    (b"\x1b[1;5D", key(KeyCode::Left, CTRL)),
    (b"\x1b[1;2A", key(KeyCode::Up, SHIFT)),
    (b"\x1b[H", key(KeyCode::Home, NONE)),
    (b"\x1b[F", key(KeyCode::End, NONE)),
    (b"\x1b[6~", key(KeyCode::PageDown, NONE)),
    (b"\x1b[3~", key(KeyCode::Delete, NONE)),
    (b"\x1b[57404u", keypad(KeyCode::Char('5'))),
    (b"\x1b[57414u", keypad(KeyCode::Enter)),
    (b"\x1b[91;5u", char('[', CTRL)),
  ]);
}

/// Kitty keyboard protocol docs, flags 1 and 4.
#[test]
fn kitty_keys_source() {
  let release = Event::Key(KeyEvent::new_with_kind(
    KeyCode::Char('a'),
    NONE,
    KeyEventKind::Release,
  ));
  check(&[
    // Ctrl-Shift-minus on a US layout reports the shifted `_`.
    (b"\x1b[45:95;6u", char('_', CTRL)),
    (b"\x1b[97:65;6u", char('A', CTRL)),
    (b"\x1b[97:65:97;6u", char('A', CTRL)),
    (b"\x1b[97;1:3u", release),
    (b"\x1b[57376u", key(KeyCode::F(13), NONE)),
    (b"\x1b[1;5P", key(KeyCode::F(1), CTRL)),
  ]);
  // Modifier and media keys have no binding and decode to nothing.
  assert_eq!(decode(&[b"\x1b[57441;2u\x1b[57428u"]), []);
}

#[test]
fn associated_text_preserves_typing() {
  check(&[
    (b"\x1b[97;1;97u", char('a', NONE)),
    (b"\x1b[97:65;2;65u", char('A', NONE)),
    (b"\x1b[101;;233u", char('é', NONE)),
    (b"\x1b[0;;229u", char('å', NONE)),
    (b"\x1b[0;;20320:22909u", Event::Paste("你好".into())),
    (b"\x1b[97;5;97u", char('a', CTRL)),
  ]);
}

/// Legacy bytes, as crossterm decodes them.
#[test]
fn legacy_keys() {
  check(&[
    (b"\x1bOA", key(KeyCode::Up, NONE)),
    (b"\x1bOP", key(KeyCode::F(1), NONE)),
    (b"\x1b[Z", key(KeyCode::BackTab, SHIFT)),
    (b"\x1bx", char('x', ALT)),
    (b"\x1b\x7f", key(KeyCode::Backspace, ALT)),
    (b"\x00", char(' ', CTRL)),
    (b"\x08", char('h', CTRL)),
    (b"\x1f", char('7', CTRL)),
    (b"A", char('A', SHIFT)),
    ("é".as_bytes(), char('é', NONE)),
  ]);
}

#[test]
fn esc_waits_for_quiet() {
  let esc = key(KeyCode::Esc, NONE);
  assert_eq!(decode(&[b"\x1b"]), std::slice::from_ref(&esc));
  assert_eq!(decode(&[b"\x1b", b"\x1b"]), [esc.clone(), esc.clone()]);
  assert_eq!(decode(&[b"\x1b\x1b[A"]), [esc, key(KeyCode::Up, NONE)]);
  let mut parser = Parser::default();
  let mut events = VecDeque::new();
  parser.feed(b"\x1b", &mut events);
  assert!(parser.waiting());
  parser.feed(b"[A", &mut events);
  assert_eq!(Vec::from(events), [key(KeyCode::Up, NONE)]);
}

/// A macOS pty hands a burst out in 1022-byte reads, which can end right
/// after an ESC; only a quiet pause makes that ESC a key.
#[test]
fn reads_cut_after_esc_stay_sequences() {
  let mut burst = Vec::new();
  for column in 1..=600 {
    burst.extend_from_slice(format!("\x1b[<35;{column};14M").as_bytes());
  }
  burst.extend_from_slice(b"\x1b[200~text\x1b[201~");
  let cut = (1..burst.len()).find(|&at| burst[at - 1] == 0x1b).unwrap();
  let mut parser = Parser::default();
  let mut events = VecDeque::new();
  for read in [&burst[..cut], &burst[cut..]] {
    for chunk in read.chunks(1022) {
      parser.feed(chunk, &mut events);
    }
  }
  parser.end(&mut events);
  let keys = events.iter().filter(|event| matches!(event, Event::Key(_)));
  assert_eq!(keys.count(), 0);
  assert_eq!(events.len(), 601);
  assert_eq!(events.back(), Some(&Event::Paste("text".into())));
}

#[test]
fn split_sequences_wait() {
  assert_eq!(decode(&[b"\x1b[105", b";5u"]), [char('i', CTRL)]);
  assert_eq!(decode(&[b"\xc3", b"\xa9"]), [char('é', NONE)]);
  assert_eq!(
    decode(&[b"\x1b[<0;20", b";10M"]),
    [cell(MouseKind::Press(Button(1)), 19, 9)]
  );
  assert_eq!(
    decode(&[b"\x1bP>|kit", b"ty(0.49.1)\x1b", b"\\"]),
    [reply(Reply::Version("kitty(0.49.1)".into()))]
  );
}

#[test]
fn kitty_mouse_recorded() {
  let left = Button(1);
  let middle = Button(2);
  let right = Button(3);
  check(&[
    (b"\x1b[<35;57;14M", cell(MouseKind::Moved, 56, 13)),
    (b"\x1b[<0;20;10M", cell(MouseKind::Press(left), 19, 9)),
    (b"\x1b[<0;20;10m", cell(MouseKind::Release(left), 19, 9)),
    (b"\x1b[<32;26;14M", cell(MouseKind::Drag(left), 25, 13)),
    (b"\x1b[<1;25;15M", cell(MouseKind::Press(middle), 24, 14)),
    (b"\x1b[<33;24;15M", cell(MouseKind::Drag(middle), 23, 14)),
    (b"\x1b[<1;24;15m", cell(MouseKind::Release(middle), 23, 14)),
    (b"\x1b[<2;25;14M", cell(MouseKind::Press(right), 24, 13)),
    (b"\x1b[<2;25;14m", cell(MouseKind::Release(right), 24, 13)),
    (b"\x1b[I", Event::Focus(true)),
  ]);
  let two = decode(&[b"\x1b[<35;28;29M\x1b[<35;28;29M"]);
  assert_eq!(
    two,
    [cell(MouseKind::Moved, 27, 28), cell(MouseKind::Moved, 27, 28)]
  );
}

/// kitty `encode_button` and `encode_mouse_event_impl` in kitty/mouse.c;
/// Ghostty and `WezTerm` encode buttons 4-11 the same way.
#[test]
fn mouse_buttons_source() {
  let shifted = Event::Mouse(Mouse {
    kind: MouseKind::Press(Button(4)),
    at: Spot::Cell { column: 4, row: 2 },
    modifiers: SHIFT | CTRL,
  });
  check(&[
    (b"\x1b[<128;5;3M", cell(MouseKind::Press(Button::BACK), 4, 2)),
    (b"\x1b[<128;5;3m", cell(MouseKind::Release(Button::BACK), 4, 2)),
    (b"\x1b[<129;5;3M", cell(MouseKind::Press(Button::FORWARD), 4, 2)),
    (b"\x1b[<131;5;3M", cell(MouseKind::Press(Button(11)), 4, 2)),
    (b"\x1b[<64;5;3M", cell(MouseKind::Press(Button(4)), 4, 2)),
    (b"\x1b[<65;5;3M", cell(MouseKind::Press(Button(5)), 4, 2)),
    (b"\x1b[<67;5;3M", cell(MouseKind::Press(Button(7)), 4, 2)),
    (b"\x1b[<84;5;3M", shifted),
    (b"\x1b[O", Event::Focus(false)),
  ]);
}

/// Mode 1016 reports pixels; kitty sends leave only then, as 256 | 32.
#[test]
fn pixel_mouse_source() {
  let mut parser = Parser { pixels: true, ..Parser::default() };
  let mut events = VecDeque::new();
  parser.feed(b"\x1b[<0;419;233M\x1b[<288;12;7M\x1b[<32;-4;9M", &mut events);
  let at = |x, y| Spot::Pixel { x, y };
  let press = Mouse {
    kind: MouseKind::Press(Button(1)),
    at: at(419, 233),
    modifiers: NONE,
  };
  let drag =
    Mouse { kind: MouseKind::Drag(Button(1)), at: at(0, 9), modifiers: NONE };
  assert_eq!(
    Vec::from(events),
    [Event::Mouse(press), Event::Leave, Event::Mouse(drag)]
  );
  // Without mode 1016 a negative coordinate is malformed.
  parser.set_pixels(false);
  let mut events = VecDeque::new();
  parser.feed(b"\x1b[<32;-4;9M", &mut events);
  assert!(events.is_empty());
}

#[test]
fn kitty_replies_recorded() {
  let events = decode(&[
    b"\x1b[48;36;120;468;840t\x1bP>|kitty(0.49.1)\x1b\\",
    b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=999999;ENOENT:Put command refers to \
      non-existent image with id: 999999 and number: 0\x1b\\\x1b[?2026;2$y\
      \x1b[?1016;2$y\x1b[?2048;1$y\x1b[?5u",
    b"\x1b]22;1,1\x1b\\",
    b"\x1b]72;t=q\x1b\\\x1b[6;13;7t\x1b[4;468;840t",
    b"\x1b[?62;52;c",
  ]);
  let mode = |mode, state| reply(Reply::Mode { mode, state });
  assert_eq!(
    events,
    [
      Event::Resize(Some(Size {
        rows: 36,
        columns: 120,
        height: 468,
        width: 840
      })),
      reply(Reply::Version("kitty(0.49.1)".into())),
      graphics(31, "OK"),
      graphics(
        999_999,
        "ENOENT:Put command refers to non-existent image with id: 999999 \
       and number: 0"
      ),
      mode(2026, ModeState::Reset),
      mode(1016, ModeState::Reset),
      mode(2048, ModeState::Set),
      reply(Reply::Keyboard(5)),
      reply(Reply::Osc { code: 22, body: "1,1".into() }),
      reply(Reply::Osc { code: 72, body: "t=q".into() }),
      reply(Reply::Window(vec![6, 13, 7])),
      reply(Reply::Window(vec![4, 468, 840])),
      reply(Reply::Attributes(vec![62, 52])),
    ]
  );
}

#[test]
fn wezterm_replies_recorded() {
  let events = decode(&[
    b"\x1bP>|WezTerm 20240203-110809-5046fc22\x1b\\\x1b_Gi=31;OK\x1b\\",
    b"\x1b[?2026;2$y\x1b[?1016;2$y\x1b[?2048;0$y\x1b[?5u",
    b"\x1b[6;16;8t\x1b[4;576;960t\x1b[?65;4;6;18;22c",
    b"\x1b[200~pasted text \xc3\xa9\x1b[201~",
    b"\t",
  ]);
  assert_eq!(
    events,
    [
      reply(Reply::Version("WezTerm 20240203-110809-5046fc22".into())),
      graphics(31, "OK"),
      reply(Reply::Mode { mode: 2026, state: ModeState::Reset }),
      reply(Reply::Mode { mode: 1016, state: ModeState::Reset }),
      reply(Reply::Mode { mode: 2048, state: ModeState::NotRecognized }),
      reply(Reply::Keyboard(5)),
      reply(Reply::Window(vec![6, 16, 8])),
      reply(Reply::Window(vec![4, 576, 960])),
      reply(Reply::Attributes(vec![65, 4, 6, 18, 22])),
      Event::Paste("pasted text é".into()),
      key(KeyCode::Tab, NONE),
    ]
  );
}

/// XTVERSION formats from the sources in terminals.md; OSC 52 and 5522
/// from the xterm and kitty clipboard docs, BEL-terminated as xterm
/// allows.
#[test]
fn other_replies_source() {
  let version = |name: &str| reply(Reply::Version(name.into()));
  check(&[
    (b"\x1bP>|ghostty 1.3.1\x1b\\", version("ghostty 1.3.1")),
    (b"\x1bP>|iTerm2 3.7.6\x1b\\", version("iTerm2 3.7.6")),
    (b"\x1bP>|Konsole 26.08.2\x1b\\", version("Konsole 26.08.2")),
    (b"\x1bP>|Rio 0.5.28\x1b\\", version("Rio 0.5.28")),
    (
      b"\x1b]52;c;aGk=\x07",
      reply(Reply::Osc { code: 52, body: "c;aGk=".into() }),
    ),
    (
      b"\x1b]5522;type=read:status=OK\x1b\\",
      reply(Reply::Osc { code: 5522, body: "type=read:status=OK".into() }),
    ),
    (b"\x1b[?1u", reply(Reply::Keyboard(1))),
    (
      b"\x1b[?2026;3$y",
      reply(Reply::Mode { mode: 2026, state: ModeState::AlwaysSet }),
    ),
    (
      b"\x1b_Gi=7,p=2;ENOENT:gone\x1b\\",
      reply(Reply::Graphics {
        id: Some(7),
        placement: Some(2),
        message: "ENOENT:gone".into(),
      }),
    ),
  ]);
}

#[test]
fn replies_interleave_with_keys() {
  let events = decode(&[b"j\x1b_Gi=31;OK\x1b\\k\x1b[?62;22c\x1b[105;5u"]);
  assert_eq!(
    events,
    [
      char('j', NONE),
      graphics(31, "OK"),
      char('k', NONE),
      reply(Reply::Attributes(vec![62, 22])),
      char('i', CTRL),
    ]
  );
}

/// kitty sent a 40000-byte `send-text` as 2048-byte pastes plus an
/// empty one, read 1022 bytes at a time; recorded geometry.
#[test]
fn kitty_paste_recorded() {
  let mut sizes = vec![2048; 19];
  sizes.extend([1088, 0]);
  let stream: Vec<u8> = sizes
    .iter()
    .flat_map(|&size| {
      [&b"\x1b[200~"[..], &vec![b'x'; size], PASTE_END].concat()
    })
    .collect();
  let reads: Vec<&[u8]> = stream.chunks(1022).collect();
  let pastes: Vec<Event> =
    sizes.iter().map(|&size| Event::Paste("x".repeat(size))).collect();
  assert_eq!(decode(&reads), pastes);
}

#[test]
fn paste_marker_split_across_reads() {
  let reads: [&[u8]; 4] = [b"\x1b[200~a\x1bb\x1b", b"[2", b"01", b"~q"];
  assert_eq!(decode(&reads), [Event::Paste("a\x1bb".into()), char('q', NONE)]);
}

#[test]
fn junk_is_skipped() {
  for junk in [
    &b"\x1b[999z"[..],
    b"\x1b[<99;1M",
    b"\x1bPq#0\x1b\\",
    b"\x80",
    b"\x1b_Xjunk\x1b\\",
  ] {
    let mut bytes = junk.to_vec();
    bytes.push(b'q');
    let events = decode(&[&bytes]);
    assert_eq!(
      events.last(),
      Some(&char('q', NONE)),
      "{}",
      junk.escape_ascii()
    );
  }
  // An ESC that starts a key abandons an unterminated string.
  assert_eq!(decode(&[b"\x1b]2;title\x1b[A"]), [key(KeyCode::Up, NONE)]);
  // Past `LIMIT`, a string or CSI is dropped whole, not read as keys.
  let body = vec![b'1'; LIMIT + 1];
  for (start, rest) in [
    (&b"\x1b]52;c;"[..], &b"a\x1b\\q"[..]),
    (b"\x1b]52;c;", b"\x1b\\q"),
    (b"\x1b[", b";5uq"),
    (b"\x1b[", b"200~q"),
  ] {
    let mut parser = Parser::default();
    let mut events = VecDeque::new();
    parser.feed(start, &mut events);
    for chunk in body.chunks(1 << 20) {
      parser.feed(chunk, &mut events);
    }
    parser.feed(rest, &mut events);
    assert_eq!(Vec::from(events), [char('q', NONE)], "{}", rest.escape_ascii());
  }
}

/// tmux 3.8 in kitty 0.49.1 after `CSI > 4 ; 2 m`, `extended-keys on`;
/// `\e[105;5u` was sent as raw text, since kitty ignores modifyOtherKeys
/// and sends Ctrl-i as `\t`. Each chord also in `extended-keys-format
/// csi-u` ([tmux source][tmux]).
///
/// [tmux]: https://github.com/tmux/tmux/blob/3.8/input-keys.c#L425-L473
#[test]
fn tmux_extended_keys_recorded() {
  check(&[
    (b"\x1b[27;5;111~", char('o', CTRL)),
    (b"\x1b[27;2;9~", key(KeyCode::BackTab, SHIFT)),
    (b"\x1b[27;5;105~", char('i', CTRL)),
    (b"\t", key(KeyCode::Tab, NONE)),
    (b"\x1b[27;3;120~", char('x', ALT)),
    (b"\x1b[27;5;13~", key(KeyCode::Enter, CTRL)),
    (b"\x1b[27;6;86~", char('V', CTRL | SHIFT)),
    (b"\x1b[111;5u", char('o', CTRL)),
    (b"\x1b[9;2u", key(KeyCode::BackTab, SHIFT)),
    (b"\x1b[86;6u", char('V', CTRL | SHIFT)),
  ]);
  // Not a key: no code, or the tilde key 27 alone.
  assert_eq!(decode(&[b"\x1b[27;5~\x1b[27~"]), []);
}

/// Kitty keyboard docs: flag 4 adds the US-layout base key, here `w`
/// under a Russian layout's `ц`.
#[test]
fn base_layout_key_serves_chords() {
  check(&[
    (b"\x1b[1094::119;5u", char('w', CTRL)),
    (b"\x1b[1094::119;3u", char('w', ALT)),
    ("ц".as_bytes(), char('ц', NONE)),
  ]);
}
