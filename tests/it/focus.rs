use super::common::SeedReader;
use super::common::keys;
use vpdf::Result;
use vpdf::app::App;
use vpdf::pdf_text::PdfText;
use vpdf::session::Session;
use vpdf_model::document::Target;
use vpdf_model::input::Event;
use vpdf_model::text::{PageId, Position};
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::{Action, Input, Keys, PanelKind};
use vpdf_ui::panel::{Destination, Row};
use vpdf_ui::split::Direction;
use vpdf_ui::window::Window;

pub(super) use crate::harness::{action, screen};
use crate::harness::{corpus as file, dispatch, raw as events, session};

pub(super) fn send(session: &mut Session, bytes: &[u8]) -> Result<()> {
  for event in events(bytes) {
    dispatch(session, &event)?;
    session.settle()?;
  }
  Ok(())
}

fn hints(app: &mut App) {
  app.overlays.open_hints(super::common::hints(
    24,
    (10, 5),
    &Target::Page(PageId(1)),
  ));
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
    assert_eq!(
      keys.feed(*key, vpdf_ui::keys::Grammar::Normal).unwrap().action,
      expected,
      "{bytes:?}"
    );
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn local_focus_preserves_reading_and_recent_cards() -> Result<()> {
  let mut session = session(&file("paper.pdf")?, screen())?;
  let app = session.app();
  keys(app, b"vjl")?;
  let range = app.reader.selection.map(|range| (range.anchor, range.kind));
  let cursor = app.reader.cursor;
  app.open_panel(PanelKind::Version)?;
  app.panel.as_mut().unwrap().view.finish_search("focus");
  keys(app, b"\t")?;
  assert_eq!(app.input_owner(), InputOwner::Content);
  keys(app, b"\x1b[Z")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    range
  );
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(app.panel.as_ref().unwrap().view.query(), "focus");
  keys(app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert!(app.reader.selection.is_none());
  app.open_panel(PanelKind::Version)?;
  assert!(app.panel_focused(), "reopening keeps the panel");
  keys(app, b"q")?;
  assert!(app.panel.is_none() && !app.quit);

  keys(session.app(), b"v")?;
  session.app().command("recent")?;
  session.settle()?;
  let app = session.app();
  assert!(app.covered());
  let selected = app.panel.as_ref().unwrap().view.selected();
  keys(app, b"\t")?;
  assert!(!app.covered() && app.layout.screen().columns > 0);
  assert!(app.panel.as_ref().unwrap().view.is_recent_view());
  keys(app, b"\t")?;
  assert!(app.covered());
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), selected);
  keys(app, b"\r")?;
  session.settle()?;
  assert!(!session.app().covered());
  assert!(session.app().panel.as_ref().unwrap().view.is_recent_view());
  assert!(session.app().reader.selection.is_none());
  assert_eq!(session.tabs.len(), 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_activation_retains_lists_and_failure_focus() -> Result<()> {
  let mut app = crate::harness::solo("book.pdf", false, screen())?;
  let destination = Position { page: PageId(20), ..Position::default() };
  app.show_panel(
    PanelKind::Results,
    vec![row(Destination::Position(destination))],
  )?;
  keys(&mut app, b"o")?;
  assert!(app.panel_focused());
  assert_eq!(app.reader.cursor.page, destination.page);
  keys(&mut app, b"\r")?;
  assert!(!app.panel_focused());
  assert_eq!(app.panel.as_ref().unwrap().view.rows().len(), 1);
  assert!(app.reader.selection.is_none());
  keys(&mut app, b"\t")?;
  app.panel.as_mut().unwrap().view.refill(vec![row(Destination::None)], "");
  keys(&mut app, b"\r")?;
  assert!(app.panel_focused());
  app.panel.as_mut().unwrap().view.refill(
    vec![row(Destination::Link(Target::Uri("file:///bad".into())))],
    "",
  );
  assert!(keys(&mut app, b"\r").is_err());
  assert!(app.panel_focused());
  keys(&mut app, b"gq")?;
  assert!(app.panel.is_none() && !app.quit);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn recent_split_reuses_target() -> Result<()> {
  let mut session = session(&file("paper.pdf")?, screen())?;
  let source = session.app().id;
  session.app().show_panel(
    PanelKind::Recent,
    vec![row(Destination::File(file("book.pdf")?))],
  )?;
  let middle = b"\x1b[<1;2;2M\x1b[<1;2;2m";
  session.draw_focused(&mut Vec::new())?;
  send(&mut session, middle)?;
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.tab().panes.len(), 2);
  assert_eq!(session.app().id, source);
  assert!(session.app().panel_focused());
  send(&mut session, b"\x17wvw")?;
  assert!(session.app().reader.selection.is_some());
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().id, source);
  session.draw_focused(&mut Vec::new())?;
  send(&mut session, middle)?;
  assert_eq!(session.app().id, source);
  assert!(session.app().panel_focused());
  assert_eq!(session.tab().panes.len(), 2);
  let target = session.tab().panes.iter().find(|app| app.id != source).unwrap();
  assert!(target.reader.selection.is_some());
  send(&mut session, b"\r")?;
  assert_ne!(session.app().id, source);
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert!(session.app().reader.selection.is_none());
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.tab().panes.len(), 2);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn recent_pick_opens_in_covered_pane() -> Result<()> {
  let paper = file("paper.pdf")?.canonicalize()?;
  let book = file("book.pdf")?.canonicalize()?;
  let reuse = format!(":vsplit {}\r\x17pgr", paper.display());
  let setups = ["gr", "\x17sgr", "\x17n", ":tabnew\r", &reuse];
  for (setup, click) in setups.iter().flat_map(|s| [(*s, false), (*s, true)]) {
    let mut session = session(&paper, screen())?;
    session.open(&book, false, screen())?;
    send(&mut session, setup.as_bytes())?;
    let list = session.app().panel.as_mut().unwrap();
    assert!(list.view.is_recent_view(), "{setup:?}");
    let picked = |row: &Row| match &row.destination {
      Destination::File(path) => *path == paper,
      _ => false,
    };
    let index = list.view.rows().iter().position(picked).unwrap();
    list.view.choose(index);
    let panes = session.tab().panes.len();
    send(&mut session, b"o")?;
    assert!(session.app().covered(), "{setup:?}: o is inert");
    assert_eq!(session.tab().panes.len(), panes, "{setup:?}");
    if click {
      session.draw_focused(&mut Vec::new())?;
      let app = session.app();
      let column = app.at.column + 4;
      let row = usize::from(app.at.row) + index
        - app.panel.as_ref().unwrap().view.top();
      send(&mut session, format!("\x1b[<0;{column};{}M", row + 2).as_bytes())?;
    } else {
      send(&mut session, b"\r")?;
    }
    let tabnew = setup == ":tabnew\r";
    assert_eq!(session.tabs.len(), 1 + usize::from(tabnew), "{setup:?}");
    assert_eq!(session.app().path, paper, "{setup:?}");
    assert_eq!(session.app().input_owner(), InputOwner::Content);
    let panes = &session.tab().panes;
    assert!(panes.iter().all(|app| !app.covered()), "{setup:?}");
    let mut paths: Vec<_> = panes.iter().map(|app| &app.path).collect();
    paths.sort();
    let want =
      if setup == "gr" || tabnew { vec![&paper] } else { vec![&book, &paper] };
    assert_eq!(paths, want, "{setup:?}");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn yp_yanks_file_path() -> Result<()> {
  // Scanned pages have no text for `y` to start from.
  for name in ["paper.pdf", "ocr.pdf"] {
    let path = std::fs::canonicalize(file(name)?)?;
    let mut session = session(&path, screen())?;
    send(&mut session, b"\"ayp")?;
    let reader = &mut session.app().reader;
    let yanked = reader.register('a').map(ToString::to_string);
    assert_eq!(yanked.as_deref(), path.to_str(), "{name}");
    assert!(reader.operator.is_none() && reader.viewport_mode(), "{name}");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn yy_joins_prose_and_keeps_code() -> Result<()> {
  use vpdf_reader::reader::Reader;
  let cases = [
    ("paper.pdf", "The dominant sequence", false, "recurrent or convolutional"),
    ("paper.pdf", "The dominant sequence", true, "recurrent or\nconvolutional"),
    ("book.pdf", ">>> a = 'banana'", false, "'banana'\n>>> b = 'banana'\n"),
    ("book.pdf", ">>> a = 'banana'", true, "'banana'\n>>> b = 'banana'\n"),
  ];
  for (name, start, raw, want) in cases {
    let mut doc = vpdf_pdf::document::Document::open(&file(name)?, "")?;
    let mut reader = Reader::default();
    reader.cursor = (0..doc.pages.len())
      .find_map(|page| {
        let text = doc.text(PageId(page)).ok()?;
        let line =
          text.lines.iter().position(|l| l.text().starts_with(start))?;
        Some(Position { page: PageId(page), line, glyph: 0 })
      })
      .unwrap();
    reader.yank(1, '"', &mut PdfText(&mut doc))?;
    if raw {
      reader.yank_raw(3, &mut PdfText(&mut doc))?;
    } else {
      reader.yank(3, '"', &mut PdfText(&mut doc))?;
    }
    let text = reader.pending_yank.take().unwrap().text;
    assert!(text.contains(want), "{name} raw={raw}: {text:?}");
    assert_eq!(
      text.lines().count(),
      if raw || start.starts_with(">>>") { 3 } else { 1 }
    );
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn failed_activation_preserves_view() -> Result<()> {
  let mut session = session(&file("paper.pdf")?, screen())?;
  send(&mut session, b"vw")?;
  let id = session.app().id;
  let cursor = session.app().reader.cursor;
  let anchor = session.app().reader.selection.unwrap().anchor;
  session.app().show_panel(
    PanelKind::Recent,
    vec![row(Destination::File(file("missing.pdf")?))],
  )?;
  let viewport = session.app().reader.scroll;
  keys(session.app(), b"\r")?;
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
  let mut session = session(&file("book.pdf")?, screen())?;
  send(&mut session, b"\x17v")?;
  let source = session.app().id;
  let left = session.app().reader.cursor;
  hints(session.app());
  send(&mut session, b"\x1b[119;5u")?;
  assert!(session.app().overlays.hints().is_none());
  assert_eq!(session.app().keys.awaiting(), Some("C-w"));
  send(&mut session, b"l")?;
  assert_ne!(session.app().id, source);
  assert_eq!(crate::harness::pane_id(&mut session, source).reader.cursor, left);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().id, source);
  send(&mut session, b"\x17W")?;
  assert_ne!(session.app().id, source);

  let app = session.app();
  hints(app);
  action(app, Action::Window(Window::Focus(Direction::Right)))?;
  session.settle()?;
  assert!(session.app().overlays.hints().is_some(), "edge focus does not blur");
  let edge = session.app().id;
  let cursor = session.app().reader.cursor;
  send(&mut session, b"\x17l")?;
  assert_eq!(session.app().id, edge);
  assert_eq!(session.app().reader.cursor, cursor);
  assert!(
    session.app().overlays.hints().is_none(),
    "input dismisses before no-op"
  );
  assert!(session.app().keys.awaiting().is_none());
  action(session.app(), Action::Window(Window::Next))?;
  session.settle()?;
  assert_eq!(session.app().id, source);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn outside_click_consumes_capture_across_panes() -> Result<()> {
  let mut session = session(&file("book.pdf")?, screen())?;
  send(&mut session, b"\x17v")?;
  let source = session.app().id;
  let tab = session.active;
  let viewport = session.app().reader.scroll;
  send(&mut session, b"/partial")?;
  assert_eq!(session.app().input_owner(), InputOwner::Prompt);
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_eq!((session.active, session.app().id), (tab, source));
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert!(session.app().search.prompt_origin().is_none());
  assert_eq!(session.app().search.query().text, "");
  assert_eq!(session.app().reader.scroll, viewport);
  assert!(session.app().reader.selection.is_none());
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_ne!(session.app().id, source);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().id, source);
  keys(session.app(), b"v")?;
  let selection = session.app().reader.selection;
  assert!(selection.is_some());
  hints(session.app());
  action(session.app(), Action::Command)?;
  send(&mut session, b"\x1b[<0;90;10M")?;
  assert_eq!(session.app().id, source);
  assert!(session.app().overlays.prompt().is_none());
  assert!(
    session.app().overlays.hints().is_some(),
    "only the top transient closes"
  );
  assert_eq!(session.app().reader.selection, selection);
  send(&mut session, b"\x1b[<32;90;20M\x1b[<0;90;20m")?;
  assert_eq!(session.app().id, source);
  assert_eq!(session.app().reader.selection, selection);
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_eq!(session.app().id, source, "hints consume the next click");
  send(&mut session, b"\x1b[<0;90;10M\x1b[<0;90;10m")?;
  assert_ne!(session.app().id, source);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn prompt_wheel_and_passive_motion_keep_owner() -> Result<()> {
  let mut session = session(&file("book.pdf")?, screen())?;
  send(&mut session, b"\x17v")?;
  let source = session.app().id;
  hints(session.app());
  send(&mut session, b"\x1b[<35;90;10M")?;
  assert!(session.app().overlays.hints().is_some());
  assert_eq!(session.app().id, source);
  action(session.app(), Action::Command)?;
  let before: Vec<_> = session
    .tab()
    .panes
    .iter()
    .map(|app| (app.reader.scroll, app.reader.animation().active()))
    .collect();
  send(&mut session, b"\x1b[<65;90;10M")?;
  let after: Vec<_> = session
    .tab()
    .panes
    .iter()
    .map(|app| (app.reader.scroll, app.reader.animation().active()))
    .collect();
  assert_eq!(before, after);
  assert_eq!(session.app().input_owner(), InputOwner::Prompt);
  send(&mut session, b"\t\x1b[Z")?;
  assert!(session.app().overlays.prompt().is_some());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn overview_and_text_use_local_owner() -> Result<()> {
  let mut app = crate::harness::solo("book.pdf", false, screen())?;
  crate::common::outline_loaded(&mut app)?;
  keys(&mut app, b"gogp")?;
  assert_eq!(app.input_owner(), InputOwner::Overview);
  let selected = app.overlays.overview().unwrap().selected();
  keys(&mut app, b"\tj")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  assert_eq!(app.overlays.overview().unwrap().selected(), selected);
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), 1);
  keys(&mut app, b"\tj")?;
  assert_eq!(app.input_owner(), InputOwner::Overview);
  assert!(app.overlays.overview().unwrap().selected() > selected);
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), 1);
  let mut status = Vec::new();
  app.status(&mut status, app.screen.rows - 1, None)?;
  assert!(String::from_utf8_lossy(&status).contains("PAGES"));
  keys(&mut app, b"\x1b[27u")?;
  keys(&mut app, b"vjl")?;
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
  let mut app = crate::harness::solo("book.pdf", false, screen())?;
  app.command("20")?;
  app.command("30")?;
  keys(&mut app, b"\x1b[111;5u")?;
  assert_eq!(app.reader.cursor.page, PageId(19));
  keys(&mut app, b"\t")?;
  assert_eq!(app.reader.cursor.page, PageId(19));
  keys(&mut app, b"\x1b[105;5u")?;
  assert_eq!(app.reader.cursor.page, PageId(29));
  keys(&mut app, b"go")?;
  let cursor = app.reader.cursor;
  keys(&mut app, b":te\t")?;
  assert_eq!(app.input_owner(), InputOwner::Prompt);
  assert_eq!(app.overlays.prompt().unwrap().text(), "text");
  assert_eq!(app.reader.cursor, cursor);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn peek_prompt_retains_destination_and_owner() -> Result<()> {
  let mut app = super::preview::open(true)?;
  app.preview_link()?;
  let target = format!("{:?}", app.overlays.preview().unwrap().target());
  keys(&mut app, b":")?;
  assert_eq!(app.input_owner(), InputOwner::Prompt);
  assert_eq!(format!("{:?}", app.overlays.preview().unwrap().target()), target);
  keys(&mut app, b"\t\x1b[Z")?;
  assert!(app.overlays.preview().is_some() && app.overlays.prompt().is_some());
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Peek);
  assert_eq!(format!("{:?}", app.overlays.preview().unwrap().target()), target);
  keys(&mut app, b"\x1b[119;5u")?;
  assert!(app.overlays.preview().is_none());
  assert_eq!(app.keys.awaiting(), Some("C-w"));
  keys(&mut app, b"w")?;
  assert!(app.keys.awaiting().is_none());
  assert!(matches!(
    app.request,
    Some(vpdf::app::Request::Window { command: Window::Next, .. })
  ));
  app.request = None;
  app.open_panel(PanelKind::Version)?;
  hints(&mut app);
  keys(&mut app, b"\t")?;
  assert_eq!(app.input_owner(), InputOwner::Content);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn middle_click_preserves_marks() -> Result<()> {
  let mut session = session(&file("paper.pdf")?, screen())?;
  let source = session.app().id;
  session.app().show_panel(
    PanelKind::Recent,
    vec![row(Destination::File(file("book.pdf")?))],
  )?;
  session.draw_focused(&mut Vec::new())?;
  send(&mut session, b"\x1b[<1;2;2M\x1b[<1;2;2m")?;
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
  let mut session = session(&file("book.pdf")?, screen())?;
  let app = session.app();
  crate::common::outline_loaded(app)?;
  keys(app, b"go60j")?;
  app.panel.as_mut().unwrap().view.finish_search("chapter");
  let panel = app.panel.as_ref().unwrap();
  let state =
    (panel.view.selected(), panel.view.top(), panel.view.query().to_owned());
  keys(app, b"\t")?;
  session.draw_focused(&mut Vec::new())?;
  let app = session.app();
  keys(app, b"\x1b[Z")?;
  let panel = app.panel.as_ref().unwrap();
  assert_eq!(
    (panel.view.selected(), panel.view.top(), panel.view.query().to_owned()),
    state
  );
  assert!(panel.view.focused());
  send(&mut session, b"\t")?;
  let id = session.app().id;
  let viewport = session.app().reader.scroll;
  send(&mut session, b"\x1b[<64;2;5M")?;
  let app = session.app();
  let panel = app.panel.as_ref().unwrap();
  let top = panel.view.top();
  assert!(top < state.1);
  assert_eq!(panel.view.selected(), state.0);
  assert_eq!(app.input_owner(), InputOwner::Content);
  session.draw_focused(&mut Vec::new())?;
  session.draw_focused(&mut Vec::new())?;
  let app = session.app();
  assert_eq!(app.panel.as_ref().unwrap().view.top(), top);
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), state.0);
  assert_eq!(app.id, id);
  assert_eq!(app.reader.scroll, viewport);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn narrow_recent_press_restores_cover_geometry() -> Result<()> {
  use vpdf_model::style::Role;
  use vpdf_terminal::style::sgr;
  let mut session = session(&file("paper.pdf")?, screen())?;
  let source = session.app().id;
  session.app().command("recent")?;
  session.settle()?;
  for click in [
    b"\x1b[<0;2;2M\x1b[<0;2;2m".as_slice(),
    b"\x1b[<0;2;10M\x1b[<0;2;10m".as_slice(),
  ] {
    send(&mut session, b"\t")?;
    assert!(!session.app().covered());
    session.draw_focused(&mut Vec::new())?;
    send(&mut session, click)?;
    assert_eq!(session.app().id, source);
    let app = session.app();
    assert!(app.covered() && app.panel_focused());
    assert_eq!(app.left, app.screen.columns);
    assert_eq!(app.layout.screen().columns, app.screen.columns);
    assert!(app.panel.as_ref().unwrap().view.is_recent_view());
    let mut out = Vec::new();
    session.draw_focused(&mut out)?;
    let hint = format!("{}Enter{} open", sgr(Role::Key), sgr(Role::Bar));
    assert!(String::from_utf8_lossy(&out).contains(&hint));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn placement_reuses_durable_views() -> Result<()> {
  use vpdf_reader::reader::{Mode, SelectionKind};
  let paper = file("paper.pdf")?;
  let book = file("book.pdf")?;
  let mut session = session(&paper, screen())?;
  send(&mut session, b":2\rvw")?;
  let target = session.app().id;
  session.app().start_search("attention", false, false)?;
  session.app().show_panel(
    PanelKind::Results,
    (0..80).map(|_| row(Destination::None)).collect(),
  )?;
  let app = session.app();
  let panel = app.panel.as_mut().unwrap();
  panel.view.choose(52);
  panel.view.navigate(Input::plain(Action::Align(1.0)), 15);
  panel.view.finish_search("committed");
  let anchor = app.reader.selection.unwrap().anchor;
  let cursor = app.reader.cursor;
  let scroll = app.reader.scroll;
  let generation = app.search.generation();
  session.open(&book, true, screen())?;
  let source = session.app().id;
  for command in ["e", "tabe", "split", "vsplit"] {
    send(&mut session, format!(":{command} {}\r", paper.display()).as_bytes())?;
    assert_eq!(session.app().id, target, "{command}");
    assert_eq!(session.tabs.len(), 2, "{command}");
    assert!(session.tabs.iter().all(|tab| tab.panes.len() == 1));
    let app = session.app();
    assert_eq!(app.reader.selection.unwrap().anchor, anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(SelectionKind::Character));
    assert_eq!(app.reader.cursor, cursor);
    assert!((app.reader.scroll - scroll).abs() < 0.1);
    assert_eq!(app.input_owner(), InputOwner::Panel);
    let panel = app.panel.as_ref().unwrap();
    assert_eq!((panel.view.selected(), panel.view.top()), (52, 40));
    assert_eq!(panel.view.query(), "committed");
    assert_eq!(app.search.query().text, "attention");
    assert_eq!(app.search.generation(), generation);
    assert!(matches!(app.search.phase(), vpdf_ui::find::Phase::Running { .. }));
    session.open(&book, true, screen())?;
    assert_eq!(session.app().id, source);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn current_view_requests_keep_transients() -> Result<()> {
  use vpdf::app::Request;
  use vpdf_reader::reader::Operator;
  use vpdf_ui::split::Axis;
  let path = file("paper.pdf")?;
  let mut session = session(&path, screen())?;
  send(&mut session, b"vw:")?;
  hints(session.app());
  session.app().reader.seed_reader(|reader| {
    reader.operator = Some(Operator { count: 1, register: '"' });
  });
  let id = session.app().id;
  let cursor = session.app().reader.cursor;
  for request in [
    Request::Open { path: path.clone(), tab: false },
    Request::Open { path: path.clone(), tab: true },
    Request::Window {
      command: Window::Split(Axis::Rows),
      count: 1,
      explicit: false,
      file: Some(path.clone().into()),
    },
    Request::Window {
      command: Window::Split(Axis::Columns),
      count: 1,
      explicit: false,
      file: Some(path.into()),
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
    assert!(app.overlays.prompt().is_some() && app.overlays.hints().is_some());
  }
  action(session.app(), Action::LocalFocus)?;
  assert!(
    session.app().overlays.prompt().is_some()
      && session.app().overlays.hints().is_some()
  );
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.tab().panes.len(), 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn new_views_blur_source_and_start_normal() -> Result<()> {
  for command in ["e", "tabe", "split", "vsplit"] {
    let mut session = session(&file("book.pdf")?, screen())?;
    send(&mut session, b":30\rvwgo")?;
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
        old.overlays.prompt().is_none()
          && old.overlays.hints().is_none()
          && old.overlays.preview().is_none()
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
  let mut session = session(&file("book.pdf")?, screen())?;
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
  assert!(session.app().overlays.prompt().is_none());
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
  let mut session = session(&file("book.pdf")?, screen())?;
  send(&mut session, b":30\r\x17v")?;
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
  let old = crate::harness::pane_id(&mut session, left);
  assert_eq!(old.reader.cursor, cursor, "old capture no longer routes");
  assert!(old.reader.selection.is_some());
  send(&mut session, b"y")?;
  assert!(session.app().reader.operator.is_some());
  session.app().command(&format!("tabe {}", file("form.pdf")?.display()))?;
  session.settle()?;
  let old = crate::harness::pane_id(&mut session, right);
  assert!(old.reader.operator.is_none());
  send(&mut session, b"gT\x1b[<0;10;5M\x1b[<32;15;6M")?;
  assert_eq!(session.app().id, left);
  assert!(session.app().reader.selection.is_some(), "new press captures");
  send(&mut session, b"\x1b[<0;15;6m:30")?;
  session.app().command(&format!("tabe {}", file("ocr.pdf")?.display()))?;
  session.settle()?;
  let old = crate::harness::pane_id(&mut session, left);
  assert!(old.overlays.prompt().is_none(), "new tab closes source prompt");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn close_uses_spatial_peer_after_history() -> Result<()> {
  let mut session = session(&file("book.pdf")?, screen())?;
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
  let path = file("book.pdf")?;
  let mut session = session(&path, screen())?;
  send(&mut session, b":30\r\x17vvwgo")?;
  let current = session.app().id;
  session.app().start_search("chapter", false, false)?;
  let app = session.app();
  let anchor = app.reader.selection.unwrap().anchor;
  let cursor = app.reader.cursor;
  let viewport = app.reader.scroll;
  let generation = app.search.generation();
  let panel = app.panel.as_ref().unwrap();
  let rows =
    (panel.view.selected(), panel.view.top(), panel.view.query().to_owned());
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
    assert_eq!(
      (panel.view.selected(), panel.view.top(), panel.view.query().to_owned()),
      rows
    );
    assert_eq!(app.search.query().text, "chapter");
    assert_eq!(app.search.generation(), generation);
    assert!(matches!(app.search.phase(), vpdf_ui::find::Phase::Running { .. }));
    assert_eq!(session.tab().panes.len(), 2);
    assert_eq!(session.tabs.len(), 1);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_entry_drops_pending_operator() -> Result<()> {
  for command in ["go", ":recent\r", ":marks\r"] {
    let mut session = session(&file("book.pdf")?, screen())?;
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
    {
      let seed_value =
        Some(vpdf_reader::reader::Operator { count: 1, register: '"' });
      session.app().reader.seed_reader(|reader| reader.operator = seed_value);
    };
    send(&mut session, command.as_bytes())?;
    assert!(session.app().reader.operator.is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_unopened_placement_routes() -> Result<()> {
  for (command, tabs, panes) in [("e", 1, 1), ("tabe", 2, 1), ("split", 1, 2)] {
    let mut session = session(&file("book.pdf")?, screen())?;
    let source = session.app().id;
    let path = file("paper.pdf")?.canonicalize()?;
    send(&mut session, format!(":{command} {}\r", path.display()).as_bytes())?;
    assert_ne!(session.app().id, source, "{command}");
    assert_eq!(session.active, tabs - 1);
    assert_eq!((session.tabs.len(), session.tab().panes.len()), (tabs, panes));
    let app = session.app();
    assert_eq!(app.path, path);
    assert_eq!(app.input_owner(), InputOwner::Content);
    assert!(app.reader.selection.is_none());
    assert!(app.search_worker.is_none() && app.ocr.is_none());
  }
  Ok(())
}
