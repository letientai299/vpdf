use super::lifecycle::*;
use super::metrics::*;
use super::output::*;
use super::pointer::*;
use super::tmux::*;
use super::*;

#[test]
fn cell_size_never_guesses() {
  use crate::caps::{Graphics, Tmux};
  let cell = |width, height| Some(Cell { width, height });
  let caps = |host, asked| Caps {
    host,
    version: None,
    keyboard: None,
    sync: Support::Missing,
    pixels: Support::Missing,
    resize: Support::Missing,
    pointer: Support::Missing,
    drop: Support::Missing,
    clipboard: None,
    cell: asked,
    attributes: Some(vec![62]),
  };
  let direct = caps(Host::Direct(Graphics::Ok), cell(9, 19));
  let unasked = caps(Host::Direct(Graphics::Ok), None);
  // The probe's client cell is stale; tmux is asked again.
  let tmux = caps(
    Host::Tmux(Some(Tmux {
      version: vec![3, 8],
      client: None,
      cell: cell(5, 11),
      passthrough: None,
      extended_keys: true,
      focus_events: true,
      origin: (0, 0),
    })),
    None,
  );
  let failed = caps(Host::Tmux(None), None);
  let fake = Some(TMUX_FAKE);
  // (window, told, caps, TMUX set, client now, want)
  let cases = [
    (cell(8, 17), cell(10, 20), Some(&direct), false, None, cell(8, 17)),
    (None, cell(10, 20), Some(&direct), false, None, cell(10, 20)),
    (None, None, Some(&direct), false, None, cell(9, 19)),
    (None, None, Some(&unasked), false, None, None),
    (fake, None, Some(&direct), false, None, fake),
    (cell(7, 13), None, Some(&tmux), false, cell(6, 12), cell(7, 13)),
    (fake, None, Some(&tmux), false, cell(7, 13), cell(7, 13)),
    (fake, None, Some(&tmux), false, fake, fake),
    (fake, cell(10, 20), Some(&tmux), false, None, None),
    (None, None, Some(&tmux), false, cell(7, 13), cell(7, 13)),
    (fake, None, Some(&failed), true, None, None),
    (fake, None, None, true, cell(7, 13), cell(7, 13)),
    (fake, None, None, true, None, None),
    (cell(9, 18), None, None, true, None, cell(9, 18)),
    (fake, None, None, false, cell(7, 13), fake),
    (None, None, None, false, None, None),
  ];
  for (index, (window, told, caps, tmux, client, want)) in
    cases.into_iter().enumerate()
  {
    let got = pick(window, told, caps, tmux, || client);
    assert_eq!(got, want, "case {index}");
  }
}

#[test]
fn screen_preserves_window_metrics() -> Result<()> {
  let caps = tmux_caps(vec![3, 8], "kitty", true);
  let Host::Tmux(Some(mut meta)) = caps.host else { unreachable!() };
  meta.origin = (17, 4);
  meta.cell = Some(Cell { width: 8, height: 22 });
  for (pixels, client, expected) in [
    ((900, 1200), meta.cell, Some((9, 20))),
    ((900, 1200), None, Some((9, 20))),
    ((1600, 1920), meta.cell, Some((8, 22))),
    ((0, 0), meta.cell, Some((8, 22))),
    ((1600, 1920), None, None),
  ] {
    meta.cell = client;
    let window =
      WindowSize { columns: 100, rows: 60, width: pixels.0, height: pixels.1 };
    let screen = screen_with(&window, Some(&meta));
    match expected {
      Some((width, height)) => {
        let screen = screen?;
        assert_eq!((screen.cell_width, screen.cell_height), (width, height));
        assert_eq!((screen.columns, screen.rows), (100, 60));
        assert_eq!(screen.origin, (17, 4));
      }
      None => assert!(screen.is_err()),
    }
  }
  Ok(())
}

fn tmux_caps(version: Vec<u32>, client: &str, focus_events: bool) -> Caps {
  use crate::caps::Tmux;
  Caps {
    host: Host::Tmux(Some(Tmux {
      version,
      client: Some(client.into()),
      cell: None,
      passthrough: None,
      extended_keys: true,
      focus_events,
      origin: (0, 0),
    })),
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
}

/// tmux 3.7 sends passthrough ahead of text it holds in a frame, and
/// 3.7b and 3.7c drop the next image after one ends.
#[test]
fn no_frames_under_tmux_before_3_8() {
  let tmux = |version| tmux_caps(version, "kitty(0.49.1)", true);
  let direct =
    Caps { host: Host::Direct(crate::caps::Graphics::Ok), ..tmux(vec![]) };
  let cases = [
    (None, true),
    (Some(direct), true),
    (Some(tmux(vec![3, 8])), true),
    (Some(tmux(vec![3, 9])), true),
    (Some(tmux(vec![])), true),
    (Some(tmux(vec![3, 7])), false),
    (Some(tmux(vec![3, 3])), false),
  ];
  for (caps, want) in cases {
    assert_eq!(synced(caps.as_ref()), want, "{caps:?}");
  }
}

#[test]
fn tmux_notices_say_what_lacks() {
  let set = || Some("Set tmux allow-passthrough all".to_owned());
  let off = "tmux focus-events off: pointer cleanup waits for polling";
  let ctrl_i = "Ctrl-i is Tab under tmux in Kitty";
  let cases = [
    (
      tmux_caps(vec![3, 8], "kitty(0.49.1)", true),
      set(),
      vec![set().unwrap(), ctrl_i.into()],
    ),
    (
      tmux_caps(vec![3, 8], "kitty(0.49.1)", false),
      None,
      vec![off.into(), ctrl_i.into()],
    ),
    (tmux_caps(vec![3, 8], "WezTerm 20240203", true), None, vec![]),
  ];
  for (caps, passthrough, want) in cases {
    assert_eq!(tmux_notices(&caps, passthrough), want);
  }
}

/// `:mouse off` undoes each mode and shape `on` set, once.
#[test]
fn mouse_capture_undoes_itself() -> Result<()> {
  use crate::caps::Graphics;
  // (XTVERSION, pixels, shapes) -> (1016 written, push, pop)
  let cases = [
    (
      "kitty(0.49.1)",
      Support::Answered,
      Support::Answered,
      true,
      ">default",
      "<",
    ),
    ("ghostty 1.3", Support::Answered, Support::Named, true, "default", "text"),
    ("WezTerm", Support::Missing, Support::Missing, false, "", ""),
  ];
  for (version, pixels, pointer, pixel, push, pop) in cases {
    let caps = Caps {
      host: Host::Direct(Graphics::Ok),
      version: Some(version.into()),
      keyboard: Some(0),
      sync: Support::Answered,
      pixels,
      resize: Support::Answered,
      pointer,
      drop: Support::Missing,
      clipboard: None,
      cell: None,
      attributes: None,
    };
    let (mut on, mut off, mut again) = (Vec::new(), Vec::new(), Vec::new());
    grab_mouse(&mut on, &caps)?;
    free_mouse(&mut off)?;
    // Suspend after `:mouse off` frees again.
    free_mouse(&mut again)?;
    let (on, off) = (String::from_utf8(on)?, String::from_utf8(off)?);
    for mode in ["1000", "1002", "1003", "1006"] {
      assert!(on.contains(&format!("\x1b[?{mode}h")), "{version} {mode}");
      assert!(off.contains(&format!("\x1b[?{mode}l")), "{version} {mode}");
    }
    assert_eq!(on.contains("\x1b[?1016h"), pixel, "{version}");
    assert_eq!(off.contains("\x1b[?1016l"), pixel, "{version}");
    let shape = |text: &str, body: &str| {
      body.is_empty() || text.contains(&format!("\x1b]22;{body}\x1b"))
    };
    assert!(shape(&on, push) && shape(&off, pop), "{version}");
    assert_eq!(on.contains("\x1b]22;"), !push.is_empty(), "{version}");
    let again = String::from_utf8(again)?;
    assert!(!again.contains("1016") && !again.contains("\x1b]22;"));
  }
  Ok(())
}

#[test]
fn shapes_write_only_changes() {
  use Shape::{Default, Pointer, Text};
  #[derive(Clone, Copy, Debug)]
  enum Step {
    Push,
    Point(Shape),
    Forget,
    Pop,
  }
  let steps = [
    Step::Point(Text),
    Step::Push,
    Step::Push,
    Step::Point(Default),
    Step::Point(Text),
    Step::Point(Text),
    Step::Forget,
    Step::Point(Text),
    Step::Point(Pointer),
    Step::Pop,
    Step::Point(Text),
    Step::Pop,
  ];
  let run = |support, tmux| {
    let mut shapes = Shapes { owner: None, shown: None };
    let mut out = Vec::new();
    for step in steps {
      match step {
        Step::Push => shapes.push(support, &mut out, tmux),
        Step::Point(shape) => shapes.point(shape, &mut out, tmux),
        Step::Forget => {
          shapes.forget();
          Ok(())
        }
        Step::Pop => shapes.pop(&mut out, tmux),
      }
      .unwrap();
    }
    String::from_utf8(out).unwrap()
  };
  let osc = |bodies: &[&str], tmux| {
    let mut out = Vec::new();
    for body in bodies {
      osc22(&mut out, body, tmux).unwrap();
    }
    String::from_utf8(out).unwrap()
  };
  let kitty = [">default", "text", "text", "pointer", "<"];
  // Ghostty starts and ends at `text`.
  let ghostty = ["default", "text", "text", "pointer", "text"];
  // (support, tmux) -> written
  let cases = [
    (Support::Answered, false, osc(&kitty, false)),
    (Support::Named, false, osc(&ghostty, false)),
    (Support::Missing, false, String::new()),
    (Support::Answered, true, osc(&kitty, true)),
  ];
  for (support, tmux, want) in cases {
    assert_eq!(run(support, tmux), want, "{support:?} {tmux}");
  }
  let mut wrapped = Vec::new();
  osc22(&mut wrapped, "<", true).unwrap();
  assert_eq!(wrapped, b"\x1bPtmux;\x1b\x1b]22;<\x1b\x1b\\\x1b\\");
}

#[test]
fn tmux_view_words() {
  let cases = [
    ("hidden", Some(View::Hidden)),
    ("inactive", Some(View::Inactive)),
    ("focused", Some(View::Active { focused: true })),
    ("away", Some(View::Active { focused: false })),
    ("", None),
    ("1", None),
  ];
  for (word, want) in cases {
    assert_eq!(view(word), want, "{word}");
  }
}

#[test]
fn resizes_ask_again_without_pixels() {
  let some = Some(Cell { width: 8, height: 16 });
  // (window cell, resize queued, want)
  let cases = [(None, false, true), (None, true, false), (some, false, false)];
  for (window, queued, want) in cases {
    assert_eq!(asks_again(window, queued), want, "{window:?} {queued}");
  }
}

#[test]
fn window_pixels_divide_into_cells() {
  let window =
    |columns, rows, width, height| WindowSize { rows, columns, width, height };
  let cases = [
    (window(80, 24, 640, 384), Some(Cell { width: 8, height: 16 })),
    (window(3, 2, 2, 1), Some(Cell { width: 1, height: 1 })),
    (window(80, 24, 0, 384), None),
    (window(80, 24, 640, 0), None),
    (window(0, 24, 640, 384), None),
  ];
  for (window, want) in cases {
    assert_eq!(window_cell(&window), want, "{window:?}");
  }
}

#[test]
fn reports_keep_their_cell() {
  let report = Size { rows: 24, columns: 80, height: 480, width: 720 };
  note(report);
  assert_eq!(told(), Some(Cell { width: 9, height: 20 }));
}

#[test]
fn clean_fits_string_width() {
  assert_eq!(clean("a\tb\x1b", 5), "a b");
  // Each heart with U+FE0F is two cells, though its chars sum to one.
  let hearts = "❤\u{FE0F}".repeat(4);
  for columns in 0..8 {
    let clean = clean(&hearts, columns);
    assert!(hearts.starts_with(&clean));
    assert!(clean.width() <= columns, "{columns}");
  }
  let hostile = hearts.repeat(25) + &"\u{301}".repeat(50_000);
  let clean = clean(&hostile, 160);
  assert!(clean.width() <= 160 && clean.chars().count() <= 8 * 160);
}

#[test]
fn cursor_offsets_encode_ansi() {
  use vpdf_model::geometry::At;
  for (at, row, column, expected) in [
    (At::default(), 0u16, 0u16, "\x1b[1;1H"),
    (At { column: 7, row: 3 }, 4, 5, "\x1b[8;13H"),
    (
      At { column: u16::MAX, row: u16::MAX },
      u16::MAX,
      u16::MAX,
      "\x1b[131071;131071H",
    ),
  ] {
    assert_eq!(goto(at, row, column), expected);
  }
}
