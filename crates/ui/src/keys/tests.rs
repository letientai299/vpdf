use super::*;

#[test]
fn obsolete_pan_keys_are_unbound() {
  for grammar in [Grammar::Normal, Grammar::Cursor, Grammar::Objects] {
    for typed in ["zh", "zl", "4zh", "4zl"] {
      let mut keys = Keys::default();
      for ch in typed.chars() {
        assert!(keys.feed(key(ch), grammar).is_none(), "{typed}");
      }
      assert_eq!(keys.pending(), "");
      let input = keys.feed(key('l'), grammar).unwrap();
      assert_eq!(input.action, Action::Motion(Motion::Right));
      assert_eq!(input.count, 1);
    }
  }
  assert!(clues("z").iter().all(|(key, _)| key != "h" && key != "l"));
}

#[test]
fn panel_yank_is_one_action() {
  for (typed, register) in [("yy", '"'), ("\"ayy", 'a')] {
    let mut keys = Keys::default();
    let mut inputs: Vec<_> =
      typed.chars().map(|ch| keys.feed(key(ch), Grammar::Panel)).collect();
    let input = inputs.pop().flatten().expect(typed);
    assert!(inputs.iter().all(Option::is_none), "{typed}");
    assert_eq!(input.action, Action::YankLine);
    assert_eq!(input.register, register);
    assert_eq!(keys.pending(), "");
  }
}

#[test]
fn control_c_cancels_pending_input() {
  let mut keys = Keys::default();
  keys.feed(key('y'), Grammar::Normal);
  let cancel = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
  assert_eq!(
    keys.feed(cancel, Grammar::Objects).unwrap().action,
    Action::Cancel
  );
  assert_eq!(keys.pending(), "");
}

#[test]
fn notation_matches_help() {
  // (binding, help notation)
  let cases = [
    ("C-o", "<C-o>"),
    ("Enter", "<CR>"),
    ("S-Space", "<S-Space>"),
    ("Space", "<Space>"),
    ("C-wv", "<C-w>v"),
    ("gg", "gg"),
    ("", ""),
  ];
  for (binding, shown) in cases {
    assert_eq!(notation(binding), shown, "{binding}");
  }
}

/// Clue names type their bindings back.
#[test]
fn names_parse_back_to_their_tokens() {
  let shown =
    ["g", "z", "C-w", "C-r", "[", "]"].into_iter().flat_map(|prefix| {
      let labels = clues(prefix).into_iter().map(|(label, _)| label);
      labels.map(|label| label.replace(' ', "Space")).collect::<Vec<_>>()
    });
  let bindings = BINDINGS.iter().map(|(sequence, ..)| sequence.to_string());
  for label in shown.chain(bindings) {
    let keys = parse(&label).unwrap_or_default();
    let tokens: Option<String> = keys.into_iter().map(token).collect();
    let plain = label.replace("S-Space", "\0").replace("Space", " ");
    let want = plain.replace('\0', "S-Space");
    assert_eq!(tokens.as_deref(), Some(want.as_str()), "{label}");
  }
  // Names are keys, not letters.
  let none = KeyModifiers::NONE;
  let cases = [
    ("BS", KeyCode::Backspace, none),
    ("Enter", KeyCode::Enter, none),
    ("Tab", KeyCode::Tab, none),
    ("Space", KeyCode::Char(' '), none),
    ("S-Space", KeyCode::Char(' '), KeyModifiers::SHIFT),
    ("C-w", KeyCode::Char('w'), KeyModifiers::CONTROL),
  ];
  for (name, code, modifiers) in cases {
    assert_eq!(parse(name), Some(vec![KeyEvent::new(code, modifiers)]));
  }
}

fn key(ch: char) -> KeyEvent {
  KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

#[test]
fn page_bindings_preserve_counts() {
  for grammar in
    [Grammar::Normal, Grammar::Cursor, Grammar::Objects, Grammar::Panel]
  {
    for (name, action) in [
      ("d", Action::Scroll(0.5)),
      ("C-d", Action::Scroll(0.5)),
      ("u", Action::Scroll(-0.5)),
      ("C-u", Action::Scroll(-0.5)),
      ("C-n", Action::Motion(Motion::NextPage)),
      ("C-p", Action::Motion(Motion::PreviousPage)),
      ("BS", Action::Motion(Motion::PreviousPage)),
    ] {
      for count in [None, Some('3')] {
        let mut keys = Keys::default();
        if let Some(count) = count {
          assert!(keys.feed(key(count), grammar).is_none());
        }
        let input = keys.feed(parse(name).unwrap()[0], grammar);
        if grammar == Grammar::Panel
          && !crate::bindings::Target::Reader(action)
            .allowed(crate::bindings::Context::Panel)
        {
          assert!(input.is_none());
          continue;
        }
        let input = input.unwrap();
        assert_eq!(input.action, action);
        assert_eq!(input.count, if count.is_some() { 3 } else { 1 });
        assert_eq!(input.explicit, count.is_some());
        assert_eq!(keys.pending(), "");
      }
    }
  }
}

#[test]
fn count_register_and_prefix() {
  let mut keys = Keys::default();
  for ch in "\"a12g".chars() {
    assert!(keys.feed(key(ch), Grammar::Objects).is_none());
  }
  let input = keys.feed(key('g'), Grammar::Objects).unwrap();
  assert_eq!(input.count, 12);
  assert_eq!(input.register, 'a');
  assert_eq!(input.action, Action::Motion(Motion::First));
  assert_eq!(
    keys.feed(key('0'), Grammar::Objects).unwrap().action,
    Action::Motion(Motion::LineStart)
  );
}

#[test]
fn section_prefix_counts() {
  for (text, forward) in [("[[", false), ("]]", true)] {
    let mut keys = Keys::default();
    let mut inputs =
      text.chars().filter_map(|ch| keys.feed(key(ch), Grammar::Objects));
    let input = inputs.next().unwrap();
    assert_eq!(input.action, Action::Section(forward));
    assert_eq!(input.count, 1);
    assert!(!input.explicit);
  }
  for (prefix, forward) in [('[', false), (']', true)] {
    let mut keys = Keys::default();
    keys.feed(key('2'), Grammar::Objects);
    assert!(keys.feed(key(prefix), Grammar::Objects).is_none());
    assert_eq!(keys.pending(), format!("2{prefix}"));
    let labels: Vec<_> =
      clues(&prefix.to_string()).into_iter().map(|(key, _)| key).collect();
    assert!(labels.contains(&prefix.to_string()));
    assert!(labels.contains(&"f".into()));
    assert_eq!(labels.len(), 2);
    let input = keys.feed(key(prefix), Grammar::Objects).unwrap();
    assert_eq!(input.action, Action::Section(forward));
    assert_eq!(input.count, 2);
    assert!(input.explicit);
    assert_eq!(keys.pending(), "");
    keys.feed(key(prefix), Grammar::Objects);
    keys.feed(key('x'), Grammar::Objects);
    assert!(keys.awaiting().is_none());
    assert_eq!(keys.feed(key('j'), Grammar::Objects).unwrap().count, 1);
  }
}

#[test]
fn marks_take_a_name() {
  let mut keys = Keys::default();
  let mut feed = |text: &str| {
    text
      .chars()
      .filter_map(|ch| keys.feed(key(ch), Grammar::Objects))
      .map(|i| i.action)
      .last()
  };
  assert_eq!(feed("ma"), Some(Action::Mark('a')));
  assert_eq!(feed("'B"), Some(Action::GoMark('B', false)));
  assert_eq!(feed("`a"), Some(Action::GoMark('a', true)));
  assert_eq!(feed("''"), Some(Action::History(false)));
  assert_eq!(feed("m1j"), Some(Action::Motion(Motion::Down)));
}

#[test]
fn ctrl_r_extends_panes() {
  for grammar in [
    Grammar::Normal,
    Grammar::Cursor,
    Grammar::Objects,
    Grammar::Panel,
    Grammar::Other,
  ] {
    let mut keys = Keys::default();
    for event in parse("3C-r").unwrap() {
      assert!(keys.feed(event, grammar).is_none());
    }
    assert!(keys.resizing());
    let input = keys.feed(key('h'), grammar).unwrap();
    assert_eq!(input.action, Action::Window(W::Extend(D::Left)));
    assert_eq!(input.count, 3);
    assert!(input.explicit);
    for (ch, direction) in
      [('h', D::Left), ('l', D::Right), ('k', D::Up), ('j', D::Down)]
    {
      let input = keys.feed(key(ch), grammar).unwrap();
      assert_eq!(input.action, Action::Window(W::Extend(direction)));
      assert_eq!(input.count, 1);
      assert!(!input.explicit);
      assert!(keys.resizing());
    }
  }
}

#[test]
fn resize_exits_on_escape() {
  let grammar = Grammar::Normal;
  let mut keys = Keys::default();
  keys.feed(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL), grammar);
  for event in parse("xq:C-w").unwrap() {
    assert!(keys.feed(event, grammar).is_none());
    assert!(keys.resizing());
  }
  assert!(keys.feed(key('2'), grammar).is_none());
  let input = keys.feed(key('l'), grammar).unwrap();
  assert_eq!(input.count, 2);
  assert!(keys.resizing());
  let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
  assert_eq!(keys.feed(escape, grammar).unwrap().action, Action::Cancel);
  assert!(!keys.resizing());
  let input = keys.feed(key('j'), grammar).unwrap();
  assert_eq!(input.action, Action::Motion(M::Down));
  assert_eq!(input.count, 1);
}

#[test]
fn ctrl_w_takes_a_window_command() {
  let mut keys = Keys::default();
  let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
  assert!(keys.feed(ctrl_w, Grammar::Objects).is_none());
  assert_eq!(keys.awaiting(), Some("C-w"));
  let input = keys.feed(key('v'), Grammar::Objects).unwrap();
  assert_eq!(input.action, Action::Window(W::Split(Axis::Columns)));
  assert!(keys.feed(ctrl_w, Grammar::Objects).is_none());
  let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
  let input = keys.feed(ctrl_l, Grammar::Objects).unwrap();
  assert_eq!(input.action, Action::Window(W::Focus(D::Right)));
  keys.feed(ctrl_w, Grammar::Objects);
  let shifted = KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT);
  let input = keys.feed(shifted, Grammar::Objects).unwrap();
  assert_eq!(input.action, Action::Window(W::Move(D::Right)));
}

#[test]
fn normalize_matches_legacy_bytes() {
  let (none, ctrl, alt, shift) = (
    KeyModifiers::NONE,
    KeyModifiers::CONTROL,
    KeyModifiers::ALT,
    KeyModifiers::SHIFT,
  );
  for (code, modifiers, want, left) in [
    (KeyCode::Char('['), ctrl, KeyCode::Esc, none),
    (KeyCode::Char('m'), ctrl, KeyCode::Enter, none),
    (KeyCode::Char('i'), ctrl, KeyCode::Char('i'), ctrl),
    (KeyCode::Tab, none, KeyCode::Tab, none),
    (KeyCode::Char('V'), ctrl, KeyCode::Char('v'), ctrl | shift),
    (KeyCode::Char('M'), ctrl, KeyCode::Enter, shift),
    (KeyCode::Char('['), ctrl | alt, KeyCode::Esc, alt),
    (KeyCode::Char('m'), ctrl | shift, KeyCode::Enter, shift),
    (KeyCode::Char('['), none, KeyCode::Char('['), none),
    (KeyCode::Char('j'), ctrl, KeyCode::Char('j'), ctrl),
  ] {
    let key = normalize(KeyEvent::new(code, modifiers));
    assert_eq!((key.code, key.modifiers), (want, left), "{code:?}");
  }
}

#[test]
fn chords_and_register_names() {
  let with = |ch, modifiers| KeyEvent::new(KeyCode::Char(ch), modifiers);
  let ctrl = |ch| with(ch, KeyModifiers::CONTROL);
  let alt = |ch| with(ch, KeyModifiers::ALT);
  let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
  let f5 = KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE);
  let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
  let ctrl_shift_v = with('v', KeyModifiers::CONTROL | KeyModifiers::SHIFT);
  let shift_a = with('A', KeyModifiers::SHIFT);
  let out = |action, register| Some((action, 1, register));
  let down = out(Action::Motion(Motion::Down), '"');
  let yank = out(Action::Yank, '"');
  let append = out(Action::Yank, 'A');
  let cancel = out(Action::Cancel, '"');
  let split = out(Action::Window(W::Split(Axis::Columns)), '"');
  let right = out(Action::Motion(Motion::Right), '"');
  let cases = [
    (vec![key('5'), ctrl('['), key('j')], vec![None, cancel, down]),
    (vec![ctrl('m')], vec![out(Action::Follow, '"')]),
    (vec![ctrl('i')], vec![out(Action::History(true), '"')]),
    (vec![alt('q')], vec![None]),
    (vec![with('c', KeyModifiers::CONTROL | KeyModifiers::ALT)], vec![None]),
    (vec![key('5'), alt('j'), key('j')], vec![None, None, down]),
    (vec![key('5'), key('"'), enter, key('j')], vec![None, None, None, down]),
    (vec![key('g'), alt('x'), key('g')], vec![None, None, None]),
    (
      vec![key('"'), key('a'), alt('x'), key('y')],
      vec![None, None, None, yank],
    ),
    (vec![ctrl('w'), ctrl_left, key('l')], vec![None, None, right]),
    (vec![ctrl('w'), ctrl_shift_v], vec![None, split]),
    (vec![key('"'), enter, key('y')], vec![None, None, yank]),
    (vec![key('"'), f5, key('y')], vec![None, None, yank]),
    (vec![key('"'), ctrl('a'), key('y')], vec![None, None, yank]),
    (vec![key('"'), shift_a, key('y')], vec![None, None, append]),
  ];
  for (events, want) in cases {
    let mut keys = Keys::default();
    let got: Vec<_> = events
      .iter()
      .map(|event| keys.feed(*event, Grammar::Objects))
      .map(|input| input.map(|i| (i.action, i.count, i.register)))
      .collect();
    assert_eq!(got, want, "{events:?}");
  }
}

#[test]
fn yank_still_takes_p() {
  let yank = |register| Some((Action::Yank, register));
  let path = |register| Some((Action::YankPath, register));
  let cases = [
    (Grammar::Normal, "yp", vec![yank('"'), path('"')]),
    (Grammar::Cursor, "yp", vec![yank('"'), path('"')]),
    (Grammar::Normal, "\"ayp", vec![None, None, yank('a'), path('a')]),
    (Grammar::Normal, "yy", vec![yank('"'), yank('"')]),
    (Grammar::Cursor, "yY", vec![yank('"'), Some((Action::YankRaw, '"'))]),
    (Grammar::Objects, "yp", vec![yank('"'), None]),
    (Grammar::Other, "yp", vec![yank('"'), None]),
  ];
  for (grammar, typed, want) in cases {
    let mut keys = Keys::default();
    let got: Vec<_> = typed
      .chars()
      .map(|ch| keys.feed(key(ch), grammar))
      .map(|input| input.map(|i| (i.action, i.register)))
      .collect();
    assert_eq!(got, want, "{typed}");
  }
}

#[test]
fn objects_accept_quotes_counts_and_aliases() {
  for (name, kind) in [
    ("w", O::Word),
    ("W", O::BigWord),
    ("s", O::Sentence),
    ("p", O::Paragraph),
    ("(", O::Parentheses),
    (")", O::Parentheses),
    ("b", O::Parentheses),
    ("[", O::Brackets),
    ("]", O::Brackets),
    ("{", O::Braces),
    ("}", O::Braces),
    ("B", O::Braces),
    ("<", O::Angles),
    (">", O::Angles),
    ("\"", O::DoubleQuote),
    ("'", O::SingleQuote),
    ("“", O::DoubleQuote),
    ("’", O::SingleQuote),
  ] {
    for (prefix, around) in [("i", false), ("a", true)] {
      let mut keys = Keys::default();
      let input = format!("\"b3{prefix}{name}")
        .chars()
        .filter_map(|ch| keys.feed(key(ch), Grammar::Objects))
        .last()
        .unwrap();
      assert_eq!(input.action, Action::Object(kind, around));
      assert_eq!(input.count, 3);
      assert_eq!(input.register, 'b');
      assert!(clues(prefix).iter().any(|(key, _)| key == name));
    }
  }
}

#[test]
fn cursor_and_object_grammar() {
  let grammars = [
    Grammar::Normal,
    Grammar::Cursor,
    Grammar::Objects,
    Grammar::Panel,
    Grammar::Other,
  ];
  for grammar in grammars {
    let mut keys = Keys::default();
    let input = keys.feed(key('i'), grammar);
    match grammar {
      Grammar::Normal => {
        assert_eq!(input.unwrap().action, Action::Cursor);
        assert!(keys.awaiting().is_none());
      }
      Grammar::Objects => {
        assert!(input.is_none());
        assert_eq!(keys.awaiting(), Some("i"));
        assert_eq!(
          keys.feed(key('w'), grammar).unwrap().action,
          Action::Object(O::Word, false)
        );
      }
      Grammar::Cursor | Grammar::Panel | Grammar::Other => {
        assert!(input.is_none());
        assert!(keys.awaiting().is_none());
      }
    }
    keys.clear();
    keys.feed(key('a'), grammar);
    assert_eq!(keys.awaiting(), (grammar == Grammar::Objects).then_some("a"));
    keys.clear();
    for ch in "\"i2mi".chars() {
      let input = keys.feed(key(ch), grammar);
      if let Some(input) = input {
        assert_eq!(input.action, Action::Mark('i'));
        assert_eq!(input.register, 'i');
        assert_eq!(input.count, 2);
      }
    }
    assert_eq!(keys.pending(), "");
    keys.feed(key('g'), grammar);
    assert!(keys.feed(key('i'), grammar).is_none());
    assert_eq!(keys.pending(), "");
  }
}

#[test]
fn escape_resets_pending_count() {
  let mut keys = Keys::default();
  keys.feed(key('9'), Grammar::Objects);
  keys.feed(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), Grammar::Objects);
  assert_eq!(keys.feed(key('j'), Grammar::Objects).unwrap().count, 1);
  assert!(keys.feed(key('D'), Grammar::Objects).is_none());
  assert_eq!(
    keys.feed(key(' '), Grammar::Objects).unwrap().action,
    Action::Screen(1)
  );
  let shifted = KeyEvent::new(KeyCode::Char(' '), KeyModifiers::SHIFT);
  assert_eq!(
    keys.feed(shifted, Grammar::Objects).unwrap().action,
    Action::Screen(-1)
  );
}

#[test]
fn yank_clue_names_the_panel_continuation() {
  assert!(clues("y").iter().any(|(key, _)| key == "y"));
  assert_eq!(key_for(Action::YankLine), Some("yy"));
}

fn clues(prefix: &str) -> Vec<(String, &'static str)> {
  let mut keys = Keys::default();
  let grammar = if prefix == "y" { Grammar::Panel } else { Grammar::Objects };
  for key in parse(prefix).unwrap() {
    keys.feed(key, grammar);
  }
  keys.clues(grammar, |_| true)
}

#[test]
fn clues_preserve_pending_counts_and_registers() {
  for grammar in [
    Grammar::Normal,
    Grammar::Cursor,
    Grammar::Objects,
    Grammar::Panel,
    Grammar::Other,
  ] {
    for prefix in ["g", "z", "i", "a", "y", "C-w", "C-r", "[", "]"] {
      let mut keys = Keys::default();
      let mut remaining = 2;
      for key in parse(&format!("\"a2{prefix}")).unwrap() {
        if let Some(input) = keys.feed(key, grammar) {
          assert_eq!(input.count, 2);
          remaining = 1;
        }
      }
      let before = keys.pending();
      let mut clues = crate::clues::Clues::default();
      clues.show(
        keys
          .clues(grammar, |_| true)
          .into_iter()
          .map(|(key, about)| (key, about.to_owned()))
          .collect(),
      );
      let Some(clues) = clues.shown() else { continue };
      for (index, (suffix, _)) in clues.items().iter().enumerate() {
        let mut replay = keys.clone();
        let inputs: Vec<_> = clues
          .keys(index)
          .unwrap()
          .into_iter()
          .filter_map(|key| replay.feed(key, grammar))
          .collect();
        assert_eq!(inputs.len(), 1, "{prefix} {suffix}");
        assert_eq!(inputs[0].count, remaining);
        assert_eq!(inputs[0].register, 'a');
      }
      assert_eq!(keys.pending(), before);
    }
  }
}

#[test]
fn effective_yank_clues() {
  for (grammar, expected) in [
    (Grammar::Panel, vec!["p", "y"]),
    (Grammar::Normal, vec!["Y", "a", "f", "i", "p"]),
    (Grammar::Cursor, vec!["Y", "a", "f", "i", "p"]),
  ] {
    let mut keys = Keys::default();
    keys.feed(key('y'), grammar);
    let labels: Vec<_> = keys
      .effective_clues(|action| {
        grammar != Grammar::Panel
          || matches!(action, Action::Yank | Action::YankPath)
      })
      .into_iter()
      .map(|(sequence, _)| sequence.to_string())
      .collect();
    assert_eq!(labels, expected);
    assert_eq!(keys.awaiting(), Some("y"));
  }
}

#[test]
fn clues_preserve_yank_state() {
  let mut keys = Keys::default();
  for event in parse("\"a2y").unwrap() {
    assert!(keys.feed(event, Grammar::Panel).is_none());
  }
  let before = keys.pending();
  let clues = keys.effective_clues(|action| {
    matches!(action, Action::Yank | Action::YankPath)
  });
  assert_eq!(clues.len(), 2);
  for (sequence, _) in clues {
    let mut replay = keys.clone();
    let inputs: Vec<_> = sequence
      .keys()
      .iter()
      .filter_map(|&event| replay.feed(event, Grammar::Panel))
      .collect();
    assert_eq!(inputs.len(), 1);
    assert!(matches!(inputs[0].action, Action::YankLine | Action::YankPath));
    assert_eq!(inputs[0].count, 2);
    assert_eq!(inputs[0].register, 'a');
  }
  assert_eq!(keys.pending(), before);
}

#[test]
fn clues_follow_operator_remaps() {
  use crate::bindings::{Bindings, Context, Control, Map, Sequence, Target};

  let mut bindings = Bindings::default();
  let map = Map::new(
    [
      ("a", Target::Control(Control::Object { around: true })),
      ("i", Target::Control(Control::Object { around: false })),
      ("f", Target::Control(Control::Resize)),
      ("q", Target::Command("recent".into())),
      ("x", Target::Reader(Action::YankPath)),
    ]
    .into_iter()
    .map(|(sequence, target)| (Sequence::parse(sequence).unwrap(), target))
    .collect(),
  )
  .unwrap();
  bindings.set(Context::Operator, map);
  let mut keys = Keys::default();
  keys.feed_with(key('y'), Context::Cursor, &bindings);
  let labels: Vec<_> = keys
    .effective_clues(|_| true)
    .into_iter()
    .map(|(sequence, _)| sequence.to_string())
    .collect();
  assert_eq!(labels, ["a", "i", "q", "x"]);
  assert!(matches!(
    keys.feed_with(key('q'), Context::Cursor, &bindings),
    Some(Dispatch::Command(command)) if command == "recent"
  ));
}

#[test]
fn yank_clues_follow_the_live_operator_grammar() {
  let mut keys = Keys::default();
  keys.feed(key('y'), Grammar::Normal);
  let labels: Vec<_> = keys
    .clues(Grammar::Objects, |action| action != Action::Yank)
    .into_iter()
    .map(|(key, _)| key)
    .collect();
  assert_eq!(labels, ["Y", "f", "p"]);
  assert_eq!(keys.awaiting(), Some("y"));
  assert_eq!(
    keys.feed(key('y'), Grammar::Objects).unwrap().action,
    Action::Yank
  );
  let mut keys = Keys::default();
  keys.feed(key('i'), Grammar::Cursor);
  assert_eq!(keys.clues(Grammar::Cursor, |_| true), [] as [(String, &str); 0]);
}

#[test]
fn copy_hints_follow_yank_prefix() {
  for grammar in [Grammar::Normal, Grammar::Cursor, Grammar::Panel] {
    let mut keys = Keys::default();
    keys.feed(key('y'), grammar);
    let continuation =
      if grammar == Grammar::Panel { Grammar::Panel } else { Grammar::Objects };
    let input = keys.feed(key('f'), continuation).unwrap();
    assert_eq!(input.action, Action::Hints(true));
    assert_eq!(keys.pending(), "");
    assert!(keys.feed(key('F'), grammar).is_none());
    assert_eq!(
      keys.feed(key('f'), grammar).unwrap().action,
      Action::Hints(false)
    );
  }
}

#[test]
fn shifted_region_keys_preserve_counts() {
  use crate::bindings::{Bindings, Context};
  let bindings = Bindings::default();
  for (ch, dx, dy) in [('H', -1, 0), ('J', 0, 1), ('K', 0, -1), ('L', 1, 0)] {
    for event in [
      key(ch),
      KeyEvent::new(
        KeyCode::Char(ch.to_ascii_lowercase()),
        KeyModifiers::SHIFT,
      ),
    ] {
      let mut keys = Keys::default();
      assert!(keys.feed_with(key('3'), Context::Region, &bindings).is_none());
      let Some(Dispatch::Input(input)) =
        keys.feed_with(event, Context::Region, &bindings)
      else {
        panic!("Region input expected");
      };
      assert_eq!(input.action, Action::RegionShift(dx, dy));
      assert_eq!(input.count, 3);
    }
  }
  let mut keys = Keys::default();
  assert_eq!(keys.feed(key('K'), Grammar::Other).unwrap().action, Action::Peek);
  assert!(keys.feed_with(key('g'), Context::Region, &bindings).is_none());
  assert!(keys.feed_with(key('K'), Context::Region, &bindings).is_none());
}

#[test]
fn binding_actions_have_metadata() {
  for (_, action, _) in BINDINGS {
    let (spec, argument) = crate::bindings::actions::describe(*action).unwrap();
    assert_eq!(spec.resolve(argument).unwrap(), *action);
  }
}

#[test]
fn window_arrows_preserve_counts() {
  for (code, direction) in [
    (KeyCode::Left, D::Left),
    (KeyCode::Down, D::Down),
    (KeyCode::Up, D::Up),
    (KeyCode::Right, D::Right),
  ] {
    let arrow = KeyEvent::new(code, KeyModifiers::NONE);
    for (prefix, action) in
      [('w', W::Focus(direction)), ('r', W::Extend(direction))]
    {
      let mut keys = Keys::default();
      assert!(keys.feed(key('3'), Grammar::Normal).is_none());
      assert!(
        keys
          .feed(
            KeyEvent::new(KeyCode::Char(prefix), KeyModifiers::CONTROL),
            Grammar::Normal,
          )
          .is_none()
      );
      let input = keys.feed(arrow, Grammar::Normal).unwrap();
      assert_eq!(input.action, Action::Window(action));
      assert_eq!(input.count, 3);
      assert!(input.explicit);
      if prefix == 'r' {
        assert!(keys.resizing());
        let repeated = keys.feed(arrow, Grammar::Normal).unwrap();
        assert_eq!(repeated.action, Action::Window(action));
        assert_eq!(repeated.count, 1);
      } else {
        assert_eq!(keys.pending(), "");
      }
    }
  }
}

#[test]
fn counted_tree_keys() {
  for (typed, action, count) in [
    ("3h", Action::Motion(Motion::Left), 3),
    ("2l", Action::Motion(Motion::Right), 2),
    ("4zR", Action::Fold(Fold::ExpandAll), 4),
    ("5zM", Action::Fold(Fold::CollapseAll), 5),
    ("za", Action::OutlineAuto, 1),
    ("zr", Action::Rotate, 1),
    ("zc", Action::Crop, 1),
  ] {
    let mut keys = Keys::default();
    let inputs: Vec<_> = parse(typed)
      .unwrap()
      .into_iter()
      .filter_map(|key| keys.feed(key, Grammar::Panel))
      .collect();
    assert_eq!(inputs.len(), 1, "{typed}");
    assert_eq!((inputs[0].action, inputs[0].count), (action, count));
  }
}
