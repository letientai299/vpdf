use super::maintenance::idle;
use super::run::*;
use super::windows::*;
use super::*;
#[cfg(test)]
use crate::SeedReader;
use crate::screen::cells;

use vpdf_model::input::{Button, Mouse, Spot};
use vpdf_model::input::{KeyCode, KeyModifiers};
use vpdf_model::text::Bounds;
use vpdf_pdf::jobs::Wake;
use vpdf_terminal::decode::Reply;

#[test]
fn tab_completion_tracks_session() -> Result<()> {
  use vpdf_ui::prompt::{Prompt, PromptKind};
  let root = tempfile::tempdir()?;
  let first = root.path().join("first.pdf");
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&first, &["first"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  let mut session = Session::new(None);
  session.open(&first, true, cells(120, 40))?;
  session.open(&second, true, cells(120, 40))?;
  session.activate(0)?;
  let tab = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
  session.app().overlays.open_prompt(Prompt::new(PromptKind::Command));
  session.app().overlays.prompt_mut().unwrap().paste("tabnext ");
  session.type_key(tab)?;
  let prompt = session.app().overlays.prompt().unwrap();
  assert_eq!(
    prompt.menu().unwrap().labels(),
    ["1  first.pdf", "2  second.pdf"]
  );
  session.type_key(tab)?;
  session.type_key(tab)?;
  session.type_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
  session.settle()?;
  assert_eq!(session.active, 1);
  session.close_tab(0)?;
  session.app().overlays.open_prompt(Prompt::new(PromptKind::Command));
  session.app().overlays.prompt_mut().unwrap().paste("tabnext ");
  session.type_key(tab)?;
  assert_eq!(session.app().overlays.prompt().unwrap().text(), "tabnext 1");
  Ok(())
}

#[test]
fn cleanup_failure_survives_detached_home() -> Result<()> {
  let root = tempfile::tempdir()?;
  let store = root.path().join("state.json");
  let mut state = State::default();
  state.recent.push(crate::state::tests::entry(
    root.path().join("missing.pdf").to_str().unwrap(),
    0,
    1,
  ));
  state.save(&store, &[], &[])?;
  let mut session = Session::new(Some(store.clone()));
  std::fs::remove_file(&store)?;
  std::fs::create_dir(&store)?;
  let (wake, woken) = crate::fixtures::wake();
  session.frontend = crate::raster::Frontend::Native(wake);
  let changed = session.clean_state();
  woken.recv_timeout(Duration::from_secs(5))?;
  assert!(session.clean_state() || changed);
  assert!(session.unsaved);
  assert!(
    session
      .startup_warning
      .as_deref()
      .is_some_and(|warning| { warning.starts_with("State not saved:") })
  );
  session.home = Some(Home::new(&[], cells(100, 40), &Wake::none(), true));
  session.present_startup();
  assert!(session.home.as_ref().unwrap().view.error().is_some());
  Ok(())
}

#[test]
fn cleanup_persists_without_open_documents() -> Result<()> {
  let root = tempfile::tempdir()?;
  let store = root.path().join("state.json");
  let missing = root.path().join("missing.pdf");
  let mut state = State::default();
  state.recent.push(crate::state::tests::entry(
    missing.to_str().unwrap(),
    0,
    1,
  ));
  state.save(&store, &[], &[])?;
  let mut session = Session::new(Some(store.clone()));
  let (wake, woken) = crate::fixtures::wake();
  session.frontend = crate::raster::Frontend::Native(wake);
  session.clean_state();
  woken.recv_timeout(Duration::from_secs(5))?;
  session.clean_state();
  assert_eq!(session.state.recent, []);
  assert_eq!(State::load(&store).recent, []);
  assert!(!session.unsaved);
  let latest = session.latest.lock().unwrap();
  assert_eq!(latest.as_ref().unwrap().0.recent, []);
  Ok(())
}

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
  session.home = Some(Home::new(&[], screen, &Wake::none(), true));
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

/// Shape tests share the global shape state.
static SHAPES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The paper at page 2, 120x40.
fn paper() -> Result<Session> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = cells(120, 40);
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
  let screen = session.app().screen;
  for (row, column) in (0..screen.rows.saturating_sub(1))
    .flat_map(|row| (0..screen.columns).map(move |column| (row, column)))
  {
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
  let wait = idle(&mut session, now, now, false, Instant::now());
  assert!(wait <= vpdf_ui::scrollbar::Bar::SHOWN, "{wait:?}");
  assert!(!session.notify(), "shown until the deadline");
  let past = now.checked_sub(vpdf_ui::scrollbar::Bar::SHOWN * 2).unwrap();
  let app = session.app();
  app.bar.observe(app.reader.scroll + 1.0, past);
  assert!(session.notify(), "the deadline draws a frame");
  assert_eq!(session.app().bar.deadline(), None);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn held_gestures_write_shapes() -> Result<()> {
  use MouseEventKind::{Down, Drag, Moved, Up};
  use vpdf_model::input::MouseButton::{Left, Middle};
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
fn landing_deadlines_expire_across_panes() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = cells(120, 40);
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
  let wait = idle(&mut session, now, now, false, Instant::now());
  assert!(wait <= vpdf_ui::page::landing::Landing::DURATION);
  assert!(!session.notify());
  for app in &mut session.tab().panes {
    app.reader.flash(
      app.reader.cursor,
      vpdf_ui::page::landing::Span::Line,
      now.checked_sub(vpdf_ui::page::landing::Landing::DURATION).unwrap(),
    );
  }
  assert_eq!(
    idle(&mut session, now, now, false, Instant::now()),
    Duration::ZERO
  );
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
  let screen = cells(80, 24);
  let lost = Notice::new(20, 70, terminal::NO_CELL.into());
  let mut session = Session::new(None);
  session.home = Some(Home::new(&[], screen, &Wake::none(), true));
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

#[test]
#[ignore = "requires verified public PDF corpus"]
fn client_changes_preserve_positions() -> Result<()> {
  let mut session = paper()?;
  session.app().command("vsplit")?;
  session.settle()?;
  session.app().command("3")?;
  let before: Vec<_> = session.panes().map(App::navigation_position).collect();
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
  let after: Vec<_> = session.panes().map(App::navigation_position).collect();
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

fn line_session(
  directory: &Path,
  annots: &str,
  screen: Screen,
) -> Result<Session> {
  let mut session = Session::new(None);
  session.open(&line_pdf(directory, annots)?, true, screen)?;
  Ok(session)
}

#[test]
fn sweeps_show_a_crosshair() -> Result<()> {
  use vpdf_model::input::MouseButton::Left;
  use vpdf_model::input::MouseEventKind::{Down, Drag, Moved, Up};
  use vpdf_ui::pointer::Hit;
  let directory = tempfile::tempdir()?;
  let screen = cells(80, 24);
  let mut session = line_session(directory.path(), "", screen)?;
  let (column, row) = find(&mut session, |hit| hit == Hit::Text)?;
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
  use vpdf_model::input::{Mouse, MouseKind, Spot};
  use vpdf_ui::pointer::Hit;
  let directory = tempfile::tempdir()?;
  let screen = cells(80, 24);
  let mut session = line_session(directory.path(), "", screen)?;
  session.screen = Some(screen);
  let (column, row) = find(&mut session, |hit| hit == Hit::Text)?;
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

fn panel_session(directory: &Path) -> Result<Session> {
  let mut session = line_session(directory, "", cells(80, 24))?;
  session.app().open_panel(PanelKind::Version)?;
  Ok(session)
}

/// Menu backdrops cover only page cells.
#[test]
fn menu_backdrops_skip_the_panel() -> Result<()> {
  use vpdf_model::input::MouseButton::Right;
  use vpdf_model::input::MouseEventKind::{Down, Up};

  let directory = tempfile::tempdir()?;
  let mut session = panel_session(directory.path())?;
  let left = session.app().left;
  // (pressed column) -> backdrop's page-area x range
  let cases = [(1, None), (left - 3, Some(0.0))];
  for (column, want) in cases {
    for kind in [Down(Right), Up(Right)] {
      let modifiers = KeyModifiers::NONE;
      session.mouse(MouseEvent { kind, column, row: 3, modifiers })?;
    }
    let app = session.app();
    let area = app.overlays.menu().expect("a menu").area();
    let rect = app.context_rect();
    assert_eq!(rect.map(|rect| rect.x0), want, "{column}");
    if let Some(rect) = rect {
      let end = f32::from(area.column + area.columns - left) * 8.0;
      assert_eq!(rect.x1, end, "{column}");
    } else {
      assert!(area.column + area.columns <= left, "{column}");
    }
    app.overlays.close_menu();
  }
  Ok(())
}

/// Page-area pixels each `z=1` placement in `out` covers.
fn highlights(out: &[u8], left: u16) -> Vec<Bounds> {
  let text = String::from_utf8_lossy(out);
  let commands = crate::kitty::commands(&text);
  let placed = commands.iter().filter(|c| c.get("z") == Some("1"));
  placed.filter_map(|command| command.bounds((left, 0))).collect()
}

/// The focused panel's page veil leaves menus over the page clear.
#[test]
fn veils_skip_menus() -> Result<()> {
  use vpdf_model::input::MouseButton::Right;
  use vpdf_model::input::MouseEventKind::{Down, Up};

  let directory = tempfile::tempdir()?;
  let mut session = panel_session(directory.path())?;
  let left = session.app().left;
  // A stretched pixel: `c=` columns wide.
  let veiled = |out: &[u8]| {
    let text = String::from_utf8_lossy(out);
    text.split("\x1b_G").any(|g| g.contains("z=1,c="))
  };
  // (pressed column) -> veiled
  let cases = [(None, true), (Some(1), true), (Some(left - 3), false)];
  for (column, want) in cases {
    session.app().overlays.close_menu();
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
  use vpdf_model::input::MouseButton::Right;
  use vpdf_model::input::MouseEventKind::{Down, Up};
  let directory = tempfile::tempdir()?;
  let screen = cells(80, 24);
  let mut session = line_session(directory.path(), "", screen)?;
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
  use vpdf_model::input::MouseButton::Left;
  use vpdf_model::input::MouseEventKind::{Down, Drag};
  use vpdf_ui::input::edge::Edge;
  use vpdf_ui::pointer::Hit;
  let directory = tempfile::tempdir()?;
  let screen = cells(80, 24);
  let mut session = line_session(directory.path(), "", screen)?;
  let (column, row) = find(&mut session, |hit| hit == Hit::Text)?;
  let now = Instant::now();
  assert!(idle(&mut session, now, now, false, Instant::now()) > Edge::INTERVAL);
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
    assert!(
      idle(&mut session, now, now, false, Instant::now()) <= Edge::INTERVAL
    );
    let before = corner(session.app());
    assert!(session.maintain(due), "{modifiers:?}");
    // Sweeps follow the scroll too.
    let moved = corner(session.app()) != before;
    assert_eq!(moved, modifiers == alt, "{modifiers:?}");
    input(&mut session, Act::Focus(false), &mut Vec::new())?;
    let later = Instant::now() + Edge::INTERVAL;
    assert!(!session.maintain(later), "{modifiers:?}");
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
fn home_file_drops_open() -> Result<()> {
  use base64::Engine;
  let directory = tempfile::tempdir()?;
  let pdf = line_pdf(directory.path(), "")?;
  let notes = directory.path().join("notes.txt");
  std::fs::write(&notes, "text")?;
  let screen = cells(80, 24);
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
  // File drops open even with a prompt.
  let mut session = Session::new(None);
  let mut home = Home::new(&[], screen, &Wake::none(), true);
  let colon = KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE);
  home.event(Event::Key(colon), &mut session.history)?;
  let (landed, out) = drop(&mut session, &mut home, &pdf)?;
  assert!(matches!(landed, Landing::Opened) && ended(&out, "1"));
  assert_eq!(session.tabs.len(), 1);
  // Without a prompt the PDF opens; a list with none is cancelled.
  let mut home = Home::new(&[], screen, &Wake::none(), true);
  let (landed, out) = drop(&mut session, &mut home, &notes)?;
  assert!(matches!(landed, Landing::Shown) && ended(&out, "0"));
  assert_eq!(home.view.error(), Some("No PDF in the drop"));
  let (landed, out) = drop(&mut session, &mut home, &pdf)?;
  assert!(matches!(landed, Landing::Opened) && ended(&out, "1"));
  assert_eq!(session.tabs.len(), 1);
  let mut failed = Session::new(None);
  let mut home = Home::new(&[], screen, &Wake::none(), true);
  let invalid = PathBuf::from(format!("{}#page=99", pdf.display()));
  let (landed, out) = drop(&mut failed, &mut home, &invalid)?;
  assert!(matches!(landed, Landing::Opened) && ended(&out, "0"));
  assert!(
    failed
      .app()
      .error
      .as_deref()
      .is_some_and(|error| error.contains("Page outside document"))
  );
  // The terminal's error shows and ends the drop.
  let mut home = Home::new(&[], screen, &Wake::none(), true);
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
  use vpdf_model::input::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
  use vpdf_terminal::decode::{Button, Mouse, MouseKind, Spot};
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
    (
      Event::Key(press),
      Some(Act::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE))),
    ),
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
  let screen = cells(121, 40);
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
      session.app().open_panel(PanelKind::Version)?;
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
  let screen = cells(121, 40);
  let mut session = line_session(directory.path(), link, screen)?;
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
  assert!(session.tab().panes[index].overlays.preview().is_some());
  let mut events = std::collections::VecDeque::new();
  vpdf_terminal::decode::Parser::default().feed(b"j", &mut events);
  for event in events {
    input(&mut session, act(event).expect("a key"), &mut Vec::new())?;
  }
  assert!(
    session.tab().panes.iter().all(|app| app.overlays.preview().is_none())
  );
  Ok(())
}

#[test]
fn static_pane_keeps_hover() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let mut session = Session::new(None);
  let path = line_pdf(directory.path(), "")?;
  let screen = cells(121, 40);
  session.open(&path, true, screen)?;
  let modifiers = KeyModifiers::NONE;
  session.app().command("vsplit")?;
  session.settle()?;
  let pointed = session.tab().panes[0].id;
  session.tab().panes[1].reader.animate_to(20.0, Instant::now());
  let app = &session.tab().panes[0];
  let mouse = MouseEvent {
    kind: MouseEventKind::Moved,
    column: app.at.column + app.left + 1,
    row: app.at.row + 1,
    modifiers,
  };
  session.mouse(mouse)?;
  session.rehover();
  assert_eq!(session.pointer.target().map(|hover| hover.pane), Some(pointed));
  assert!(!session.skimmed);
  session.tab().panes[1].reader.cancel_scroll();
  Ok(())
}

/// Hover waits out a wheel gesture, then tests the page again.
#[test]
fn wheel_turns_defer_page_hover() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let link = "/Annots[<</Type/Annot/Subtype/Link/Rect[15 90 200 125]\
                /Dest[0 0 R/XYZ 0 50 0]>>]";
  let screen = cells(121, 40);
  let mut session = line_session(directory.path(), link, screen)?;
  let app = session.app();
  let bounds = app.doc.links(PageId(0))?[0].first();
  let rect = app.layout.project(PageId(0), bounds, app.reader.scroll);
  let (x, y) = rect.centre();
  let column = app.at.column + app.left + (x / 8.0) as u16;
  let row = app.at.row + (y / 16.0) as u16;
  let modifiers = KeyModifiers::NONE;
  let link_hit = |session: &Session| {
    matches!(
      session.pointer.target().map(|hover| hover.hit),
      Some(Hit::Link(..))
    )
  };
  session.mouse(MouseEvent {
    kind: MouseEventKind::Moved,
    column,
    row,
    modifiers,
  })?;
  assert!(link_hit(&session));
  session.app().reader.animate_to(20.0, Instant::now());
  session.rehover();
  assert!(!session.mouse(MouseEvent {
    kind: MouseEventKind::Moved,
    column,
    row,
    modifiers,
  })?);
  assert!(!link_hit(&session));
  assert_eq!(session.hover_settle_due(), None);
  session.app().reader.cancel_scroll();
  session.rehover();
  assert!(link_hit(&session));
  // At the top, the turn moves nothing yet starts a gesture.
  let kind = MouseEventKind::ScrollUp;
  session.mouse(MouseEvent { kind, column, row, modifiers })?;
  session.animate(Instant::now());
  session.rehover();
  assert!(!link_hit(&session));
  let due = session.hover_settle_due().expect("a deferred hover");
  assert!(!session.maintain(due.checked_sub(Duration::from_nanos(1)).unwrap()));
  assert!(session.maintain(due));
  assert_eq!(session.hover_settle_due(), None);
  // The gesture has ended by now in real time too.
  std::thread::sleep(due.saturating_duration_since(Instant::now()));
  session.rehover();
  assert!(link_hit(&session));
  session.mouse(MouseEvent { kind, column, row, modifiers })?;
  session.animate(Instant::now());
  session.rehover();
  assert!(!link_hit(&session));
  let due = session.hover_settle_due();
  let kind = MouseEventKind::Moved;
  assert!(!session.mouse(MouseEvent { kind, column, row, modifiers })?);
  assert!(!link_hit(&session));
  assert_eq!(session.hover_settle_due(), due);
  session.mouse(MouseEvent { kind, column: column + 1, row, modifiers })?;
  assert!(link_hit(&session));
  assert_eq!(session.hover_settle_due(), None);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn deferred_panels_clear_capture() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = cells(121, 40);
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

#[test]
fn decoded_commands_change_state() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let mut session = Session::new(None);
  let screen = cells(80, 24);
  session.open(&line_pdf(directory.path(), "")?, true, screen)?;
  let mut parser = vpdf_terminal::decode::Parser::default();
  let mut events = std::collections::VecDeque::new();
  parser.feed(b"\x1b[58;1:3u", &mut events);
  assert!(act(events.pop_front().unwrap()).is_none());
  assert!(session.app().overlays.prompt().is_none());
  parser.feed(b"\x1b[58;1:2u", &mut events);
  let action = act(events.pop_front().unwrap()).unwrap();
  let Act::Key(key) = &action else { panic!("key") };
  assert_eq!(key.code, KeyCode::Char(':'));
  assert_eq!(key.kind, vpdf_model::input::KeyEventKind::Repeat);
  input(&mut session, action, &mut Vec::new())?;
  assert!(session.app().overlays.prompt().is_some());
  parser.feed(b"\x1b[200~info\x1b[201~\r", &mut events);
  for event in events {
    input(&mut session, act(event).unwrap(), &mut Vec::new())?;
  }
  assert!(session.app().overlays.prompt().is_none());
  assert_eq!(
    session.app().panel.as_ref().unwrap().view.kind(),
    PanelKind::Info
  );
  Ok(())
}

#[test]
fn sidebar_keys_toggle_focus() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = line_pdf(directory.path(), "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  for (shortcut, kind, command) in [
    ("go", PanelKind::Outline, "toc"),
    ("gs", PanelKind::Results, ""),
    ("gr", PanelKind::Recent, "recent"),
    ("ga", PanelKind::Annotations, "annotations"),
  ] {
    for (keys, focused) in [
      (shortcut, Some(true)),
      ("Tab", Some(false)),
      (shortcut, Some(true)),
      (shortcut, None),
      (shortcut, Some(true)),
    ] {
      for key in vpdf_ui::keys::parse(keys).expect("valid sidebar keys") {
        session.type_key(key)?;
        session.settle()?;
      }
      assert_eq!(
        session.app().panel.is_some(),
        focused.is_some(),
        "{shortcut}"
      );
      if let Some(focused) = focused {
        let panel = &session.app().panel.as_ref().unwrap().view;
        assert_eq!(panel.kind(), kind, "{shortcut}");
        assert_eq!(panel.focused(), focused, "{shortcut}");
      }
    }
    assert!(session.app().panel_focused(), "{shortcut}");
    session.app().action(vpdf_ui::keys::Input::plain(
      vpdf_ui::keys::Action::Panel(kind),
    ))?;
    session.settle()?;
    assert_eq!(session.app().panel.as_ref().unwrap().view.kind(), kind);
    assert!(session.app().panel_focused());
    if !command.is_empty() {
      session.app().command(command)?;
      session.settle()?;
      assert_eq!(session.app().panel.as_ref().unwrap().view.kind(), kind);
      assert!(session.app().panel_focused());
    }
  }
  Ok(())
}

pub(super) fn settings_file(directory: &Path) -> crate::config::Loaded {
  let path = directory.join("config.toml");
  std::fs::write(
    &path,
    concat!(
      "[reading]\ncursorline = false\nzoom = 2.0\n",
      "[ocr]\nlanguage = 'eng+vie'\nunknown = true",
    ),
  )
  .unwrap();
  crate::config::Options { config: Some(path), ..Default::default() }.load()
}

#[test]
fn configured_views_and_reload() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("one.pdf");
  let second = directory.path().join("two.pdf");
  crate::fixtures::pdf(&path, &["one", "two"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  let mut session = Session::configured(None, settings_file(directory.path()));
  let screen = cells(120, 40);
  session.open(&path, true, screen)?;
  assert!(!session.app().cursorline);
  assert_eq!(session.app().layout.zoom(), 2.0);
  let width = session.app().layout.pages()[0].width;
  let default = App::with_screen(&path, "", false, session.app().screen)?;
  assert!((width - default.layout.pages()[0].width * 2.0).abs() <= 1.0);
  assert_eq!(session.app().settings.language(), "eng+vie");
  session.app().command("cursorline")?;
  session.app().layout.request_zoom(vpdf_ui::layout::Zoom::To(3.0));
  std::fs::write(directory.path().join("config.toml"), "[reading]\nzoom = 4")?;
  session.app().reload_with("")?;
  assert_eq!(session.app().settings.language(), "eng+vie");
  assert!(session.app().cursorline);
  assert_eq!(session.app().layout.zoom(), 3.0);
  session.split(Axis::Columns, None, Place::After)?;
  assert_eq!(session.app().layout.zoom(), 3.0);
  session.open(&second, true, screen)?;
  assert!(!session.app().cursorline);
  assert_eq!(session.app().layout.zoom(), 2.0);
  session.open(&path, false, screen)?;
  assert_eq!(session.app().layout.zoom(), 3.0);
  Ok(())
}

#[test]
fn saved_view_launch_precedence() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("source.pdf");
  let store = directory.path().join("state.json");
  crate::fixtures::pdf(&path, &["one", "two", "three"], "")?;
  let screen = cells(120, 40);
  let mut original = Session::new(Some(store.clone()));
  original.open(&path, true, screen)?;
  original.app().layout.request_zoom(vpdf_ui::layout::Zoom::To(3.0));
  original.start_at(3)?;
  original.record();
  original.save()?;
  let mut session =
    Session::configured(Some(store), settings_file(directory.path()));
  session.open(&path, true, screen)?;
  assert_eq!(session.app().layout.zoom(), 3.0);
  assert_eq!(session.app().reader.cursor.page, PageId(2));
  session.start_at(2)?;
  assert_eq!(session.app().reader.cursor.page, PageId(1));
  assert!(!session.app().cursorline);
  Ok(())
}

#[test]
fn terminal_startup_warning() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let screen = cells(120, 40);
  let config = settings_file(directory.path());
  let warning = config.warning.clone().unwrap();
  let mut session = Session::configured(None, config);
  session.screen = Some(screen);
  let mut home = Home::new(&[], screen, &Wake::none(), true);
  let mut out = Vec::new();
  session.draw_home(&mut home, &mut out)?;
  assert_eq!(home.view.error(), Some(warning.as_str()));
  assert!(String::from_utf8_lossy(&out).contains("Config"));
  let path = directory.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["one"], "")?;
  let mut session = Session::configured(None, settings_file(directory.path()));
  session.open(&path, true, screen)?;
  session.app().reader.notice("Normal startup");
  session.draw_frame(&mut Vec::new())?;
  assert_eq!(session.app().reader.message, warning);
  session.app().reader.cancel();
  session.draw_frame(&mut Vec::new())?;
  assert_eq!(session.app().reader.message, "");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus and Tesseract"]
fn real_ocr_and_cancel() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let config = directory.path().join("config.toml");
  std::fs::write(&config, "[ocr]\nlanguage = 'vpdf_missing_language'")?;
  let loaded =
    crate::config::Options { config: Some(config), ..Default::default() }
      .load();
  let source = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("scan.pdf");
  let mut session = Session::configured(None, loaded);
  session.open(&source, true, cells(120, 40))?;
  session.start_at(10)?;
  let app = session.app();
  app.command("ocr")?;
  let result = app
    .ocr
    .take()
    .unwrap()
    .work
    .result
    .recv_timeout(Duration::from_secs(30))?;
  assert!(result.unwrap_err().contains("vpdf_missing_language"));
  app.command("ocr eng")?;
  let result = app
    .ocr
    .take()
    .unwrap()
    .work
    .result
    .recv_timeout(Duration::from_secs(30))?;
  assert!(result.unwrap().unwrap().plain().contains("computing devices"));
  let bounds = app.doc.pages[9].bounds;
  app.select_region(PageId(9), bounds)?;
  app.region_operation(vpdf_ui::region::Operation::Ocr)?;
  let deadline = Instant::now() + Duration::from_secs(30);
  while app.region_job.is_some() {
    app.region_updates()?;
    anyhow::ensure!(Instant::now() < deadline, "Region OCR timeout");
    std::thread::sleep(Duration::from_millis(10));
  }
  assert!(app.reader.message.contains("vpdf_missing_language"));
  Ok(())
}

#[test]
fn config_reload_reaches_inactive_panes() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("one.pdf");
  let second = directory.path().join("two.pdf");
  crate::fixtures::pdf(&path, &["one", "two"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  let mut session = Session::configured(None, settings_file(directory.path()));
  let screen = cells(120, 40);
  session.open(&path, true, screen)?;
  session.split(Axis::Columns, None, Place::After)?;
  session.open(&second, true, screen)?;
  for tab in &mut session.tabs {
    for app in &mut tab.panes {
      app.layout.request_zoom(vpdf_ui::layout::Zoom::To(3.0));
      app.cursorline = true;
      app.keys.feed(
        KeyEvent::new(KeyCode::Char('3'), KeyModifiers::NONE),
        vpdf_ui::keys::Grammar::Normal,
      );
      app.overlays.open_prompt(vpdf_ui::prompt::Prompt::new(
        vpdf_ui::prompt::PromptKind::Command,
      ));
      app.overlays.prompt_mut().unwrap().replace("unsubmitted text");
    }
  }
  let positions: Vec<_> = session
    .tabs
    .iter()
    .flat_map(|tab| tab.panes.iter().map(|app| app.reader.cursor))
    .collect();
  std::fs::write(
    directory.path().join("config.toml"),
    concat!(
      "[appearance]\ntheme = 'dracula'\n",
      "[reading]\nzoom = 4\ncursorline = false\n",
      "inactive_brightness = 0.4\n[ocr]\nlanguage = 'vie'",
    ),
  )?;
  session.reload_config();
  let mut positions = positions.into_iter();
  for tab in &session.tabs {
    for app in &tab.panes {
      assert_eq!(app.settings.theme(), vpdf_model::theme::Theme::Dracula);
      assert_eq!(app.settings.zoom(), 4.0);
      assert_eq!(app.settings.inactive_brightness(), 0.4);
      assert_eq!(app.settings.language(), "vie");
      assert_eq!(app.layout.zoom(), 3.0);
      assert!(app.cursorline);
      assert_eq!(app.reader.cursor, positions.next().unwrap());
      assert_eq!(app.keys.pending(), "");
      assert_eq!(app.overlays.prompt().unwrap().text(), "unsubmitted text");
    }
  }
  let third = directory.path().join("three.pdf");
  crate::fixtures::pdf(&third, &["third"], "")?;
  session.open(&third, true, screen)?;
  assert_eq!(session.app().layout.zoom(), 4.0);
  assert!(!session.app().cursorline);
  for argument in
    ["", "help", "bindings", "bindings normal", "licenses", "reload extra"]
  {
    assert!(session.app().command(&format!("config {argument}")).is_err());
    assert_eq!(session.app().request, None);
  }
  session.app().command("config reload")?;
  assert_eq!(session.app().request, Some(Request::ConfigReload));
  session.settle()?;
  Ok(())
}

#[test]
fn reload_replaces_config_diagnostics() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("one.pdf");
  crate::fixtures::pdf(&path, &["one"], "")?;
  let mut session = Session::configured(None, settings_file(directory.path()));
  session.open(&path, true, cells(120, 40))?;
  session.present_startup();
  assert!(session.app().reader.message.contains("Config"));
  std::fs::write(directory.path().join("config.toml"), "")?;
  session.reload_config();
  assert_eq!(session.app().reader.message, "");
  session.app().reader.notice("Other message");
  session.reload_config();
  assert_eq!(session.app().reader.message, "Other message");
  Ok(())
}

#[test]
fn prompt_remaps_and_reload_preserve_editing() -> Result<()> {
  use super::interaction::Interaction;
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("one.pdf");
  let config = directory.path().join("config.toml");
  crate::fixtures::pdf(&path, &["one"], "")?;
  std::fs::write(
    &config,
    concat!(
      "[keybindings.command]\n",
      "'<C-a>' = { unbind = true }\n",
      "'<D-a>' = { action = 'prompt.start' }\n",
      "'<D-r>' = { command = 'config reload' }\n",
      "'<Esc>' = { unbind = true }\n",
      "'<D-q>' = { action = 'prompt.cancel' }\n",
    ),
  )?;
  let loaded = crate::config::Options {
    config: Some(config.clone()),
    ..Default::default()
  }
  .load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let mut session = Session::configured(None, loaded);
  session.open(&path, true, cells(120, 40))?;
  let key = |code, modifiers| Interaction::Key(KeyEvent::new(code, modifiers));
  session.interact(key(KeyCode::Char(':'), KeyModifiers::NONE))?;
  for ch in "abc".chars() {
    session.interact(key(KeyCode::Char(ch), KeyModifiers::NONE))?;
  }
  session.interact(key(KeyCode::Esc, KeyModifiers::NONE))?;
  session.interact(key(KeyCode::Char('a'), KeyModifiers::CONTROL))?;
  session.interact(key(KeyCode::Char('!'), KeyModifiers::NONE))?;
  assert_eq!(session.app().overlays.prompt().unwrap().text(), "abc!");
  session.interact(key(KeyCode::Char('a'), KeyModifiers::SUPER))?;
  session.interact(key(KeyCode::Char('x'), KeyModifiers::NONE))?;
  assert_eq!(session.app().overlays.prompt().unwrap().text(), "xabc!");
  std::fs::write(&config, "[reading]\ninactive_brightness = 0.3")?;
  session.interact(key(KeyCode::Char('r'), KeyModifiers::SUPER))?;
  assert_eq!(session.app().settings.inactive_brightness(), 0.3);
  assert_eq!(session.app().overlays.prompt().unwrap().text(), "xabc!");
  session.interact(key(KeyCode::Char('a'), KeyModifiers::CONTROL))?;
  session.interact(key(KeyCode::Char('y'), KeyModifiers::NONE))?;
  assert_eq!(session.app().overlays.prompt().unwrap().text(), "yxabc!");
  session.interact(key(KeyCode::Esc, KeyModifiers::NONE))?;
  assert!(session.app().overlays.prompt().is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn theme_reload_preserves_real_raster() -> Result<()> {
  use sha2::{Digest, Sha256};
  use vpdf_model::theme::Theme;
  let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
  assert_eq!(
    vpdf_pdf::content_cache::hex(&Sha256::digest(std::fs::read(&path)?)),
    "bdfaa68d8984f0dc02beaca527b76f207d99b666d31d1da728ee0728182df697"
  );
  let directory = tempfile::tempdir()?;
  let config = directory.path().join("config.toml");
  std::fs::write(&config, "")?;
  let mut session = Session::configured(
    None,
    crate::config::Options {
      config: Some(config.clone()),
      ..Default::default()
    }
    .load(),
  );
  session.open(&path, true, cells(100, 32))?;
  let raster = |session: &mut Session| {
    vpdf_pdf::raster::visit_page(
      &mut session.app().doc,
      PageId(0),
      512,
      |_, _, bytes| Ok(Sha256::digest(bytes).to_vec()),
    )
  };
  let original = raster(&mut session)?;
  session.draw_frame(&mut Vec::new())?;
  let keys = session.app().tiles.wanted().to_vec();
  let zoom = session.app().layout.zoom();
  let position = session.app().reader.cursor;
  for &theme in Theme::ALL {
    std::fs::write(
      &config,
      format!("[appearance]\ntheme = '{}'", theme.name()),
    )?;
    session.reload_config();
    assert_eq!(session.app().layout.zoom(), zoom);
    assert_eq!(session.app().reader.cursor, position);
    assert_eq!(session.app().tiles.wanted(), keys);
    assert_eq!(raster(&mut session)?, original, "{theme:?}");
    let mut scene = vpdf_model::scene::Recorder::new(|_| true);
    session.app().scene_pane(&mut scene, false)?;
    let scene = scene.into_scene();
    let has_theme = scene.iter().any(|op| {
      matches!(op, vpdf_model::scene::Op::Theme(selected) if *selected == theme)
    });
    assert_eq!(has_theme, theme != Theme::Default);
  }
  Ok(())
}

#[test]
fn theme_commands_update_session() -> Result<()> {
  use vpdf_model::theme::Theme;
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("one.pdf");
  let second = directory.path().join("two.pdf");
  crate::fixtures::pdf(&path, &["one", "two"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  let mut session = Session::configured(None, settings_file(directory.path()));
  let config = directory.path().join("config.toml");
  let original = std::fs::read(&config)?;
  let screen = cells(120, 40);
  session.open(&path, true, screen)?;
  session.split(Axis::Columns, None, Place::After)?;
  session.open(&second, true, screen)?;
  let position = session.app().reader.cursor;
  let zoom = session.app().layout.zoom();
  let tiles = session.app().tiles.wanted().to_vec();
  for name in ["theme", "colorscheme"] {
    for &theme in Theme::ALL {
      session.app().command(&format!("{name} {}", theme.name()))?;
      session.settle()?;
      assert_eq!(session.settings.theme(), theme);
      for tab in &session.tabs {
        for app in &tab.panes {
          assert_eq!(app.settings.theme(), theme);
        }
      }
      assert_eq!(session.app().reader.cursor, position);
      assert_eq!(session.app().layout.zoom(), zoom);
      assert_eq!(session.app().tiles.wanted(), tiles);
    }
    session.app().command(&format!("{name} nord"))?;
    session.settle()?;
    for argument in ["", "unknown", "nord extra"] {
      assert!(session.app().command(&format!("{name} {argument}")).is_err());
      assert_eq!(session.app().request, None);
      assert_eq!(session.settings.theme(), Theme::Nord);
    }
  }
  session.split(Axis::Columns, None, Place::After)?;
  assert_eq!(session.app().settings.theme(), Theme::Nord);
  session.open(&path, true, screen)?;
  assert_eq!(session.app().settings.theme(), Theme::Nord);
  assert_eq!(std::fs::read(&config)?, original);
  session.reload_config();
  assert_eq!(session.settings.theme(), Theme::Default);
  assert_eq!(std::fs::read(&config)?, original);
  Ok(())
}
