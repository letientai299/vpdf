use std::collections::VecDeque;
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::{App, InputOwner, Window};
use vpdf::decode::{Event, Parser};
use vpdf::document::Target;
use vpdf::hints::{Hint, Hints};
use vpdf::keys::{Action, Input, Keys, PanelKind};
use vpdf::layout::Screen;
use vpdf::panel::{Destination, Row};
use vpdf::prompt::History;
use vpdf::session::Session;
use vpdf::split::Direction;
use vpdf::text::{PageId, Position};

fn screen() -> Screen {
  Screen {
    columns: 121,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  }
}

fn file(name: &str) -> Result<PathBuf> {
  Ok(PathBuf::from(std::env::var("VPDF_CORPUS")?).join(name))
}

fn events(bytes: &[u8]) -> VecDeque<Event> {
  let mut events = VecDeque::new();
  Parser::default().feed(bytes, &mut events);
  events
}

fn key(app: &mut App, bytes: &[u8]) -> Result<()> {
  let mut history = History::default();
  for event in events(bytes) {
    let Event::Key(key) = event else { panic!("key bytes") };
    app.key(key, &mut history)?;
  }
  Ok(())
}

fn send(session: &mut Session, bytes: &[u8]) -> Result<()> {
  for event in events(bytes) {
    match event {
      Event::Key(event) => {
        session.app().key(event, &mut History::default())?;
      }
      Event::Mouse(mouse) => {
        session.mouse(mouse.cells().expect("cell report"))?;
      }
      _ => panic!("input bytes"),
    }
    session.settle()?;
  }
  Ok(())
}

fn action(app: &mut App, action: Action) -> Result<()> {
  app.action(Input { action, count: 1, explicit: false, register: '"' })
}

fn hints(app: &mut App) {
  app.hints = Some(Hints {
    items: vec![Hint {
      label: "aa".into(),
      target: Target::Page(PageId(1)),
      cell: (10, 5),
    }],
    typed: String::new(),
    copy: false,
  });
}

fn row(destination: Destination) -> Row {
  Row::new("destination".into(), String::new(), destination)
}

#[test]
fn raw_focus_and_history_keys_differ() {
  let mut keys = Keys::default();
  for (bytes, expected) in [
    (&b"\t"[..], Action::LocalFocus),
    (&b"\x1b[9;2u"[..], Action::LocalFocus),
    (&b"\x1b[Z"[..], Action::LocalFocus),
    (&b"\x1b[105;5u"[..], Action::History(true)),
    (&b"\x1b[111;5u"[..], Action::History(false)),
  ] {
    let decoded = events(bytes);
    assert_eq!(decoded.len(), 1, "{bytes:?}");
    let Event::Key(key) = decoded.front().unwrap() else { panic!("key") };
    assert_eq!(keys.feed(*key).unwrap().action, expected, "{bytes:?}");
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn local_focus_preserves_reading_and_recent_cards() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("paper.pdf")?, true, screen())?;
  let app = session.app();
  key(app, b"vjl")?;
  let range = app.reader.selection.map(|range| (range.anchor, range.kind));
  let cursor = app.reader.cursor;
  app.open_panel(PanelKind::Help)?;
  app.panel.as_mut().unwrap().search.query = "focus".into();
  key(app, b"\t")?;
  assert_eq!(app.input_owner(), InputOwner::Content);
  key(app, b"\x1b[Z")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    range
  );
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(app.panel.as_ref().unwrap().search.query, "focus");
  key(app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert!(app.reader.selection.is_none());
  app.open_panel(PanelKind::Help)?;
  assert!(app.panel_focused(), "reopening keeps the panel");
  key(app, b"q")?;
  assert!(app.panel.is_none() && !app.quit);

  key(session.app(), b"v")?;
  session.app().command("recent")?;
  session.settle()?;
  let app = session.app();
  assert!(app.covered());
  let selected = app.panel.as_ref().unwrap().selected;
  key(app, b"\t")?;
  assert!(!app.covered() && app.layout.screen.columns > 0);
  assert!(app.panel.as_ref().unwrap().shelf.is_some());
  key(app, b"\t")?;
  assert!(app.covered());
  assert_eq!(app.panel.as_ref().unwrap().selected, selected);
  key(app, b"\r")?;
  session.settle()?;
  assert!(!session.app().covered());
  assert!(session.app().panel.as_ref().unwrap().shelf.is_some());
  assert!(session.app().reader.selection.is_none());
  assert_eq!(session.tabs.len(), 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_activation_retains_lists_and_failure_focus() -> Result<()> {
  let mut app = App::with_screen(&file("book.pdf")?, "", false, screen())?;
  let destination = Position { page: PageId(20), ..Position::default() };
  app.show_panel(
    PanelKind::Results,
    vec![row(Destination::Position(destination))],
  )?;
  key(&mut app, b"o")?;
  assert!(app.panel_focused());
  assert_eq!(app.reader.cursor.page, destination.page);
  key(&mut app, b"\r")?;
  assert!(!app.panel_focused());
  assert_eq!(app.panel.as_ref().unwrap().rows.len(), 1);
  assert!(app.reader.selection.is_none());
  key(&mut app, b"\t")?;
  app.panel.as_mut().unwrap().rows = vec![row(Destination::None)];
  key(&mut app, b"\r")?;
  assert!(app.panel_focused());
  app.panel.as_mut().unwrap().rows =
    vec![row(Destination::Link(Target::Uri("file:///bad".into())))];
  assert!(key(&mut app, b"\r").is_err());
  assert!(app.panel_focused());
  key(&mut app, b"gq")?;
  assert!(app.panel.is_none() && !app.quit);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cross_document_panel_placement_and_reuse() -> Result<()> {
  for (activation, focus) in [(&b"\r"[..], true), (&b"o"[..], false)] {
    let mut session = Session::new("", false, None);
    session.open(&file("paper.pdf")?, true, screen())?;
    let source = session.app().id;
    session.app().show_panel(
      PanelKind::Recent,
      vec![row(Destination::File(file("book.pdf")?))],
    )?;
    send(&mut session, activation)?;
    assert_eq!(session.tabs.len(), if focus { 2 } else { 1 });
    assert_eq!(session.tab().panes.len(), if focus { 1 } else { 2 });
    assert_eq!(session.app().id == source, !focus);
    if focus {
      assert_eq!(session.app().input_owner(), InputOwner::Content);
      send(&mut session, b"vwgo")?;
      assert!(session.app().reader.selection.is_some());
      session.activate(0)?;
    }
    assert!(session.app().panel_focused());
    send(&mut session, b"o")?;
    assert_eq!(session.app().id, source);
    assert_eq!(
      session.tabs.iter().map(|tab| tab.panes.len()).sum::<usize>(),
      2
    );
    send(&mut session, b"\r")?;
    assert_ne!(session.app().id, source);
    assert_eq!(session.app().input_owner(), InputOwner::Content);
    assert!(session.app().reader.selection.is_none());
    assert_eq!(
      session.tabs.iter().map(|tab| tab.panes.len()).sum::<usize>(),
      2
    );
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn failed_activation_preserves_view() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("paper.pdf")?, true, screen())?;
  send(&mut session, b"vw")?;
  let id = session.app().id;
  let cursor = session.app().reader.cursor;
  let anchor = session.app().reader.selection.unwrap().anchor;
  session.app().show_panel(
    PanelKind::Recent,
    vec![row(Destination::File(file("missing.pdf")?))],
  )?;
  let viewport = session.app().reader.scroll;
  key(session.app(), b"\r")?;
  assert!(session.settle().is_err());
  assert_eq!((session.active, session.app().id), (0, id));
  assert_eq!((session.tabs.len(), session.tab().panes.len()), (1, 1));
  assert_eq!(session.app().reader.cursor, cursor);
  assert_eq!(session.app().reader.scroll, viewport);
  assert_eq!(session.app().reader.selection.unwrap().anchor, anchor);
  assert_eq!(session.app().input_owner(), InputOwner::Panel);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn transient_prefix_dispatch_and_noop_focus() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  send(&mut session, b"\x17v")?;
  let source = session.app().id;
  let left = session.app().reader.cursor;
  hints(session.app());
  send(&mut session, b"\x1b[119;5u")?;
  assert!(session.app().hints.is_none());
  assert_eq!(session.app().keys.awaiting(), Some("C-w"));
  send(&mut session, b"l")?;
  assert_ne!(session.app().id, source);
  assert_eq!(
    session
      .tab()
      .panes
      .iter()
      .find(|pane| pane.id == source)
      .unwrap()
      .reader
      .cursor,
    left
  );
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().id, source);
  send(&mut session, b"\x17W")?;
  assert_ne!(session.app().id, source);

  let app = session.app();
  hints(app);
  action(app, Action::Window(Window::Focus(Direction::Right)))?;
  session.settle()?;
  assert!(session.app().hints.is_some(), "edge focus does not blur");
  let edge = session.app().id;
  let cursor = session.app().reader.cursor;
  send(&mut session, b"\x17l")?;
  assert_eq!(session.app().id, edge);
  assert_eq!(session.app().reader.cursor, cursor);
  assert!(session.app().hints.is_none(), "input dismisses before no-op");
  assert!(session.app().keys.awaiting().is_none());
  action(session.app(), Action::Window(Window::Next))?;
  session.settle()?;
  assert_eq!(session.app().id, source);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn outside_click_consumes_capture_across_panes() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  send(&mut session, b"\x17v")?;
  let source = session.app().id;
  let tab = session.active;
  let viewport = session.app().reader.scroll;
  send(&mut session, b"/partial")?;
  assert_eq!(session.app().input_owner(), InputOwner::Prompt);
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_eq!((session.active, session.app().id), (tab, source));
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert!(session.app().search_origin.is_none());
  assert_eq!(session.app().search.query.text, "");
  assert_eq!(session.app().reader.scroll, viewport);
  assert!(session.app().reader.selection.is_none());
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_ne!(session.app().id, source);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().id, source);
  key(session.app(), b"v")?;
  hints(session.app());
  action(session.app(), Action::Command)?;
  send(&mut session, b"\x1b[<0;90;10M")?;
  assert_eq!(session.app().id, source);
  assert!(session.app().prompt.is_none());
  assert!(session.app().hints.is_some(), "only the top transient closes");
  assert!(session.app().reader.selection.is_none());
  send(&mut session, b"\x1b[<32;90;20M\x1b[<0;90;20m")?;
  assert_eq!(session.app().id, source);
  assert!(session.app().reader.selection.is_none());
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_eq!(session.app().id, source, "hints consume the next click");
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_ne!(session.app().id, source);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn prompt_wheel_and_passive_motion_keep_owner() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  send(&mut session, b"\x17v")?;
  let source = session.app().id;
  hints(session.app());
  send(&mut session, b"\x1b[<35;90;10M")?;
  assert!(session.app().hints.is_some());
  assert_eq!(session.app().id, source);
  action(session.app(), Action::Command)?;
  let before: Vec<_> = session
    .tab()
    .panes
    .iter()
    .map(|app| (app.reader.scroll, app.animation.is_some()))
    .collect();
  send(&mut session, b"\x1b[<65;90;10M")?;
  let after: Vec<_> = session
    .tab()
    .panes
    .iter()
    .map(|app| (app.reader.scroll, app.animation.is_some()))
    .collect();
  assert_eq!(before, after);
  assert_eq!(session.app().input_owner(), InputOwner::Prompt);
  send(&mut session, b"\t\x1b[Z")?;
  assert!(session.app().prompt.is_some());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn overview_and_text_use_local_owner() -> Result<()> {
  let mut app = App::with_screen(&file("book.pdf")?, "", false, screen())?;
  crate::common::outline_loaded(&mut app)?;
  key(&mut app, b"gogp")?;
  assert_eq!(app.input_owner(), InputOwner::Overview);
  let selected = app.overview.as_ref().unwrap().selected;
  key(&mut app, b"\tj")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert_eq!(app.overview.as_ref().unwrap().selected, selected);
  assert_eq!(app.panel.as_ref().unwrap().selected, 1);
  key(&mut app, b"\tj")?;
  assert_eq!(app.input_owner(), InputOwner::Overview);
  assert!(app.overview.as_ref().unwrap().selected > selected);
  assert_eq!(app.panel.as_ref().unwrap().selected, 1);
  let mut status = Vec::new();
  app.status(&mut status, app.screen.rows - 1)?;
  assert!(String::from_utf8_lossy(&status).contains("PAGES"));
  key(&mut app, b"\x1b[27u")?;
  key(&mut app, b"vjl")?;
  let selection = app.reader.selection.map(|range| (range.anchor, range.kind));
  app.command("text")?;
  assert!(app.text_mode);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    selection
  );
  app.command("text")?;
  assert!(app.text_mode, "opening text focuses its presentation");
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    selection
  );
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_history_is_separate_from_local_focus() -> Result<()> {
  let mut app = App::with_screen(&file("book.pdf")?, "", false, screen())?;
  app.command("page 20")?;
  app.command("page 30")?;
  key(&mut app, b"\x1b[111;5u")?;
  assert_eq!(app.reader.cursor.page, PageId(19));
  key(&mut app, b"\t")?;
  assert_eq!(app.reader.cursor.page, PageId(19));
  key(&mut app, b"\x1b[105;5u")?;
  assert_eq!(app.reader.cursor.page, PageId(29));
  key(&mut app, b"go")?;
  let cursor = app.reader.cursor;
  key(&mut app, b":te\t")?;
  assert_eq!(app.input_owner(), InputOwner::Prompt);
  assert_eq!(app.prompt.as_ref().unwrap().text, "text");
  assert_eq!(app.reader.cursor, cursor);
  key(&mut app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn peek_prompt_retains_destination_and_owner() -> Result<()> {
  let mut app = App::with_screen(&file("paper.pdf")?, "", true, screen())?;
  let link = app
    .doc
    .links(PageId(1))?
    .into_iter()
    .find(|link| !matches!(link.target, Target::Uri(_)))
    .unwrap();
  let position = app.doc.text(PageId(1))?.nearest(
    PageId(1),
    f32::midpoint(link.bounds.x0, link.bounds.x1),
    f32::midpoint(link.bounds.y0, link.bounds.y1),
  );
  app.go_to(position, true)?;
  app.preview_link()?;
  let target = format!("{:?}", app.peek.as_ref().unwrap().target);
  key(&mut app, b":")?;
  assert_eq!(app.input_owner(), InputOwner::Prompt);
  assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
  key(&mut app, b"\t\x1b[Z")?;
  assert!(app.peek.is_some() && app.prompt.is_some());
  key(&mut app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Peek);
  assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
  key(&mut app, b"\x1b[119;5u")?;
  assert!(app.peek.is_none());
  assert_eq!(app.keys.awaiting(), Some("C-w"));
  key(&mut app, b"w")?;
  assert!(app.keys.awaiting().is_none());
  assert!(matches!(
    app.request,
    Some(vpdf::app::Request::Window { command: Window::Next, .. })
  ));
  app.request = None;
  app.open_panel(PanelKind::Help)?;
  hints(&mut app);
  key(&mut app, b"\t")?;
  assert_eq!(app.input_owner(), InputOwner::Content);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn clicked_file_and_global_mark_keep_source_panel() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("paper.pdf")?, true, screen())?;
  let source = session.app().id;
  session.app().show_panel(
    PanelKind::Recent,
    vec![row(Destination::File(file("book.pdf")?))],
  )?;
  session.app().draw(&mut Vec::new())?;
  send(&mut session, b"\x1b[<0;2;2M\x1b[<0;2;2m")?;
  assert_eq!(session.app().id, source);
  assert!(session.app().panel_focused());
  assert_eq!(session.tab().panes.len(), 2);
  session.app().request = Some(vpdf::app::Request::GoMark('A', true));
  assert!(session.settle().is_err());
  session.app().set_mark('A')?;
  session.settle()?;
  session
    .app()
    .show_panel(PanelKind::Marks, vec![row(Destination::Mark('A'))])?;
  send(&mut session, b"o")?;
  assert_eq!(session.app().id, source);
  assert!(session.app().panel_focused());
  send(&mut session, b"\r")?;
  assert_eq!(session.app().id, source);
  assert!(!session.app().panel_focused());
  assert_eq!(session.tab().panes.len(), 2);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn outline_row_survives_focus_and_draw() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  let app = session.app();
  crate::common::outline_loaded(app)?;
  key(app, b"go60j")?;
  app.panel.as_mut().unwrap().search.query = "chapter".into();
  let panel = app.panel.as_ref().unwrap();
  let state = (panel.selected, panel.top, panel.search.query.clone());
  key(app, b"\t")?;
  app.draw(&mut Vec::new())?;
  key(app, b"\x1b[Z")?;
  let panel = app.panel.as_ref().unwrap();
  assert_eq!((panel.selected, panel.top, panel.search.query.clone()), state);
  assert!(panel.focused);
  send(&mut session, b"\t")?;
  let id = session.app().id;
  let viewport = session.app().reader.scroll;
  send(&mut session, b"\x1b[<64;2;5M")?;
  let app = session.app();
  let panel = app.panel.as_ref().unwrap();
  let top = panel.top;
  assert!(top < state.1);
  assert_eq!(panel.selected, state.0);
  assert_eq!(app.input_owner(), InputOwner::Content);
  app.draw(&mut Vec::new())?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.panel.as_ref().unwrap().top, top);
  assert_eq!(app.panel.as_ref().unwrap().selected, state.0);
  assert_eq!(app.id, id);
  assert_eq!(app.reader.scroll, viewport);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn narrow_recent_press_restores_cover_geometry() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("paper.pdf")?, true, screen())?;
  let source = session.app().id;
  session.app().command("recent")?;
  session.settle()?;
  for click in [
    b"\x1b[<0;2;2M\x1b[<0;2;2m".as_slice(),
    b"\x1b[<0;2;10M\x1b[<0;2;10m".as_slice(),
  ] {
    send(&mut session, b"\t")?;
    assert!(!session.app().covered());
    session.app().draw(&mut Vec::new())?;
    send(&mut session, click)?;
    assert_eq!(session.app().id, source);
    let app = session.app();
    assert!(app.covered() && app.panel_focused());
    assert_eq!(app.left, app.screen.columns);
    assert_eq!(app.layout.screen.columns, app.screen.columns);
    assert!(app.panel.as_ref().unwrap().shelf.is_some());
    let mut out = Vec::new();
    app.draw(&mut out)?;
    assert!(String::from_utf8_lossy(&out).contains("Enter open"));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn placement_reuses_durable_views() -> Result<()> {
  let mut session = Session::new("", false, None);
  let paper = file("paper.pdf")?;
  let book = file("book.pdf")?;
  session.open(&paper, true, screen())?;
  send(&mut session, b":page 2\rvw")?;
  let target = session.app().id;
  session.app().start_search("attention", false, false)?;
  session.app().show_panel(
    PanelKind::Outline,
    (0..80).map(|_| row(Destination::None)).collect(),
  )?;
  let app = session.app();
  let panel = app.panel.as_mut().unwrap();
  panel.selected = 52;
  panel.top = 40;
  panel.search.query = "committed".into();
  let anchor = app.reader.selection.unwrap().anchor;
  let cursor = app.reader.cursor;
  let scroll = app.reader.scroll;
  let generation = app.search.generation;
  session.open(&book, true, screen())?;
  let source = session.app().id;
  for command in ["e", "tabe", "split", "vsplit"] {
    send(&mut session, format!(":{command} {}\r", paper.display()).as_bytes())?;
    assert_eq!(session.app().id, target, "{command}");
    assert_eq!(session.tabs.len(), 2, "{command}");
    assert!(session.tabs.iter().all(|tab| tab.panes.len() == 1));
    let app = session.app();
    assert_eq!(app.reader.selection.unwrap().anchor, anchor);
    assert_eq!(app.reader.cursor, cursor);
    assert!((app.reader.scroll - scroll).abs() < 0.1);
    assert_eq!(app.input_owner(), InputOwner::Panel);
    let panel = app.panel.as_ref().unwrap();
    assert_eq!((panel.selected, panel.top), (52, 40));
    assert_eq!(panel.search.query, "committed");
    assert_eq!(app.search.query.text, "attention");
    assert_eq!(app.search.generation, generation);
    assert!(matches!(app.search.phase, vpdf::find::Phase::Running { .. }));
    session.open(&book, true, screen())?;
    assert_eq!(session.app().id, source);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn current_view_requests_keep_transients() -> Result<()> {
  use vpdf::app::Request;
  use vpdf::reader::Operator;
  use vpdf::split::Axis;
  let mut session = Session::new("", false, None);
  let path = file("paper.pdf")?;
  session.open(&path, true, screen())?;
  send(&mut session, b"vw:")?;
  hints(session.app());
  session.app().reader.operator = Some(Operator { count: 1, register: '"' });
  let id = session.app().id;
  let cursor = session.app().reader.cursor;
  for request in [
    Request::Open { path: path.clone(), tab: false },
    Request::Open { path: path.clone(), tab: true },
    Request::Window {
      command: Window::Split(Axis::Rows),
      count: 1,
      explicit: false,
      file: Some(path.clone()),
    },
    Request::Window {
      command: Window::Split(Axis::Columns),
      count: 1,
      explicit: false,
      file: Some(path),
    },
    Request::Window {
      command: Window::Focus(Direction::Left),
      count: 1,
      explicit: false,
      file: None,
    },
    Request::Window {
      command: Window::Next,
      count: 1,
      explicit: false,
      file: None,
    },
    Request::Window {
      command: Window::Only,
      count: 1,
      explicit: false,
      file: None,
    },
    Request::Window {
      command: Window::ToTab,
      count: 1,
      explicit: false,
      file: None,
    },
  ] {
    session.app().request = Some(request);
    session.settle()?;
    let app = session.app();
    assert_eq!(app.id, id);
    assert_eq!(app.reader.cursor, cursor);
    assert!(app.reader.selection.is_some());
    assert!(app.reader.operator.is_some());
    assert!(app.prompt.is_some() && app.hints.is_some());
  }
  action(session.app(), Action::LocalFocus)?;
  assert!(session.app().prompt.is_some() && session.app().hints.is_some());
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.tab().panes.len(), 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn new_views_blur_source_and_start_normal() -> Result<()> {
  for command in ["e", "tabe", "split", "vsplit"] {
    let mut session = Session::new("", false, None);
    session.open(&file("book.pdf")?, true, screen())?;
    send(&mut session, b":page 30\rvwgo")?;
    let source = session.app().id;
    let anchor = session.app().reader.selection.unwrap().anchor;
    send(&mut session, b"g")?;
    assert!(session.app().keys.awaiting().is_some());
    let path = file("paper.pdf")?;
    session.app().command(&format!("{command} {}", path.display()))?;
    session.settle()?;
    let app = session.app();
    assert_ne!(app.id, source, "{command}");
    assert!(app.reader.selection.is_none() && app.reader.operator.is_none());
    assert_eq!(app.input_owner(), InputOwner::Content);
    let old = session
      .tabs
      .iter()
      .flat_map(|tab| &tab.panes)
      .find(|app| app.id == source);
    if let Some(old) = old {
      assert_eq!(old.reader.selection.unwrap().anchor, anchor);
      assert_eq!(old.input_owner(), InputOwner::Panel);
      assert!(old.keys.awaiting().is_none());
      assert!(
        old.prompt.is_none() && old.hints.is_none() && old.peek.is_none()
      );
    }
    let expected = match command {
      "e" => (1, 1),
      "tabe" => (2, 1),
      _ => (1, 2),
    };
    assert_eq!((session.tabs.len(), session.tab().panes.len()), expected);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn detach_returns_to_live_peer() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  let right = session.app().id;
  send(&mut session, b"\x17v")?;
  let left = session.app().id;
  send(&mut session, b"\x17s")?;
  let top = session.app().id;
  send(&mut session, b"\x17l\x17h")?;
  assert_eq!(session.app().id, top);
  send(&mut session, b"vwgo")?;
  let anchor = session.app().reader.selection.unwrap().anchor;
  send(&mut session, b"\t\t")?;
  assert_eq!(session.app().reader.selection.unwrap().anchor, anchor);
  assert_eq!(session.app().input_owner(), InputOwner::Panel);
  send(&mut session, b"\x17T")?;
  assert_eq!(session.app().id, top);
  assert!(session.app().reader.selection.is_some());
  assert!(session.app().panel_focused());
  assert!(session.app().prompt.is_none());
  assert_eq!(session.tabs.len(), 2);
  send(&mut session, b"gT")?;
  assert_eq!(session.app().id, right, "detach prefers the live last peer");
  send(&mut session, b"gt")?;
  assert_eq!(session.app().reader.selection.unwrap().anchor, anchor);
  send(&mut session, b":tabclose\r")?;
  assert_eq!(session.app().id, right, "tab restores its focused split");
  send(&mut session, b"\x17h\x17c")?;
  assert_eq!(session.app().id, right);
  assert!(session.tab().panes.iter().all(|app| app.id != left));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn transitions_drop_capture_and_owned_input() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  send(&mut session, b":page 30\r\x17v")?;
  let left = session.app().id;
  send(&mut session, b"\x1b[<0;10;5M\x1b[<32;15;6M")?;
  assert!(session.app().reader.selection.is_some());
  let before = session.app().reader.cursor;
  send(&mut session, b"\x17h\x1b[<32;20;8M")?;
  let cursor = session.app().reader.cursor;
  assert_ne!(cursor, before, "edge focus retains capture");
  send(&mut session, b"\x17l")?;
  let right = session.app().id;
  send(&mut session, b"\x1b[<32;95;10M\x1b[<0;95;10m")?;
  let old = session.tab().panes.iter().find(|app| app.id == left).unwrap();
  assert_eq!(old.reader.cursor, cursor, "old capture no longer routes");
  assert!(old.reader.selection.is_some());
  send(&mut session, b"y")?;
  assert!(session.app().reader.operator.is_some());
  session.app().command(&format!("tabe {}", file("form.pdf")?.display()))?;
  session.settle()?;
  let old = session
    .tabs
    .iter()
    .flat_map(|tab| &tab.panes)
    .find(|app| app.id == right)
    .unwrap();
  assert!(old.reader.operator.is_none());
  send(&mut session, b"gT\x1b[<0;10;5M\x1b[<32;15;6M")?;
  assert_eq!(session.app().id, left);
  assert!(session.app().reader.selection.is_some(), "new press captures");
  send(&mut session, b"\x1b[<0;15;6m:page")?;
  session.app().command(&format!("tabe {}", file("ocr.pdf")?.display()))?;
  session.settle()?;
  let old = session
    .tabs
    .iter()
    .flat_map(|tab| &tab.panes)
    .find(|app| app.id == left)
    .unwrap();
  assert!(old.prompt.is_none(), "new tab closes source prompt");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn close_uses_spatial_peer_after_history() -> Result<()> {
  let mut session = Session::new("", false, None);
  session.open(&file("book.pdf")?, true, screen())?;
  let bottom = session.app().id;
  send(&mut session, b"\x17v\x17l\x17s")?;
  let middle = session.app().id;
  send(&mut session, b"\x17j\x17k\x17s\x17c")?;
  assert_eq!(session.app().id, middle, "close prefers last live peer");
  send(&mut session, b"\x17c")?;
  assert_eq!(session.app().id, bottom, "spatial peer precedes first leaf");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn current_split_reuse_is_noop() -> Result<()> {
  let mut session = Session::new("", false, None);
  let path = file("book.pdf")?;
  session.open(&path, true, screen())?;
  send(&mut session, b":page 30\r\x17vvwgo")?;
  let current = session.app().id;
  session.app().start_search("chapter", false, false)?;
  let app = session.app();
  let anchor = app.reader.selection.unwrap().anchor;
  let cursor = app.reader.cursor;
  let viewport = app.reader.scroll;
  let generation = app.search.generation;
  let panel = app.panel.as_ref().unwrap();
  let rows = (panel.selected, panel.top, panel.search.query.clone());
  let tab = session.active;
  for command in ["e", "tabe", "split", "vsplit"] {
    send(&mut session, format!(":{command} {}\r", path.display()).as_bytes())?;
    assert_eq!((session.active, session.app().id), (tab, current), "{command}");
    let app = session.app();
    assert_eq!(app.reader.selection.unwrap().anchor, anchor);
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.scroll, viewport);
    assert_eq!(app.input_owner(), InputOwner::Panel);
    let panel = app.panel.as_ref().unwrap();
    assert_eq!((panel.selected, panel.top, panel.search.query.clone()), rows);
    assert_eq!(app.search.query.text, "chapter");
    assert_eq!(app.search.generation, generation);
    assert!(matches!(app.search.phase, vpdf::find::Phase::Running { .. }));
    assert_eq!(session.tab().panes.len(), 2);
    assert_eq!(session.tabs.len(), 1);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_entry_drops_pending_operator() -> Result<()> {
  for command in ["go", ":recent\r", ":marks\r"] {
    let mut session = Session::new("", false, None);
    session.open(&file("book.pdf")?, true, screen())?;
    send(&mut session, b"y")?;
    assert!(session.app().reader.operator.is_some());
    send(&mut session, command.as_bytes())?;
    assert!(session.app().panel_focused());
    assert!(session.app().reader.operator.is_none());
    if command == "go" {
      send(&mut session, b"q")?;
      assert!(session.app().panel.is_none());
      assert!(session.app().reader.operator.is_none());
      send(&mut session, b"go")?;
    }
    send(&mut session, b"\t")?;
    session.app().reader.operator =
      Some(vpdf::reader::Operator { count: 1, register: '"' });
    send(&mut session, command.as_bytes())?;
    assert!(session.app().reader.operator.is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_unopened_placement_routes() -> Result<()> {
  for (command, tabs, panes) in [("e", 1, 1), ("tabe", 2, 1), ("split", 1, 2)] {
    let mut session = Session::new("", false, None);
    session.open(&file("book.pdf")?, true, screen())?;
    let source = session.app().id;
    let path = file("paper.pdf")?;
    send(&mut session, format!(":{command} {}\r", path.display()).as_bytes())?;
    assert_ne!(session.app().id, source, "{command}");
    assert_eq!(session.active, tabs - 1);
    assert_eq!((session.tabs.len(), session.tab().panes.len()), (tabs, panes));
    let app = session.app();
    assert_eq!(app.path, path);
    assert_eq!(app.input_owner(), InputOwner::Content);
    assert!(app.reader.selection.is_none());
    assert!(app.search.worker.is_none() && app.ocr.is_none());
  }
  Ok(())
}
