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
  app.open_panel(PanelKind::Help)?;
  assert!(app.panel_focused(), "reopening keeps the panel");
  key(app, b"q")?;
  assert!(app.panel.is_none() && !app.quit);

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
    assert_eq!(
      session.tabs.iter().map(|tab| tab.panes.len()).sum::<usize>(),
      2
    );
  }
  let mut session = Session::new("", false, None);
  session.open(&file("paper.pdf")?, true, screen())?;
  session.app().show_panel(
    PanelKind::Recent,
    vec![row(Destination::File(file("missing.pdf")?))],
  )?;
  key(session.app(), b"\r")?;
  assert!(session.settle().is_err());
  assert!(session.app().panel_focused());
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
  app.open_panel(PanelKind::Help)?;
  app.command("overview")?;
  assert_eq!(app.input_owner(), InputOwner::Overview);
  let selected = app.overview.as_ref().unwrap().selected;
  key(&mut app, b"\tj")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert_eq!(app.overview.as_ref().unwrap().selected, selected);
  assert_eq!(app.panel.as_ref().unwrap().selected, 1);
  key(&mut app, b"\t")?;
  assert_eq!(app.input_owner(), InputOwner::Overview);
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
  let mut app = App::with_screen(&file("book.pdf")?, "", false, screen())?;
  crate::common::outline_loaded(&mut app)?;
  app.open_panel(PanelKind::Outline)?;
  key(&mut app, b"60j")?;
  app.panel.as_mut().unwrap().search.query = "chapter".into();
  let panel = app.panel.as_ref().unwrap();
  let state = (panel.selected, panel.top, panel.search.query.clone());
  key(&mut app, b"\t")?;
  app.draw(&mut Vec::new())?;
  key(&mut app, b"\x1b[Z")?;
  let panel = app.panel.as_ref().unwrap();
  assert_eq!((panel.selected, panel.top, panel.search.query.clone()), state);
  assert!(panel.focused);
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
