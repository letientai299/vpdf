use super::run::*;
use super::windows::*;
use super::*;
#[cfg(test)]
use crate::SeedReader;

use crossterm::event::{KeyCode, KeyModifiers};
use vpdf_model::text::Bounds;
use vpdf_terminal::decode::{Button, Mouse, Spot};

#[test]
#[ignore = "requires verified public PDF corpus"]
fn perf_is_global_and_persistent() -> Result<()> {
  let root = tempfile::tempdir()?;
  let store = root.path().join("state.json");
  let mut session = paper()?;
  session.store = Some(store.clone());
  let screen = session.screen.expect("screen");
  session.app().command("perf on")?;
  session.settle()?;
  assert!(session.state.perf);
  let content = screen.rows - vpdf_ui::perf::rows(screen);
  assert_eq!(session.app().screen.rows, content);
  assert_eq!(session.app().at.row, vpdf_ui::perf::rows(screen));
  session.app().command("vsplit")?;
  session.settle()?;
  assert!(session.tab().panes.iter().all(|a| a.screen.rows == content));
  let mut out = Vec::new();
  session.draw_frame(&mut out)?;
  let text = String::from_utf8(out)?;
  assert_eq!(text.matches("FPS").count(), 1);
  session.save()?;
  assert!(Session::new(Some(store)).state.perf);
  session.app().command("perf")?;
  session.settle()?;
  assert!(!session.state.perf);
  assert!(session.tab().panes.iter().all(|a| a.screen.rows == screen.rows));
  assert!(session.app().command("perf maybe").is_err());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn perf_split_respects_focused_pane_size() -> Result<()> {
  let mut session = paper()?;
  session.toggle_perf(Some(true))?;
  for _ in 0..16 {
    let fits = Node::fits(Axis::Columns, full(session.app().screen));
    let count = session.tab().panes.len();
    let result = session.split(Axis::Columns, None, Place::After);
    if !fits {
      assert!(result.is_err());
      assert_eq!(session.tab().panes.len(), count);
      return Ok(());
    }
    result?;
  }
  panic!("splits never reached the minimum pane width");
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn perf_start_screen_reserves_once() -> Result<()> {
  let mut session = paper()?;
  let screen = session.screen.expect("screen");
  session.tabs.clear();
  session.home = Some(Home::new(&[], screen));
  session.toggle_perf(Some(true))?;
  let content = screen.rows - vpdf_ui::perf::rows(screen);
  assert_eq!(session.home.as_ref().expect("home").view.screen().rows, content);
  session.toggle_perf(Some(true))?;
  assert_eq!(session.home.as_ref().expect("home").view.screen().rows, content);
  session.toggle_perf(Some(false))?;
  assert_eq!(
    session.home.as_ref().expect("home").view.screen().rows,
    screen.rows
  );
  Ok(())
}

#[test]
fn image_timeout_retains_transfer() {
  let mut session = Session::new(None);
  let capture = crate::region::next_capture();
  let now = Instant::now();
  session.image_write = Some(ImageWrite {
    pane: 999,
    capture,
    state: ImageState::Waiting(now + Duration::from_secs(10)),
  });
  assert!(!session.image_timeout(now));
  assert!(session.image_timeout(now + Duration::from_secs(10)));
  assert!(!session.image_timeout(now + Duration::from_secs(20)));
  let original = session.image_write.as_ref().unwrap();
  assert_eq!(original.capture, capture);
  assert!(matches!(original.state, ImageState::TimedOut));
  assert!(!session.image_status("type=read:status=DONE"));
  assert!(session.image_write.is_some());
  assert!(session.image_status("type=write:status=DONE"));
  assert!(session.image_write.is_none());
  assert!(!session.image_status("type=write:status=DONE"));
}

/// Shape tests share the global shape state.
static SHAPES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The paper at page 2, 120x40.
fn paper() -> Result<Session> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen)?;
  session.app().command("2")?;
  Ok(session)
}

/// The first page cell `wanted` accepts.
pub(super) fn find(
  session: &mut Session,
  wanted: fn(Hit) -> bool,
) -> Result<(u16, u16)> {
  let page = Region::Page;
  for (row, column) in (0..39).flat_map(|row| (0..120).map(move |c| (row, c))) {
    if wanted(session.app().hovered(page, column, row, None)?) {
      return Ok((column, row));
    }
  }
  anyhow::bail!("no such cell")
}

/// Shapes a frame writes after `kind`.
fn shape_after(
  session: &mut Session,
  kind: MouseEventKind,
  (column, row): (u16, u16),
) -> Result<Vec<String>> {
  let modifiers = KeyModifiers::NONE;
  session.mouse(MouseEvent { kind, column, row, modifiers })?;
  session.settle()?;
  let mut out = Vec::new();
  session.draw_frame(&mut out)?;
  Ok(shapes(&out))
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn frames_write_changed_shapes() -> Result<()> {
  let _shapes = SHAPES.lock();
  let mut session = paper()?;
  let link = find(&mut session, |hit| matches!(hit, Hit::Link(..)))?;
  let blank = find(&mut session, |hit| hit == Hit::Nothing)?;
  terminal::claim_pointer(
    &mut Vec::new(),
    vpdf_terminal::caps::Support::Answered,
    false,
  )?;
  let frame =
    |session: &mut Session, cell: Option<(u16, u16)>| -> Result<Vec<String>> {
      match cell {
        Some(cell) => shape_after(session, MouseEventKind::Moved, cell),
        None => {
          let mut out = Vec::new();
          session.draw_frame(&mut out)?;
          Ok(shapes(&out))
        }
      }
    };
  // (move to, written)
  let steps = [
    (Some(link), vec!["pointer"]),
    (None, vec![]),
    (Some(blank), vec!["default"]),
    (Some(link), vec!["pointer"]),
  ];
  for (step, (cell, want)) in steps.into_iter().enumerate() {
    assert_eq!(frame(&mut session, cell)?, want, "step {step}");
  }
  // A focus-out, then hiding, hands over once.
  let mut out = Vec::new();
  input(&mut session, Act::Focus(false), &mut out)?;
  assert_eq!(shapes(&out), ["default"]);
  out.clear();
  session.hide(&mut out)?;
  assert_eq!(shapes(&out), Vec::<String>::new());
  session.sight = Sight::Focused;
  // (move to, written)
  let steps =
    [(None, vec![]), (Some(blank), vec!["default"]), (Some(blank), vec![])];
  for (step, (cell, want)) in steps.into_iter().enumerate() {
    assert_eq!(frame(&mut session, cell)?, want, "shown again, step {step}");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn scrollbars_wake_and_hide() -> Result<()> {
  let mut session = paper()?;
  session.draw_frame(&mut Vec::new())?;
  session.app().reader.seed_reader(|reader| reader.scroll += 64.0);
  session.draw_frame(&mut Vec::new())?;
  let now = Instant::now();
  let wait = idle(&mut session, now, now, false);
  assert!(wait <= vpdf_ui::scrollbar::Bar::SHOWN, "{wait:?}");
  assert!(!session.notify(), "shown until the deadline");
  let past = now.checked_sub(vpdf_ui::scrollbar::Bar::SHOWN * 2).unwrap();
  let app = session.app();
  app.bar.observe(app.reader.scroll + 1.0, past);
  assert!(session.notify(), "the deadline draws a frame");
  assert_eq!(session.app().bar_until(), None);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn held_gestures_write_shapes() -> Result<()> {
  use MouseEventKind::{Down, Drag, Moved, Up};
  use crossterm::event::MouseButton::{Left, Middle};
  let _shapes = SHAPES.lock();
  let mut session = paper()?;
  let blank = find(&mut session, |hit| hit == Hit::Nothing)?;
  let near = (blank.0 + 3, blank.1);
  terminal::claim_pointer(
    &mut Vec::new(),
    vpdf_terminal::caps::Support::Answered,
    false,
  )?;
  // (command first, event, cell) -> written
  let steps = [
    ("", Moved, blank, vec![]),
    ("", Down(Middle), blank, vec![]),
    ("", Drag(Middle), near, vec!["grabbing"]),
    ("", Up(Middle), near, vec!["default"]),
    ("vsplit", Moved, (60, 5), vec!["col-resize"]),
    ("", Down(Left), (60, 5), vec![]),
    ("", Drag(Left), (40, 5), vec![]),
    ("", Up(Left), (40, 5), vec![]),
    ("", Moved, (10, 39), vec!["default"]),
    ("split", Moved, (5, 19), vec!["row-resize"]),
  ];
  for (command, kind, cell, want) in steps {
    if !command.is_empty() {
      session.app().command(command)?;
      session.settle()?;
    }
    let got = shape_after(&mut session, kind, cell)?;
    assert_eq!(got, want, "{kind:?} at {cell:?}");
  }
  session.app().open_panel(PanelKind::Outline)?;
  let edge = session.app().left - 1;
  // (cell) -> written
  let steps = [
    ((edge - 1, 3), vec!["col-resize"]),
    ((edge, 3), vec![]),
    ((edge + 2, 3), vec!["default"]),
  ];
  for (cell, want) in steps {
    assert_eq!(shape_after(&mut session, Moved, cell)?, want, "{cell:?}");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn image_status_preserves_bar() -> Result<()> {
  use crate::region::Transfer;
  use std::collections::VecDeque;
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen)?;
  session.app().move_figure(true, 1)?;
  session.app().open_region()?;
  let old = session.app().region.as_ref().unwrap().snapshot().clone();
  let pane = session.app().id;
  session.image_write = Some(ImageWrite {
    pane,
    capture: old.id,
    state: ImageState::Waiting(Instant::now() + Duration::from_millis(500)),
  });
  let now = Instant::now();
  assert!(idle(&mut session, now, now, false) <= Duration::from_millis(500));
  session.app().select_region(old.page, old.bounds)?;
  let new = session.app().region.as_ref().unwrap().snapshot().clone();
  assert_ne!(old.id, new.id);
  session.app().region_status(new.id, "New operation".into());
  let mut events = VecDeque::new();
  vpdf_terminal::decode::Parser::default()
    .feed(b"\x1b]5522;type=write:status=DONE\x1b\\", &mut events);
  let event = act(events.pop_front().unwrap()).unwrap();
  assert!(!event.typed());
  input(&mut session, event, &mut Vec::new())?;
  assert!(session.image_write.is_none());
  assert_eq!(
    session.app().region.as_ref().unwrap().status(),
    Some("New operation")
  );
  assert_eq!(session.app().reader.message, "New operation");
  session.image_write =
    Some(ImageWrite { pane, capture: old.id, state: ImageState::TimedOut });
  session.app().region_send = Some(Transfer { id: new.id, png: vec![1] });
  let mut output = Vec::new();
  assert!(session.image_updates(&mut output));
  assert_eq!(output, [] as [u8; 0]);
  assert_eq!(session.image_write.as_ref().unwrap().capture, old.id);
  assert_eq!(
    session.app().reader.message,
    "Image clipboard awaiting previous status"
  );
  assert!(session.image_status("type=write:status=EPERM"));
  assert!(session.image_write.is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_deadlines_expire_across_panes() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen)?;
  session.app().request = Some(Request::Window {
    command: Window::Split(Axis::Columns),
    count: 1,
    explicit: false,
    file: None,
  });
  session.settle()?;
  let now = Instant::now();
  for app in &mut session.tab().panes {
    app.reader.flash(
      app.reader.cursor,
      vpdf_ui::page::landing::Span::Line,
      Instant::now(),
    );
    app.reader.notice("Occupied");
    let remaining =
      app.reader.landing().deadline().unwrap().duration_since(now);
    assert!(remaining >= vpdf_ui::page::landing::Landing::DURATION);
    assert!(remaining < Duration::from_millis(650));
  }
  let wait = idle(&mut session, now, now, false);
  assert!(wait <= vpdf_ui::page::landing::Landing::DURATION);
  assert!(!session.notify());
  for app in &mut session.tab().panes {
    app.reader.flash(
      app.reader.cursor,
      vpdf_ui::page::landing::Span::Line,
      now.checked_sub(vpdf_ui::page::landing::Landing::DURATION).unwrap(),
    );
  }
  assert_eq!(idle(&mut session, now, now, false), Duration::ZERO);
  assert!(session.notify());
  assert!(
    session.tab().panes.iter().all(|app| app
      .reader
      .landing()
      .position()
      .is_none())
  );
  assert!(!session.notify());
  Ok(())
}

#[test]
fn refit_keeps_the_layout_without_metrics() {
  let screen = Screen {
    columns: 80,
    rows: 24,
    origin: (0, 0),
    cell_width: 8,
    cell_height: 16,
  };
  let lost = Notice::new(20, 70, terminal::NO_CELL.into());
  let mut session = Session::new(None);
  session.home = Some(Home::new(&[], screen));
  let failed = || Err(anyhow::anyhow!(terminal::NO_CELL));
  let grid = || Ok((70, 20));
  // (measured, want unmeasured, want start screen rows)
  let cases = [
    (failed(), Some(lost.clone()), 24),
    (Ok(None), Some(lost), 24),
    (Ok(Some(Screen { rows: 30, ..screen })), None, 30),
  ];
  for (index, (measured, unmeasured, rows)) in cases.into_iter().enumerate() {
    assert!(session.refit(measured, grid).is_ok(), "case {index}");
    assert_eq!(session.unmeasured, unmeasured, "case {index}");
    let home = session.home.as_ref().map(|home| home.view.screen().rows);
    assert_eq!(home, Some(rows), "case {index}");
  }
}

/// tmux tells only its active pane about focus; the rest is asked.
#[test]
fn sight_follows_tmux() {
  use Cause::{FocusOut, Start, Tick};
  use Sight::{Focused, Hidden, Unfocused};
  let active = View::Active { focused: true };
  let away = View::Active { focused: false };
  // (sight, cause, view, want)
  let cases = [
    (Focused, Start, active, Focused),
    (Focused, Start, View::Inactive, Unfocused),
    (Focused, Start, View::Hidden, Hidden),
    (Focused, FocusOut, View::Inactive, Unfocused),
    (Focused, FocusOut, View::Hidden, Hidden),
    // A popup or menu over the active pane.
    (Focused, FocusOut, active, Hidden),
    // The client lost the OS focus.
    (Focused, FocusOut, away, Unfocused),
    (Focused, Start, away, Unfocused),
    (Unfocused, Tick, View::Hidden, Hidden),
    (Unfocused, Tick, View::Inactive, Unfocused),
    (Hidden, Tick, View::Inactive, Unfocused),
    (Hidden, Tick, View::Hidden, Hidden),
    // The popup is still open; focus-in ends it.
    (Hidden, Tick, active, Hidden),
    (Unfocused, Tick, away, Unfocused),
  ];
  for (now, cause, view, want) in cases {
    assert_eq!(sight(now, cause, view), want, "{now:?} {cause:?} {view:?}");
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn client_changes_preserve_positions() -> Result<()> {
  let mut session = paper()?;
  session.app().command("vsplit")?;
  session.settle()?;
  session.app().command("3")?;
  let before: Vec<_> = session
    .tabs
    .iter()
    .flat_map(|tab| &tab.panes)
    .map(App::navigation_position)
    .collect();
  let snapshot = |pid| {
    vpdf_terminal::tmux_observer::parse(&format!(
      "$0\t@0\tfocused\n{pid}:100:$0:@0:9:26:0;\n"
    ))
    .unwrap()
  };
  session.observation(snapshot(10), &mut Vec::new())?;
  let pane = session.app().id;
  session.resizing = Some(Edge::Panel(pane));
  assert!(session.observation(snapshot(11), &mut Vec::new())?);
  let after: Vec<_> = session
    .tabs
    .iter()
    .flat_map(|tab| &tab.panes)
    .map(App::navigation_position)
    .collect();
  assert_eq!(after, before);
  assert_eq!(session.resizing, None);
  assert!(!session.observation(snapshot(11), &mut Vec::new())?);
  Ok(())
}

#[test]
fn observations_recover_without_focus() -> Result<()> {
  let mut session = Session::new(None);
  let snapshot = |view, clients: &str| {
    vpdf_terminal::tmux_observer::parse(&format!("$0\t@0\t{view}\n{clients}\n"))
      .unwrap()
  };
  let client = "10:100:$0:@0:9:26:0;";
  let mut output = Vec::new();
  session.observation(snapshot("focused", client), &mut output)?;
  assert_eq!(session.sight, Sight::Focused);
  session.observation(snapshot("hidden", client), &mut output)?;
  assert_eq!(session.sight, Sight::Hidden);
  assert!(output.starts_with(terminal::frame_end().as_bytes()));
  assert!(session.observation(snapshot("focused", client), &mut output)?);
  assert_eq!(session.sight, Sight::Focused);
  session.observation(snapshot("inactive", client), &mut output)?;
  assert_eq!(session.sight, Sight::Unfocused);
  assert!(!session.observation(snapshot("inactive", client), &mut output)?);
  session.observation(snapshot("hidden", ""), &mut output)?;
  assert_eq!(session.sight, Sight::Hidden);
  let replacement = "11:100:$0:@0:9:26:0;";
  assert!(session.observation(snapshot("focused", replacement), &mut output)?);
  assert_eq!(session.sight, Sight::Focused);
  assert!(!session.observation(snapshot("focused", replacement), &mut output)?);
  Ok(())
}

/// Entering `Hidden` hides; leaving it asks for a frame.
#[test]
fn turns_report_reveals() -> Result<()> {
  use Sight::{Focused, Hidden, Unfocused};
  let mut session = Session::new(None);
  // (to, frame due, sight after)
  let steps = [
    (Unfocused, false, Unfocused),
    (Hidden, false, Hidden),
    (Hidden, false, Hidden),
    (Unfocused, true, Unfocused),
    (Hidden, false, Hidden),
    (Focused, true, Focused),
    (Focused, false, Focused),
  ];
  for (to, due, after) in steps {
    let mut out = Vec::new();
    assert_eq!(session.turn(to, &mut out)?, due, "{to:?}");
    assert_eq!(session.sight, after, "{to:?}");
  }
  Ok(())
}

/// Keys and the pointer reveal a pane whose focus-in got lost.
#[test]
fn input_proves_the_pane_shown() -> Result<()> {
  use Sight::{Focused, Hidden, Unfocused};
  let key = Act::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
  let mouse = Act::Mouse(Mouse {
    kind: MouseKind::Press(Button::LEFT),
    at: Spot::Cell { column: 1, row: 1 },
    modifiers: KeyModifiers::NONE,
  });
  // (sight, act, sight after)
  let cases = [
    (Hidden, &key, Focused),
    (Unfocused, &key, Focused),
    (Hidden, &mouse, Unfocused),
    (Unfocused, &mouse, Unfocused),
    (Hidden, &Act::Missing(1), Hidden),
    (Hidden, &Act::Focus(false), Hidden),
  ];
  for (now, act, want) in cases {
    let mut session = Session::new(None);
    session.sight = now;
    session.shown_by(act, &mut Vec::new())?;
    assert_eq!(session.sight, want, "{now:?} {act:?}");
  }
  Ok(())
}

/// Pointer input draws only when it reveals or changes something.
#[test]
fn pointer_input_reports_frames() -> Result<()> {
  let at = Spot::Cell { column: 1, row: 1 };
  let motion = || {
    Act::Mouse(Mouse {
      kind: MouseKind::Moved,
      at,
      modifiers: KeyModifiers::NONE,
    })
  };
  // (sight, act) -> frame
  let cases = [
    (Sight::Hidden, motion(), true),
    (Sight::Unfocused, motion(), false),
    (Sight::Focused, Act::Leave, false),
  ];
  for (sight, act, want) in cases {
    let mut session = Session::new(None);
    session.sight = sight;
    let at = format!("{sight:?} {act:?}");
    assert_eq!(input(&mut session, act, &mut Vec::new())?, want, "{at}");
  }
  Ok(())
}

/// Pointer shapes a frame writes.
fn shapes(out: &[u8]) -> Vec<String> {
  let out = String::from_utf8_lossy(out);
  out
    .split("\x1b]22;")
    .skip(1)
    .map(|rest| rest.split('\x1b').next().unwrap_or("").to_owned())
    .collect()
}

/// One text line; `annots` join the page.
fn line_pdf(directory: &Path, annots: &str) -> Result<PathBuf> {
  let pdf = directory.join("a.pdf");
  crate::fixtures::pdf(&pdf, &["alpha beta gamma"], annots)?;
  Ok(pdf)
}

#[test]
fn sweeps_show_a_crosshair() -> Result<()> {
  use crossterm::event::MouseButton::Left;
  use crossterm::event::MouseEventKind::{Down, Drag, Moved, Up};
  use vpdf_ui::pointer::{Hit, Region};
  let directory = tempfile::tempdir()?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  let app = session.app();
  let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
  let text = cells.find(|&(column, row)| {
    app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
  });
  let (column, row) = text.expect("a text cell");
  terminal::claim_pointer(
    &mut Vec::new(),
    vpdf_terminal::caps::Support::Answered,
    false,
  )?;
  let (alt, plain) = (KeyModifiers::ALT, KeyModifiers::NONE);
  // (event, modifiers, column) -> shapes written
  let steps = [
    (Moved, plain, column, vec!["text"]),
    (Down(Left), alt, column, vec!["crosshair"]),
    (Drag(Left), alt, column + 6, vec![]),
    // Lost release: motion drops the sweep.
    (Moved, plain, column, vec!["text"]),
    (Down(Left), alt, column, vec!["crosshair"]),
    (Drag(Left), alt, column + 6, vec![]),
    (Up(Left), alt, column + 6, vec!["text"]),
  ];
  for (step, (kind, modifiers, column, want)) in steps.into_iter().enumerate() {
    session.mouse(MouseEvent { kind, column, row, modifiers })?;
    let mut out = Vec::new();
    session.draw_frame(&mut out)?;
    assert_eq!(shapes(&out), want, "step {step}: {kind:?}");
  }
  Ok(())
}

/// `:mouse off` drops hover and later reports; `:mouse` toggles.
#[test]
fn mouse_command_toggles_capture() -> Result<()> {
  use vpdf_terminal::decode::{Mouse, MouseKind, Spot};
  use vpdf_ui::pointer::{Hit, Region};
  let directory = tempfile::tempdir()?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  session.screen = Some(screen);
  let app = session.app();
  let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
  let text = cells.find(|&(column, row)| {
    app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
  });
  let (column, row) = text.expect("a text cell");
  let at = Spot::Cell { column, row };
  let moved =
    Mouse { kind: MouseKind::Moved, at, modifiers: KeyModifiers::NONE };
  let hovered = |session: &Session| session.pointer.target().map(|h| h.hit);
  assert!(session.mouse_report(moved)?);
  assert_eq!(hovered(&session), Some(Hit::Text));
  // (command, capture after, hover after a report)
  let steps = [
    ("mouse off", false, None),
    ("mouse", true, Some(Hit::Text)),
    ("mouse", false, None),
    ("mouse on", true, Some(Hit::Text)),
  ];
  for (command, on, hover) in steps {
    // A held pan gets no release once capture ends.
    session.app().grab.begin((0.0, 0.0));
    session.app().grab.pixels((0.0, 0.0));
    session.app().command(command)?;
    session.settle()?;
    assert_eq!(terminal::mouse(), on, "{command}");
    assert_eq!(session.app().grabbing(), on, "{command}: pan");
    assert_eq!(hovered(&session), None, "{command}: dropped");
    let info = session.app().info();
    assert_eq!(info.contains("Mouse: on"), on, "{command}: info");
    session.mouse_report(moved)?;
    assert_eq!(hovered(&session), hover, "{command}: report");
    session.pointer.leave();
  }
  assert!(session.app().command("mouse maybe").is_err());
  Ok(())
}

/// Menu backdrops cover only page cells.
#[test]
fn menu_backdrops_skip_the_panel() -> Result<()> {
  use crossterm::event::MouseButton::Right;
  use crossterm::event::MouseEventKind::{Down, Up};
  use vpdf_ui::keys::PanelKind;
  let directory = tempfile::tempdir()?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  session.app().open_panel(PanelKind::Help)?;
  let left = session.app().left;
  // (pressed column) -> backdrop's page-area x range
  let cases = [(1, None), (left - 3, Some(0.0))];
  for (column, want) in cases {
    for kind in [Down(Right), Up(Right)] {
      let modifiers = KeyModifiers::NONE;
      session.mouse(MouseEvent { kind, column, row: 3, modifiers })?;
    }
    let app = session.app();
    let area = app.context_menu.as_ref().expect("a menu").area();
    let rect = app.context_rect();
    assert_eq!(rect.map(|rect| rect.x0), want, "{column}");
    if let Some(rect) = rect {
      let end = f32::from(area.column + area.columns - left) * 8.0;
      assert_eq!(rect.x1, end, "{column}");
    } else {
      assert!(area.column + area.columns <= left, "{column}");
    }
    app.context_menu = None;
  }
  Ok(())
}

/// Page-area pixels each `z=1` placement in `out` covers.
fn highlights(out: &[u8], left: u16) -> Vec<Bounds> {
  let text = String::from_utf8_lossy(out);
  let parts = text.split("\x1b7\x1b[").skip(1);
  let placed = parts.filter_map(|part| {
    let (at, rest) = part.split_once('H')?;
    let (row, column) = at.split_once(';')?;
    let control = rest.strip_prefix("\x1b_G")?.split(';').next()?;
    let value = |key: &str| {
      let mut pairs = control.split(',').filter_map(|p| p.split_once('='));
      pairs.find(|(name, _)| *name == key).map(|(_, value)| value)
    };
    let number = |key| value(key).and_then(|v| v.parse::<f32>().ok());
    if value("z") != Some("1") {
      return None;
    }
    let column = column.parse::<f32>().ok()? - 1.0 - f32::from(left);
    let row = row.parse::<f32>().ok()? - 1.0;
    let x0 = column * 8.0 + number("X")?;
    let y0 = row * 16.0 + number("Y")?;
    let (x1, y1) = (x0 + number("w")?, y0 + number("h")?);
    Some(Bounds { x0, y0, x1, y1 })
  });
  placed.collect()
}

/// The focused panel's page veil leaves menus over the page clear.
#[test]
fn veils_skip_menus() -> Result<()> {
  use crossterm::event::MouseButton::Right;
  use crossterm::event::MouseEventKind::{Down, Up};
  use vpdf_ui::keys::PanelKind;
  let directory = tempfile::tempdir()?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  session.app().open_panel(PanelKind::Help)?;
  let left = session.app().left;
  // A stretched pixel: `c=` columns wide.
  let veiled = |out: &[u8]| {
    let text = String::from_utf8_lossy(out);
    text.split("\x1b_G").any(|g| g.contains("z=1,c="))
  };
  // (pressed column) -> veiled
  let cases = [(None, true), (Some(1), true), (Some(left - 3), false)];
  for (column, want) in cases {
    session.app().context_menu = None;
    for kind in column.map_or_default(|_| vec![Down(Right), Up(Right)]) {
      let column = column.unwrap_or_default();
      let modifiers = KeyModifiers::NONE;
      session.mouse(MouseEvent { kind, column, row: 3, modifiers })?;
    }
    assert!(session.app().panel_focused(), "{column:?}");
    let mut out = Vec::new();
    session.draw_focused(&mut out)?;
    assert_eq!(veiled(&out), want, "{column:?}");
  }
  Ok(())
}

/// Thumbnail rings leave the menu over them clear.
#[test]
fn overview_rings_skip_the_menu() -> Result<()> {
  use crossterm::event::MouseButton::Right;
  use crossterm::event::MouseEventKind::{Down, Up};
  let directory = tempfile::tempdir()?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  session.app().open_overview()?;
  session.draw_focused(&mut Vec::new())?;
  let app = session.app();
  let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
  let thumbnail =
    cells.find(|&(column, row)| app.thumbnail_at(column, row).is_some());
  let (column, row) = thumbnail.expect("a thumbnail");
  for kind in [Down(Right), Up(Right)] {
    let modifiers = KeyModifiers::NONE;
    session.mouse(MouseEvent { kind, column, row, modifiers })?;
  }
  let menu = session.app().context_rect().expect("a menu over the page");
  let mut out = Vec::new();
  session.draw_focused(&mut out)?;
  let rings = highlights(&out, session.app().left);
  assert!(!rings.is_empty(), "the selected ring shows");
  let over = |b: &Bounds| {
    b.x0 < menu.x1 && menu.x0 < b.x1 && b.y0 < menu.y1 && menu.y0 < b.y1
  };
  assert!(!rings.iter().any(over), "{rings:?} over {menu:?}");
  Ok(())
}

/// Focus loss ends held drags.
#[test]
fn edge_drags_scroll_until_focus_loss() -> Result<()> {
  use crossterm::event::MouseButton::Left;
  use crossterm::event::MouseEventKind::{Down, Drag};
  use vpdf_ui::input::edge::Edge;
  use vpdf_ui::pointer::{Hit, Region};
  let directory = tempfile::tempdir()?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  let app = session.app();
  let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
  let text = cells.find(|&(column, row)| {
    app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
  });
  let (column, row) = text.expect("a text cell");
  let now = Instant::now();
  assert!(idle(&mut session, now, now, false) > Edge::INTERVAL);
  let (alt, plain) = (KeyModifiers::ALT, KeyModifiers::NONE);
  let corner = |app: &App| {
    app.gesture.sweep().and_then(vpdf_ui::input::gesture::Sweep::corner)
  };
  for modifiers in [plain, alt] {
    let event = |kind, row| MouseEvent { kind, column, row, modifiers };
    session.app().reader.seed_reader(|reader| reader.scroll = 0.0);
    session.mouse(event(Down(Left), row))?;
    // The status row is past it.
    session.mouse(event(Drag(Left), 23))?;
    let due = session.autoscroll_due().expect("a held edge drag");
    assert!(idle(&mut session, now, now, false) <= Edge::INTERVAL);
    let before = corner(session.app());
    assert!(session.autoscroll(due), "{modifiers:?}");
    // Sweeps follow the scroll too.
    let moved = corner(session.app()) != before;
    assert_eq!(moved, modifiers == alt, "{modifiers:?}");
    input(&mut session, Act::Focus(false), &mut Vec::new())?;
    let later = Instant::now() + Edge::INTERVAL;
    assert!(!session.autoscroll(later), "{modifiers:?}");
    assert_eq!(session.autoscroll_due(), None);
    // Drags after it hold nothing.
    session.mouse(event(Drag(Left), 23))?;
    assert_eq!(session.autoscroll_due(), None, "{modifiers:?}");
    assert!(!session.app().sweeping(), "{modifiers:?}");
    assert_eq!(session.app().sweep_hint(), None);
  }
  Ok(())
}

#[test]
fn start_screen_drops_open_or_type() -> Result<()> {
  use base64::Engine;
  let directory = tempfile::tempdir()?;
  let pdf = line_pdf(directory.path(), "")?;
  let notes = directory.path().join("notes.txt");
  std::fs::write(&notes, "text")?;
  let screen = Screen {
    columns: 80,
    rows: 24,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let list = |path: &Path| {
    let uri = format!("file://{}\r\n", path.display());
    base64::engine::general_purpose::STANDARD.encode(uri)
  };
  let drop = |session: &mut Session, home: &mut Home, path: &Path| {
    let mut out = Vec::new();
    let mut landed = Landing::Wait;
    for body in [
      "t=M:x=1:y=1:o=1;text/uri-list".to_owned(),
      format!("t=r:x=1;{}", list(path)),
      "t=r:x=1".to_owned(),
    ] {
      landed = session.home_drop(home, &body, &mut out)?;
    }
    Ok::<_, anyhow::Error>((landed, String::from_utf8_lossy(&out).into()))
  };
  let ended =
    |out: &String, how: &str| out.ends_with(&format!("t=r:o={how}\x1b\\"));
  // A prompt takes the path as text.
  let mut session = Session::new(None);
  let mut home = Home::new(&[], screen);
  let colon = KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE);
  home.event(Event::Key(colon), &mut session.history)?;
  let (landed, out) = drop(&mut session, &mut home, &pdf)?;
  assert!(matches!(landed, Landing::Shown) && ended(&out, "1"));
  assert!(session.tabs.is_empty());
  // Without a prompt the PDF opens; a list with none is cancelled.
  let mut home = Home::new(&[], screen);
  let (landed, out) = drop(&mut session, &mut home, &notes)?;
  assert!(matches!(landed, Landing::Shown) && ended(&out, "0"));
  assert_eq!(home.view.error(), Some("No PDF in the drop"));
  let (landed, out) = drop(&mut session, &mut home, &pdf)?;
  assert!(matches!(landed, Landing::Opened) && ended(&out, "1"));
  assert_eq!(session.tabs.len(), 1);
  // The terminal's error shows and ends the drop.
  let mut home = Home::new(&[], screen);
  let mut out = Vec::new();
  session.home_drop(&mut home, "t=M:x=1:y=1:o=1;text/uri-list", &mut out)?;
  let landed = session.home_drop(&mut home, "t=R:x=1;EIO", &mut out)?;
  assert!(matches!(landed, Landing::Shown));
  assert!(ended(&String::from_utf8_lossy(&out).into(), "0"));
  assert_eq!(home.view.error(), Some("Drop failed: EIO"));
  Ok(())
}

#[test]
fn only_actionable_input_counts() {
  let none = KeyModifiers::NONE;
  let press = KeyEvent::new(KeyCode::Char('j'), none);
  let release =
    KeyEvent::new_with_kind(KeyCode::Char('j'), none, KeyEventKind::Release);
  let mouse = |kind, at| Event::Mouse(Mouse { kind, at, modifiers: none });
  let cell = Spot::Cell { column: 1, row: 2 };
  let size = Size { rows: 24, columns: 80, height: 384, width: 640 };
  let graphics = |id, message: &str| {
    Event::Reply(Reply::Graphics {
      id,
      placement: Some(1),
      message: message.into(),
    })
  };
  let click =
    |at| Mouse { kind: MouseKind::Press(Button::LEFT), at, modifiers: none };
  let pixel = Spot::Pixel { x: 1, y: 2 };
  let cases = [
    (Event::Key(press), Some(Act::Key(press))),
    (Event::Key(release), None),
    (Event::Paste("a".into()), Some(Act::Paste("a".into()))),
    (Event::Resize(None), Some(Act::Resize(None))),
    (Event::Resize(Some(size)), Some(Act::Resize(Some(size)))),
    (
      mouse(MouseKind::Press(Button::LEFT), cell),
      Some(Act::Mouse(click(cell))),
    ),
    (
      mouse(MouseKind::Press(Button::BACK), cell),
      Some(Act::Mouse(Mouse {
        kind: MouseKind::Press(Button::BACK),
        at: cell,
        modifiers: none,
      })),
    ),
    (mouse(MouseKind::Release(Button::BACK), cell), None),
    (
      mouse(MouseKind::Press(Button::LEFT), pixel),
      Some(Act::Mouse(click(pixel))),
    ),
    (Event::Leave, Some(Act::Leave)),
    (Event::Focus(true), Some(Act::Focus(true))),
    (Event::Reply(Reply::Keyboard(5)), None),
    (graphics(Some(9), "ENOENT:no such image"), Some(Act::Missing(9))),
    (graphics(Some(9), "OK"), None),
    (graphics(None, "ENOENT:no such image"), None),
    (
      Event::Reply(Reply::Osc { code: 72, body: "t=m".into() }),
      Some(Act::Drop("t=m".into())),
    ),
  ];
  for (event, want) in cases {
    let at = format!("{event:?}");
    assert_eq!(act(event), want, "{at}");
  }
  // None of these moves `touched`.
  let moved = act(mouse(MouseKind::Moved, cell)).expect("motion");
  assert!(!moved.typed() && !Act::Leave.typed());
  assert!(!Act::Drop("t=m".into()).typed());
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn local_keys_clear_capture() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 121,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&root.join("book.pdf"), true, screen)?;
  let mut events = std::collections::VecDeque::new();
  let mut parser = vpdf_terminal::decode::Parser::default();
  for bytes in [
    &b":30\r\x1b[<0;30;10M\x1b[<32;35;12M"[..],
    &b"\t"[..],
    &b"\x1b[<32;100;32M\x1b[<0;100;32m"[..],
  ] {
    if bytes == b"\t" {
      session.app().open_panel(PanelKind::Help)?;
    }
    let cursor = session.app().reader.cursor;
    parser.feed(bytes, &mut events);
    for event in events.drain(..) {
      let act = act(event).expect("actionable bytes");
      input(&mut session, act, &mut Vec::new())?;
      session.settle()?;
    }
    if bytes.starts_with(b"\x1b[<32;100") {
      assert_eq!(session.app().reader.cursor, cursor);
    }
  }
  assert!(session.app().reader.selection.is_some());
  assert!(!session.app().panel_focused());
  Ok(())
}

/// Keys close previews in every pane (U15).
#[test]
fn keys_close_other_previews() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let link = "/Annots[<</Type/Annot/Subtype/Link/Rect[15 90 200 125]\
                /Dest[3 0 R/XYZ 0 50 0]>>]";
  let screen = Screen {
    columns: 121,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new(None);
  session.open(&line_pdf(directory.path(), link)?, true, screen)?;
  session.app().command("vsplit")?;
  session.settle()?;
  let tab = session.tab();
  let index = 1 - tab.focus;
  let app = &mut tab.panes[index];
  let bounds = app.doc.links(PageId(0))?[0].first();
  let rect = app.layout.project(PageId(0), bounds, app.reader.scroll);
  let (x, y) = rect.centre();
  let column = app.at.column + app.left + (x / 8.0) as u16;
  let row = app.at.row + (y / 16.0) as u16;
  let (kind, modifiers) = (MouseEventKind::Moved, KeyModifiers::NONE);
  session.mouse(MouseEvent { kind, column, row, modifiers })?;
  assert!(session.hover_preview(Instant::now() + vpdf_ui::input::links::REST));
  assert!(session.tab().panes[index].peek.is_some());
  let mut events = std::collections::VecDeque::new();
  vpdf_terminal::decode::Parser::default().feed(b"j", &mut events);
  for event in events {
    input(&mut session, act(event).expect("a key"), &mut Vec::new())?;
  }
  assert!(session.tab().panes.iter().all(|app| app.peek.is_none()));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn deferred_panels_clear_capture() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 121,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  for command in [b":recent\r".as_slice(), b":marks\r".as_slice()] {
    let mut session = Session::new(None);
    session.open(&root.join("book.pdf"), true, screen)?;
    let mut events = std::collections::VecDeque::new();
    let mut parser = vpdf_terminal::decode::Parser::default();
    for bytes in [
      b":30\r\x1b[<0;30;10M\x1b[<32;35;12M".as_slice(),
      command,
      b"\x1b[<32;40;15M\x1b[<0;40;15m".as_slice(),
    ] {
      let cursor = session.app().reader.cursor;
      parser.feed(bytes, &mut events);
      for event in events.drain(..) {
        input(&mut session, act(event).expect("input bytes"), &mut Vec::new())?;
        session.settle()?;
      }
      if bytes.starts_with(b"\x1b[<32;40") {
        assert_eq!(session.app().reader.cursor, cursor);
      }
    }
    assert!(session.app().panel_focused());
    assert!(session.app().reader.selection.is_some());
  }
  Ok(())
}
