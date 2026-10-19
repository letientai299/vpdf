//! Reply fixtures. kitty's, `WezTerm`'s, and tmux's are what the probe
//! got from kitty 0.49.1 (the cell round through a pty without pixel
//! size), a `WezTerm` 20240203-110809-5046fc22 headless mux, and tmux 3.8
//! in kitty; the others follow each terminal's XTVERSION and protocol
//! sources.
use super::*;
use crate::decode::{Event, Parser};
use std::collections::VecDeque;

/// The replies in `bytes`, decoded as the reader does.
fn replies(bytes: &[u8]) -> Vec<Reply> {
  let mut events = VecDeque::new();
  Parser::default().feed(bytes, &mut events);
  events
    .into_iter()
    .filter_map(|event| match event {
      Event::Reply(reply) => Some(reply),
      _ => None,
    })
    .collect()
}

const DA1: &[u8] = b"\x1b[?62;22c";
const OK: &[u8] = b"\x1b_Gi=31;OK\x1b\\";

fn version(name: &str) -> Vec<u8> {
  format!("\x1bP>|{name}\x1b\\").into_bytes()
}

fn tmux_info(client: Option<&str>) -> Tmux {
  Tmux {
    version: vec![3, 8],
    client: client.map(str::to_owned),
    cell: Some(Cell { width: 7, height: 13 }),
    passthrough: Some(Passthrough::On),
    extended_keys: true,
    focus_events: true,
    origin: (0, 0),
  }
}

fn tmux_with(client: Option<&str>) -> Route {
  Route::Tmux(Some(tmux_info(client)))
}

#[test]
fn kitty_recorded() {
  let bytes = [
    &version("kitty(0.49.1)"),
    OK,
    b"\x1b[?2026;2$y\x1b[?1016;2$y\x1b[?2048;2$y\x1b[?0u",
    b"\x1b]22;1\x1b\\\x1b]72;t=q\x1b\\\x1b[?62;52;c",
    b"\x1b[6;13;7t\x1b[?62;52;c",
  ]
  .concat();
  let caps = caps(Route::Direct, &replies(&bytes));
  assert_eq!(
    caps,
    Caps {
      host: Host::Direct(Graphics::Ok),
      version: Some("kitty(0.49.1)".into()),
      keyboard: Some(0),
      sync: Support::Answered,
      pixels: Support::Answered,
      resize: Support::Answered,
      pointer: Support::Answered,
      drop: Support::Answered,
      clipboard: None,
      cell: Some(Cell { width: 7, height: 13 }),
      attributes: Some(vec![62, 52]),
    }
  );
  assert_eq!(caps.profile(), Profile::Graphics);
}

#[test]
fn wezterm_recorded() {
  let bytes = [
    &version("WezTerm 20240203-110809-5046fc22"),
    OK,
    b"\x1b[?2026;2$y\x1b[?1016;2$y\x1b[?2048;0$y\x1b[?0u",
    b"\x1b[?65;4;6;18;22c",
  ]
  .concat();
  let caps = caps(Route::Direct, &replies(&bytes));
  assert_eq!(
    caps,
    Caps {
      host: Host::Direct(Graphics::Ok),
      version: Some("WezTerm 20240203-110809-5046fc22".into()),
      keyboard: Some(0),
      sync: Support::Answered,
      pixels: Support::Answered,
      resize: Support::Missing,
      pointer: Support::Missing,
      drop: Support::Missing,
      clipboard: None,
      cell: None,
      attributes: Some(vec![65, 4, 6, 18, 22]),
    }
  );
  assert_eq!(caps.profile(), Profile::Graphics);
}

/// tmux answers DA1 itself and ignores the keyboard query.
#[test]
fn tmux_recorded() {
  let bytes = [
    &version("tmux 3.8"),
    b"\x1b[?2026;2$y\x1b[?1016;0$y\x1b[?2048;0$y\x1b[?1;2;4c".as_slice(),
  ]
  .concat();
  let caps = caps(tmux_with(Some("kitty(0.49.1)")), &replies(&bytes));
  assert_eq!(
    caps,
    Caps {
      host: Host::Tmux(Some(tmux_info(Some("kitty(0.49.1)")))),
      version: Some("tmux 3.8".into()),
      keyboard: None,
      sync: Support::Answered,
      pixels: Support::Missing,
      resize: Support::Missing,
      pointer: Support::Missing,
      drop: Support::Missing,
      clipboard: None,
      cell: None,
      attributes: Some(vec![1, 2, 4]),
    }
  );
  assert_eq!(caps.terminal(), Some("kitty(0.49.1)"));
  assert_eq!(caps.profile(), Profile::Graphics);
}

#[test]
fn profiles_follow_answers_and_names() {
  let lacks = |lack| Profile::Unsupported(lack);
  let named =
    |name: &str, graphics: &[u8]| [&version(name), graphics, DA1].concat();
  let cases: [(&str, Route, Vec<u8>, Profile); 18] = [
    ("ghostty", Route::Direct, named("ghostty 1.3.1", OK), Profile::Graphics),
    ("konsole", Route::Direct, named("Konsole 26.08.2", OK), Profile::Graphics),
    ("iterm2", Route::Direct, named("iTerm2 3.7.6", OK), Profile::CropQuirk),
    ("rio", Route::Direct, named("Rio 0.5.28", OK), Profile::Graphics),
    (
      "old rio",
      Route::Direct,
      named("Rio 0.5.26", OK),
      lacks(Lack::Terminal("Rio before 0.5.27 breaks Unicode placeholders")),
    ),
    (
      "warp",
      Route::Direct,
      named("Warp(v0.2026.03.25)", OK),
      lacks(Lack::Terminal("Warp lacks image crops and cell offsets")),
    ),
    (
      "foot",
      Route::Direct,
      named("foot(1.28.0)", b""),
      lacks(Lack::Graphics(None)),
    ),
    ("unknown", Route::Direct, named("newterm 1.0", OK), Profile::Graphics),
    (
      "silent",
      Route::Direct,
      named("newterm 1.0", b""),
      lacks(Lack::Graphics(None)),
    ),
    (
      "error",
      Route::Direct,
      named("newterm 1.0", b"\x1b_Gi=31;EINVAL:bad\x1b\\"),
      lacks(Lack::Graphics(Some("EINVAL:bad".into()))),
    ),
    ("no DA1", Route::Direct, version("kitty(0.49.1)"), lacks(Lack::Answer)),
    // An answer is a fact even when DA1's comes too late.
    ("OK, no DA1", Route::Direct, [OK].concat(), Profile::Graphics),
    ("screen", Route::Screen, Vec::new(), lacks(Lack::Screen)),
    (
      "tmux, kitty",
      tmux_with(Some("kitty(0.49.1)")),
      named("tmux 3.8", b""),
      Profile::Graphics,
    ),
    (
      "tmux, iterm2",
      tmux_with(Some("iTerm2 3.7.6")),
      named("tmux 3.8", b""),
      Profile::CropQuirk,
    ),
    (
      "nested tmux",
      tmux_with(Some("tmux 3.8")),
      named("tmux 3.8", b""),
      lacks(Lack::Client(Some("tmux 3.8".into()))),
    ),
    (
      "tmux failed",
      Route::Tmux(None),
      named("tmux 3.8", b""),
      lacks(Lack::Tmux),
    ),
    (
      "tmux, no client",
      tmux_with(None),
      named("tmux 3.8", b""),
      lacks(Lack::Client(None)),
    ),
  ];
  for (label, route, bytes, want) in cases {
    assert_eq!(caps(route, &replies(&bytes)).profile(), want, "{label}");
  }
}

#[test]
fn pointer_shapes_answer_or_ghostty() {
  let ghostty = || tmux_with(Some("ghostty 1.3.1"));
  let kitty = || tmux_with(Some("kitty(0.49.1)"));
  let cases: [(Route, &[u8], &str, Support); 8] = [
    (Route::Direct, b"\x1b]22;1\x1b\\", "kitty(0.49.1)", Support::Answered),
    (Route::Direct, b"\x1b]22;0\x1b\\", "kitty(0.49.1)", Support::Missing),
    (Route::Direct, b"", "kitty(0.49.1)", Support::Missing),
    (Route::Direct, b"", "ghostty 1.3.1", Support::Named),
    (Route::Direct, b"\x1b]22;1\x1b\\", "ghostty 1.3.1", Support::Answered),
    // Under tmux the client answers.
    (kitty(), b"\x1b]22;1\x1b\\", "tmux 3.8", Support::Answered),
    (kitty(), b"", "tmux 3.8", Support::Missing),
    (ghostty(), b"", "tmux 3.8", Support::Named),
  ];
  for (route, osc, name, want) in cases {
    let bytes = [&version(name), osc, DA1].concat();
    let caps = caps(route, &replies(&bytes));
    assert_eq!(caps.pointer, want, "{name} {}", osc.escape_ascii());
  }
}

#[test]
fn title_stack_follows_what_answered() {
  let cases = [
    (Route::Direct, "kitty(0.49.1)", true),
    (Route::Direct, "iTerm2 3.7.6", true),
    (Route::Direct, "ghostty 1.3.1", false),
    (Route::Direct, "WezTerm 20240203-110809-5046fc22", false),
    (Route::Direct, "Konsole 260802", false),
    // tmux keeps the title as the pane's, whatever its client.
    (tmux_with(Some("ghostty 1.3.1")), "tmux 3.8", true),
  ];
  for (route, name, want) in cases {
    let caps = caps(route, &replies(&named_da1(name)));
    assert_eq!(caps.title_stack(), want, "{name}");
  }
}

/// GNU screen gets no write; the cell round follows the first only
/// without TIOCGWINSZ pixels, and its report fills `cell`.
#[test]
fn rounds_ask_what_the_route_allows() -> Result<()> {
  let kitty = [&version("kitty(0.49.1)"), OK, DA1].concat();
  let cell = [b"\x1b[6;13;7t", DA1].concat();
  let cases = [
    (Route::Screen, false, 0, None),
    (Route::Direct, true, 1, None),
    (Route::Direct, false, 2, Some(Cell { width: 7, height: 13 })),
  ];
  for (route, pixels, want, cell_size) in cases {
    let mut asked = Vec::new();
    let caps = rounds(route.clone(), pixels, |request| {
      asked.push(request.to_vec());
      Ok(replies(if asked.len() == 1 { &kitty } else { &cell }))
    })?;
    assert_eq!(asked.len(), want, "{route:?} {pixels}");
    if want > 0 {
      assert_eq!(asked[0], request(&route));
    }
    if want > 1 {
      assert_eq!(asked[1], b"\x1b[16t");
    }
    assert_eq!(caps.cell, cell_size, "{route:?} {pixels}");
  }
  Ok(())
}

#[test]
fn modes_need_a_usable_state() {
  let cases = [
    (0, Support::Missing),
    (1, Support::Answered),
    (2, Support::Answered),
    (3, Support::Answered),
    (4, Support::Missing),
  ];
  for (state, want) in cases {
    let bytes = format!("\x1b[?2026;{state}$y\x1b[?62c");
    let caps = caps(Route::Direct, &replies(bytes.as_bytes()));
    assert_eq!(caps.sync, want, "{state}");
    assert_eq!(caps.pixels, Support::Missing);
  }
}

#[test]
fn cell_comes_from_a_cell_report() {
  let cases: [(&[u8], Option<Cell>); 4] = [
    (b"\x1b[6;13;7t", Some(Cell { width: 7, height: 13 })),
    (b"\x1b[4;468;840t", None),
    (b"\x1b[6;0;7t", None),
    (b"\x1b[6;99999;7t", None),
  ];
  for (report, want) in cases {
    let caps = caps(Route::Direct, &replies(&[report, DA1].concat()));
    assert_eq!(caps.cell, want, "{}", report.escape_ascii());
  }
}

/// Wayst resizes its window on `CSI 16 t`; iTerm2 answers in points.
#[test]
fn cell_query_only_when_safe() {
  let cases = [
    (Route::Direct, named_da1("kitty(0.49.1)"), false, true),
    (Route::Direct, named_da1("kitty(0.49.1)"), true, false),
    (Route::Direct, named_da1("Wayst(0.0.1)"), false, false),
    (Route::Direct, named_da1("iTerm2 3.7.6"), false, false),
    (Route::Direct, version("kitty(0.49.1)"), false, false),
    (Route::Direct, DA1.to_vec(), false, true),
    // tmux answers `CSI 16 t` itself; `Tmux.cell` has the client's.
    (tmux_with(Some("kitty(0.49.1)")), named_da1("tmux 3.8"), false, false),
  ];
  for (route, bytes, pixels, want) in cases {
    let caps = caps(route, &replies(&bytes));
    assert_eq!(asks_cell(&caps, pixels), want, "{caps:?} {pixels}");
  }
}

fn named_da1(name: &str) -> Vec<u8> {
  [&version(name), DA1].concat()
}

/// `TMUX_FORMAT` fields after the version: client, cell width and
/// height, passthrough, extended keys, focus events, pane left and top,
/// window offsets, status position and lines.
#[test]
fn tmux_reports_parse() {
  let kitty = Some("kitty(0.49.1)".to_owned());
  let cases = [
    (
      "3.8\tkitty(0.49.1)\t7\t13\ton\ton\t1\t0\t0\t\t\tbottom\ton\n",
      Some(Tmux {
        version: vec![3, 8],
        client: kitty.clone(),
        cell: Some(Cell { width: 7, height: 13 }),
        passthrough: Some(Passthrough::On),
        extended_keys: true,
        focus_events: true,
        origin: (0, 0),
      }),
    ),
    // tmux 3.8 with an empty config, in the right pane below a 9-row
    // one, the status line on top.
    (
      "3.8\tkitty(0.49.1)\t7\t13\toff\toff\t0\t41\t9\t\t\ttop\ton\n",
      Some(Tmux {
        version: vec![3, 8],
        client: kitty.clone(),
        cell: Some(Cell { width: 7, height: 13 }),
        passthrough: Some(Passthrough::Off),
        extended_keys: false,
        focus_events: false,
        origin: (41, 10),
      }),
    ),
    // A window larger than its client, panned right 20 and down 6, and
    // two status lines on top.
    (
      "3.8\tkitty(0.49.1)\t7\t13\tall\talways\t1\t41\t9\t20\t6\ttop\t2",
      Some(Tmux {
        version: vec![3, 8],
        client: kitty.clone(),
        cell: Some(Cell { width: 7, height: 13 }),
        passthrough: Some(Passthrough::All),
        extended_keys: true,
        focus_events: true,
        origin: (21, 5),
      }),
    ),
    // No client attached, and tmux before 3.3 without the option.
    (
      "3.2a\t\t\t\t\toff\t0\t0\t0\t\t\tbottom\toff\n",
      Some(Tmux {
        version: vec![3, 2],
        client: None,
        cell: None,
        passthrough: None,
        extended_keys: false,
        focus_events: false,
        origin: (0, 0),
      }),
    ),
    (
      "next-3.9\tWezTerm 20240203\t0\t0\tall\ton\t1\t5\t3\t9\t9\ttop\toff",
      Some(Tmux {
        version: vec![3, 9],
        client: Some("WezTerm 20240203".into()),
        cell: None,
        passthrough: Some(Passthrough::All),
        extended_keys: true,
        focus_events: true,
        origin: (0, 0),
      }),
    ),
    (
      "openbsd-7.8\tkitty(0.49.1)\t7\t13\toff\ton\t1\t0\t0\t\t\tbottom\ton",
      Some(Tmux {
        version: vec![],
        client: kitty,
        cell: Some(Cell { width: 7, height: 13 }),
        passthrough: Some(Passthrough::Off),
        extended_keys: true,
        focus_events: true,
        origin: (0, 0),
      }),
    ),
    ("\t\t\t\t\t\t\t\t\t\t\t\t\n", None),
    ("3.8\tkitty(0.49.1)\t7\t13\ton\n", None),
    ("3.8 kitty 7 13 on\n", None),
    ("", None),
  ];
  for (display, want) in cases {
    assert_eq!(tmux(display), want, "{display:?}");
  }
}

#[test]
fn screen_wins_over_tmux() {
  let tmux =
    || tmux("3.8\tkitty(0.49.1)\t7\t13\toff\ton\t1\t0\t0\t\t\tbottom\ton");
  let cases = [
    (&[][..], Route::Direct),
    (&["TMUX"][..], Route::Tmux(tmux())),
    (&["STY"][..], Route::Screen),
    (&["STY", "TMUX"][..], Route::Screen),
  ];
  for (set, want) in cases {
    let mut asked = false;
    let route = route(
      |name| set.contains(&name),
      || {
        asked = true;
        tmux()
      },
    );
    assert_eq!(route, want, "{set:?}");
    assert_eq!(asked, set == ["TMUX"], "{set:?}");
  }
}

/// No graphics query goes to tmux, where its answer would land in the
/// active pane as keys.
#[test]
fn requests_per_route() {
  let direct = b"\x1b[>0q\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\
    \x1b[?2026$p\x1b[?1016$p\x1b[?2048$p\x1b[?u\
    \x1b]72;t=q\x1b\\\x1b]22;?pointer\x1b\\\x1b[?5522$p";
  assert_eq!(request(&Route::Direct), direct);
  let tmux = request(&tmux_with(None));
  assert!(!tmux.windows(3).any(|window| window == b"\x1b_G"));
}

#[test]
fn versions_compare_numerically() {
  let cases = [
    ("kitty(0.49.1)", "kitty", vec![0, 49, 1]),
    (
      "WezTerm 20240203-110809-5046fc22",
      "wezterm",
      vec![20_240_203, 110_809, 5046, 22],
    ),
    ("iTerm2 3.5.10beta1", "iterm2", vec![3, 5, 10, 1]),
    ("xterm.js(6.1.0)", "xterm.js", vec![6, 1, 0]),
    ("foot", "foot", vec![]),
  ];
  for (xtversion, name, version) in cases {
    assert_eq!(identify(xtversion), (name.into(), version), "{xtversion}");
  }
  assert_eq!(named("iTerm2 3.5.10"), Named::Crop);
  assert_eq!(named("iTerm2 3.5.5"), named("iTerm2 3.4"));
}

#[test]
fn placeholders_follow_name_and_version() {
  let cases = [
    ("kitty(0.49.1)", true),
    ("kitty(0.27.1)", false),
    ("kitty", false),
    ("ghostty 1.3.1", true),
    ("iTerm2 3.5.6beta2", true),
    ("iTerm2 3.5.5", false),
    ("Rio 0.5.27", true),
    ("Rio 0.5.26", false),
    ("WezTerm 20240203-110809-5046fc22", false),
    ("Konsole 26.08.2", false),
    ("Zellij 0.45.1", false),
    ("tmux 3.8", false),
  ];
  for (xtversion, want) in cases {
    assert_eq!(placeholders(xtversion), want, "{xtversion}");
  }
}

/// Asserts `caps` shows each `present` line and no line starting
/// with an `absent` prefix.
fn shows(name: &str, caps: &Caps, present: &[&str], absent: &[&str]) {
  let text = caps.to_string();
  for line in present {
    assert!(text.lines().any(|shown| shown == *line), "{name}: {line}\n{text}");
  }
  for prefix in absent {
    let shown = text.lines().any(|shown| shown.starts_with(prefix));
    assert!(!shown, "{name}: {prefix}\n{text}");
  }
}

#[test]
fn info_without_tmux() {
  let kitty = [
    &version("kitty(0.49.1)"),
    OK,
    b"\x1b[?2026;2$y\x1b[?1016;2$y\x1b[?2048;2$y\x1b[?0u",
    b"\x1b]22;1\x1b\\\x1b]72;t=q\x1b\\\x1b[6;13;7t\x1b[?62;52;c",
  ]
  .concat();
  assert_eq!(
    caps(Route::Direct, &replies(&kitty)).to_string(),
    "Profile: Kitty graphics\n\
     XTVERSION: kitty(0.49.1)\n\
     Graphics query: OK\n\
     Kitty keyboard: yes, flags 0 at start\n\
     Synchronized output: yes\n\
     Pixel mouse: yes\n\
     Resize reports: yes\n\
     Pointer shapes: yes\n\
     Drag and drop: yes\n\
     Image clipboard: no answer\n\
     Cell reply: 7x13 px\n\
     DA1: 62;52"
  );
  let wezterm = [
    &version("WezTerm 20240203-110809-5046fc22"),
    OK,
    b"\x1b[?2026;2$y\x1b[?1016;2$y\x1b[?2048;0$y\x1b[?0u\x1b[?65;4;6c",
  ]
  .concat();
  let named =
    |name: &str, graphics: &[u8]| [&version(name), graphics, DA1].concat();
  let cases = [
    (
      "wezterm",
      Route::Direct,
      wezterm,
      &[
        "Profile: Kitty graphics",
        "Resize reports: no",
        "Pointer shapes: no",
        "DA1: 65;4;6",
      ][..],
    ),
    (
      "ghostty",
      Route::Direct,
      named("ghostty 1.3.1", OK),
      &["Profile: Kitty graphics", "Pointer shapes: yes, by name"],
    ),
    (
      "iterm2",
      Route::Direct,
      named("iTerm2 3.7.6", OK),
      &["Profile: Kitty graphics, iTerm2 crop sizing"],
    ),
    (
      "foot",
      Route::Direct,
      named("foot(1.28.0)", b""),
      &[
        "Profile: unsupported (No Kitty graphics)",
        "Graphics query: no answer",
        "Kitty keyboard: no",
      ],
    ),
    (
      "error",
      Route::Direct,
      named("newterm 1.0", b"\x1b_Gi=31;EINVAL:bad\x1b\\"),
      &[
        "Profile: unsupported (Kitty graphics failed: EINVAL:bad)",
        "Graphics query: failed: EINVAL:bad",
      ],
    ),
    (
      "warp error",
      Route::Direct,
      named("Warp(v0.2026.03.25)", b"\x1b_Gi=31;EINVAL:bad\x1b\\"),
      &[
        "Profile: unsupported (Warp lacks image crops and cell offsets)",
        "Graphics query: failed: EINVAL:bad",
      ],
    ),
    (
      "screen",
      Route::Screen,
      Vec::new(),
      &[
        "Profile: unsupported (GNU screen cannot pass Kitty graphics)",
        "XTVERSION: none",
        "GNU screen: not queried",
        "DA1: no answer",
      ],
    ),
  ];
  for (name, route, bytes, present) in cases {
    shows(name, &caps(route, &replies(&bytes)), present, &["tmux", "Cell"]);
  }
}

/// A line break in an answer cannot add a row or a heading.
#[test]
fn info_keeps_answers_on_their_row() {
  let bytes = [
    &version("evil\n# Metadata\nProfile: Kitty graphics"),
    b"\x1b_Gi=31;EINVAL:a\nb\x1b\\".as_slice(),
    DA1,
  ]
  .concat();
  let caps = caps(Route::Direct, &replies(&bytes));
  let present = [
    "XTVERSION: evil # Metadata Profile: Kitty graphics",
    "Profile: unsupported (Kitty graphics failed: EINVAL:a b)",
  ];
  shows("direct", &caps, &present, &["#"]);
  let tmux = Tmux { client: Some("x\ny".into()), ..tmux_info(None) };
  let caps = Caps { host: Host::Tmux(Some(tmux)), ..caps };
  shows("tmux", &caps, &["tmux client: x y"], &["y"]);
}

#[test]
fn info_under_tmux() {
  let bytes = |name: &str| {
    [&version(name), b"\x1b[?2026;2$y\x1b[?1;2;4c".as_slice()].concat()
  };
  let cases = [
    ("tmux 3.8", vec![3, 8], Some(Passthrough::Off), "3.8", "off"),
    ("tmux 3.8", vec![3, 8], Some(Passthrough::On), "3.8", "on"),
    ("tmux 3.8", vec![3, 8], Some(Passthrough::All), "3.8", "all"),
    ("tmux 3.2a", vec![3, 2], None, "3.2", "on, no option before tmux 3.3"),
    ("tmux openbsd-7.8", vec![], None, "unknown release", "unknown"),
  ];
  for (xtversion, version, passthrough, release, setting) in cases {
    let tmux =
      Tmux { version, passthrough, ..tmux_info(Some("kitty(0.49.1)")) };
    let line = format!("tmux passthrough: {setting}");
    let present = [
      "Profile: Kitty graphics",
      &format!("XTVERSION: {xtversion}"),
      &format!("tmux: {release}"),
      "tmux client: kitty(0.49.1)",
      "tmux cell: 7x13 px",
      &line,
      "tmux extended keys: on",
      "tmux focus events: on",
      "Kitty keyboard: no",
    ];
    let caps = caps(Route::Tmux(Some(tmux)), &replies(&bytes(xtversion)));
    shows(&line, &caps, &present, &["Graphics query", "GNU screen"]);
  }
  let bytes = bytes("tmux 3.8");
  let no_client = caps(tmux_with(None), &replies(&bytes));
  let present = [
    "Profile: unsupported (tmux names no client terminal)",
    "tmux client: none",
  ];
  shows("no client", &no_client, &present, &[]);
  let failed = caps(Route::Tmux(None), &replies(&bytes));
  let present =
    ["Profile: unsupported (Cannot query tmux)", "tmux: cannot query"];
  shows("tmux failed", &failed, &present, &["tmux passthrough"]);
}

#[test]
fn info_names_what_vpdf_set() {
  let tmux = |passthrough, version| Tmux {
    version,
    passthrough,
    ..tmux_info(Some("kitty(0.49.1)"))
  };
  let cases = [
    (Some(Passthrough::Off), vec![3, 8], None, "off"),
    (
      Some(Passthrough::Off),
      vec![3, 8],
      Some(Passthrough::On),
      "off at start, vpdf set on",
    ),
    (
      Some(Passthrough::On),
      vec![3, 8],
      Some(Passthrough::All),
      "on at start, vpdf set all",
    ),
    (None, vec![3, 2], None, "on, no option before tmux 3.3"),
  ];
  for (start, version, set, want) in cases {
    assert_eq!(tmux(start, version).passthrough_row(set), want, "{want}");
  }
}

/// Hidden-pane deletes require passthrough from invisible panes.
#[test]
fn passthrough_allows_hidden_cleanup() {
  use Passthrough::{All, Off, On};
  let cases = [
    (Some(Off), true, Some(All)),
    (Some(On), true, Some(All)),
    (Some(All), true, None),
    (Some(Off), false, Some(All)),
    (Some(On), false, Some(All)),
    (Some(All), false, None),
    (None, true, None),
  ];
  for (passthrough, focus_events, want) in cases {
    let tmux = Tmux { passthrough, focus_events, ..tmux_info(None) };
    assert_eq!(tmux.wanted(), want, "{passthrough:?} {focus_events}");
  }
}

/// tmux asks its client for modifyOtherKeys, which kitty ignores.
#[test]
fn kitty_under_tmux_merges_ctrl_i() {
  let tmux = [&version("tmux 3.8"), DA1].concat();
  let cases = [
    (tmux_with(Some("kitty(0.49.1)")), &tmux, true),
    (tmux_with(Some("WezTerm 20240203-110809-5046fc22")), &tmux, false),
    (tmux_with(None), &tmux, false),
    (Route::Direct, &[&version("kitty(0.49.1)"), OK, DA1].concat(), false),
  ];
  for (route, bytes, want) in cases {
    let caps = caps(route.clone(), &replies(bytes));
    assert_eq!(caps.merges_ctrl_i(), want, "{route:?}");
  }
}

/// Keyboard support comes from its own answers, never from graphics:
/// tmux and `WezTerm` without `enable_kitty_keyboard` draw but miss it.
#[test]
fn keys_need_disambiguation_enabled() {
  let wezterm = "WezTerm 20240203-110809-5046fc22";
  let cases = [
    (
      "kitty",
      Route::Direct,
      [&version("kitty(0.49.1)"), OK, b"\x1b[?0u", DA1].concat(),
      Some(KEYBOARD),
      None,
    ),
    (
      "zellij, flag 1 only",
      Route::Direct,
      [&version("Zellij 0.45.1"), b"\x1b[?0u".as_slice(), DA1].concat(),
      Some(DISAMBIGUATE),
      None,
    ),
    (
      "refused",
      Route::Direct,
      [&version("kitty(0.49.1)"), OK, b"\x1b[?0u", DA1].concat(),
      Some(ALTERNATE_KEYS),
      Some(KeyLack::Refused(Some(ALTERNATE_KEYS))),
    ),
    (
      "unconfirmed",
      Route::Direct,
      [b"\x1b[?0u", DA1].concat(),
      None,
      Some(KeyLack::Refused(None)),
    ),
    (
      "tmux",
      tmux_with(Some("kitty(0.49.1)")),
      [&version("tmux 3.8"), DA1].concat(),
      None,
      Some(KeyLack::Protocol(Some("tmux 3.8".into()))),
    ),
    (
      "WezTerm default",
      Route::Direct,
      [&version(wezterm), OK, DA1].concat(),
      None,
      Some(KeyLack::Protocol(Some(wezterm.into()))),
    ),
    ("screen", Route::Screen, Vec::new(), None, Some(KeyLack::Screen)),
    (
      "no DA1",
      Route::Direct,
      version("kitty(0.49.1)"),
      None,
      Some(KeyLack::Unanswered),
    ),
    (
      "flags without DA1",
      Route::Direct,
      [&version("kitty(0.49.1)"), b"\x1b[?0u".as_slice()].concat(),
      Some(KEYBOARD),
      None,
    ),
  ];
  for (label, route, bytes, enabled, want) in cases {
    let caps = caps(route, &replies(&bytes));
    assert_eq!(caps.key_lack(enabled), want, "{label}");
  }
  let messages = [
    (KeyLack::Screen, "GNU screen lacks Kitty keyboard disambiguation"),
    (KeyLack::Unanswered, "Terminal did not answer queries"),
    (
      KeyLack::Protocol(Some("tmux 3.8".into())),
      "tmux 3.8 lacks Kitty keyboard disambiguation",
    ),
    (KeyLack::Protocol(None), "Terminal lacks Kitty keyboard disambiguation"),
    (
      KeyLack::Protocol(Some(wezterm.into())),
      "WezTerm needs enable_kitty_keyboard = true",
    ),
    (
      KeyLack::Refused(Some(4)),
      "Kitty keyboard disambiguation refused, flags 4",
    ),
    (KeyLack::Refused(None), "Terminal did not confirm Kitty keyboard flags"),
    (
      KeyLack::ExtendedKeys,
      "tmux needs `set -s extended-keys on` in tmux.conf",
    ),
    (KeyLack::TmuxRelease, "tmux before 3.2 lacks extended keys"),
  ];
  for (lack, want) in messages {
    assert_eq!(lack.to_string(), want);
  }
}

/// Label, route, probe replies, readback per round, and result.
type Negotiation<'a> =
  (&'a str, Route, &'a [u8], &'a [&'a [u8]], Option<KeyLack>);

/// The push and its readback go out in one round, and only where the
/// probe found the protocol. Rounds without disambiguation, as when a
/// child program left its answers unread, repeat the readback up to
/// `READBACKS` times; the last flags win. tmux never answers `CSI ? u`:
/// with `extended-keys` it gets modifyOtherKeys mode 2 instead, and
/// without, nothing.
#[test]
fn negotiate_pushes_then_reads_back() -> Result<()> {
  let kitty = [&version("kitty(0.49.1)"), OK, b"\x1b[?0u", DA1].concat();
  let tmux = [&version("tmux 3.8"), DA1].concat();
  let (on, off) = (b"\x1b[?29u".as_slice(), b"\x1b[?4u".as_slice());
  let refused = Some(KeyLack::Refused(Some(ALTERNATE_KEYS)));
  let direct = || Route::Direct;
  let plain = Tmux { extended_keys: false, ..tmux_info(None) };
  let old = Tmux { version: vec![3, 1], ..plain.clone() };
  let cases: [Negotiation; 11] = [
    ("accepted", direct(), &kitty, &[on], None),
    ("refused", direct(), &kitty, &[off, off, off], refused.clone()),
    ("refused, then silent", direct(), &kitty, &[off, b"", b""], refused),
    (
      "unconfirmed",
      direct(),
      &kitty,
      &[b"", b"", b""],
      Some(KeyLack::Refused(None)),
    ),
    ("stale DA1", direct(), &kitty, &[b"", on], None),
    ("stale flags first", direct(), &kitty, &[b"\x1b[?0u\x1b[?29u"], None),
    ("stale flags round", direct(), &kitty, &[b"\x1b[?0u", b"", on], None),
    ("tmux", tmux_with(Some("kitty(0.49.1)")), &tmux, &[b""], None),
    (
      "tmux, extended-keys off",
      Route::Tmux(Some(plain)),
      &tmux,
      &[],
      Some(KeyLack::ExtendedKeys),
    ),
    (
      "tmux 3.1",
      Route::Tmux(Some(old)),
      &tmux,
      &[],
      Some(KeyLack::TmuxRelease),
    ),
    // `Lack::Tmux` stops the start alone.
    ("tmux failed", Route::Tmux(None), &tmux, &[], None),
  ];
  for (label, route, probed, readbacks, want) in cases {
    let tmux = matches!(route, Route::Tmux(Some(_)));
    let caps = caps(route, &replies(probed));
    let mut asked = Vec::new();
    let lack = caps.negotiate(|request| {
      asked.push(request.to_vec());
      Ok(replies(&[readbacks[asked.len() - 1], DA1].concat()))
    })?;
    assert_eq!(lack, want, "{label}");
    assert_eq!(asked.len(), readbacks.len(), "{label}");
    let mut requests = asked.iter();
    let push: &[u8] =
      if tmux { MODIFY_OTHER_KEYS } else { b"\x1b[>29u\x1b[?u" };
    if let Some(first) = requests.next() {
      assert_eq!(first, push, "{label}");
    }
    assert!(requests.all(|request| request == b"\x1b[?u"), "{label}");
  }
  Ok(())
}

#[test]
fn clipboard_requires_explicit_answer() {
  for route in [Route::Direct, tmux_with(Some("kitty(0.49.1)"))] {
    let unanswered = caps(route.clone(), &replies(DA1));
    assert_eq!(unanswered.clipboard, None);
    assert!(unanswered.to_string().contains("Image clipboard: no answer"));
    for (state, want) in [
      (0, Support::Missing),
      (1, Support::Answered),
      (2, Support::Answered),
      (3, Support::Answered),
      (4, Support::Missing),
    ] {
      let bytes = format!("\x1b[?5522;{state}$y");
      assert_eq!(
        caps(route.clone(), &replies(bytes.as_bytes())).clipboard,
        Some(want)
      );
    }
  }
  let mut wrapped = Vec::new();
  let query = b"\x1b]22;?pointer\x1b\\\x1b[?5522$p";
  crate::terminal::passthrough(&mut wrapped, query, true).unwrap();
  assert!(request(&tmux_with(None)).ends_with(&wrapped));
}

#[test]
fn pixel_mouse_needs_matching_units() {
  let mode = |state| format!("\x1b[?1016;{state}$y").into_bytes();
  // (XTVERSION, DECRQM state) -> pixel reports in TIOCGWINSZ units
  let cases = [
    ("kitty(0.49.1)", 2, true),
    ("ghostty 1.3.1", 1, true),
    ("kitty(0.49.1)", 0, false),
    // iTerm2 reports points against device pixels.
    ("iTerm2 3.7.6", 2, false),
    ("WezTerm 20240203-110809-5046fc22", 2, false),
  ];
  for (name, state, want) in cases {
    let bytes = [named_da1(name), mode(state)].concat();
    let caps = caps(Route::Direct, &replies(&bytes));
    assert_eq!(caps.pixel_mouse(), want, "{name} {state}");
  }
}

#[test]
fn cell_offsets_follow_outer_terminal() {
  for (name, expected) in [
    ("WezTerm 20240203-110809-5046fc22", true),
    ("kitty(0.49.2)", false),
    ("iTerm2 3.7.6", false),
    ("ghostty 1.3.1", false),
    ("Rio 0.5.28", false),
  ] {
    let direct = caps(Route::Direct, &replies(&version(name)));
    assert_eq!(direct.cell_offsets(), expected);
    let mux = caps(tmux_with(Some(name)), &replies(&version("tmux 3.8")));
    assert_eq!(mux.cell_offsets(), expected);
  }
}

#[test]
fn wire_tmux_without_metadata() -> Result<()> {
  let caps = caps(
    Route::Direct,
    &replies(&[version("tmux 3.8"), DA1.to_vec()].concat()),
  );
  assert_eq!(caps.host, Host::Tmux(None));
  assert_eq!(caps.terminal(), None);
  assert_eq!(caps.profile(), Profile::Unsupported(Lack::Tmux));
  assert_eq!(caps.negotiate(|_| panic!("unverified keys"))?, None);
  Ok(())
}

#[test]
fn passthrough_reset_preserves_changes() {
  use Passthrough::{All, Off, On};
  use std::cell::Cell as State;
  for (changed, current, expected) in [
    (Some(All), Some(All), Some(On)),
    (Some(All), Some(Off), Some(Off)),
    (Some(All), Some(On), Some(On)),
    (Some(All), None, None),
    (None, Some(All), Some(All)),
  ] {
    let state = State::new(current);
    let queried = State::new(false);
    reset_passthrough_with(
      changed,
      || {
        queried.set(true);
        state.get()
      },
      |value| state.set(Some(value)),
    );
    assert_eq!(state.get(), expected);
    assert_eq!(queried.get(), changed == Some(All));
  }
}
