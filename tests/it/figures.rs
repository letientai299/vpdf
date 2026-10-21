use std::path::PathBuf;
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_model::text::PageId;

#[test]
fn raw_figure_bindings() {
  use std::collections::VecDeque;
  use vpdf_terminal::decode::{Event, Parser};
  use vpdf_ui::keys::{Action, Grammar, Keys};
  for (bytes, expected) in [
    (&b"]i"[..], Action::Figure(true)),
    (&b"[i"[..], Action::Figure(false)),
    (&b"zF"[..], Action::FigureFit),
    (&b"gF"[..], Action::Region),
  ] {
    let mut events = VecDeque::new();
    Parser::default().feed(bytes, &mut events);
    let mut keys = Keys::default();
    let mut actions = Vec::new();
    for event in events {
      let Event::Key(key) = event else { panic!("key bytes") };
      if let Some(input) = keys.feed(crate::input::key(key), Grammar::Normal) {
        actions.push(input.action);
      }
    }
    assert_eq!(actions, [expected], "{bytes:?}");
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_figure_navigation_fit() -> Result<()> {
  use vpdf_ui::keys::{Action, Input};
  use vpdf_ui::region::Region;
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = super::focus::screen();
  let mut app = Solo::open(&root.join("paper.pdf"), "", false, screen)?;
  app.action(Input::plain(Action::Cursor))?;
  app.move_figure(true, 1)?;
  let first = app.overlays.region().unwrap().snapshot().clone();
  assert_ne!(first.page, PageId(0), "Title rule remained: {first:?}");
  app.move_figure(true, 1)?;
  let second = app.overlays.region().unwrap().snapshot().clone();
  assert!(
    second.page > first.page
      || second.bounds.y0 > first.bounds.y0
      || second.bounds.x0 > first.bounds.x0
  );
  app.move_figure(false, 1)?;
  let returned = app.overlays.region().unwrap().snapshot();
  assert_eq!(returned.page, first.page);
  assert_eq!(returned.bounds.y0, first.bounds.y0);
  assert_eq!(returned.bounds.x0, first.bounds.x0);
  app.fit_figure()?;
  let rect = app.layout.project(first.page, first.bounds, app.reader.scroll);
  assert!(rect.width() <= screen.width() + 1.0, "{rect:?}");
  assert!(rect.height() <= screen.height() + 1.0, "{rect:?}");
  assert!(
    rect.y1 <= screen.height() - f32::from(screen.cell_height) + 1.0,
    "Action bar overlaps figure: {rect:?}"
  );
  app.reader.visual(vpdf_reader::reader::SelectionKind::Character);
  let selection = app.reader.selection;
  let cursor = app.reader.cursor;
  let mode = app.reader.mode();
  app.action(Input::plain(Action::Region))?;
  assert_eq!(
    app.overlays.region().and_then(Region::action),
    Some(vpdf_ui::region::Operation::Image)
  );
  assert_eq!(app.reader.mode(), mode);
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    selection.map(|range| (range.anchor, range.kind))
  );
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_region_preserves_state() -> Result<()> {
  use super::common::keys;
  use super::modes::app;
  use vpdf_reader::reader::Mode;
  use vpdf_ui::input::InputOwner;
  use vpdf_ui::region::Operation;
  use vpdf_ui::region::Region;
  let mut app = app("paper.pdf", false)?;
  keys(&mut app, b"i]ivgF")?;
  let mode = app.reader.mode();
  assert!(matches!(mode, Mode::Visual(_)));
  let cursor = app.reader.cursor;
  let selection = app.reader.selection.map(|range| (range.anchor, range.kind));
  let scroll = app.reader.scroll;
  let capture = app.overlays.region().unwrap().snapshot().clone();
  keys(&mut app, b"hjkl")?;
  assert_eq!(
    app.overlays.region().and_then(Region::action),
    Some(Operation::Image)
  );
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(app.reader.scroll, scroll);
  keys(&mut app, b"g")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  assert_eq!(app.keys.pending(), "");
  keys(&mut app, b":\x1b[27u")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  assert_eq!(app.overlays.region().unwrap().snapshot().id, capture.id);
  keys(&mut app, b":\r")?;
  assert_eq!(app.input_owner(), InputOwner::Region);
  assert!(keys(&mut app, b":unknown-region-command\r").is_err());
  assert_eq!(app.input_owner(), InputOwner::Region);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.reader.mode(), mode);
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(
    app.reader.selection.map(|range| (range.anchor, range.kind)),
    selection
  );
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  keys(&mut app, b"\x1b[27u")?;
  assert_eq!(app.reader.mode(), Mode::Normal);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn raw_region_pointer_transfer() -> Result<()> {
  use super::focus::{screen, send};
  use vpdf::session::Session;
  use vpdf_reader::reader::Mode;
  use vpdf_ui::input::InputOwner;
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen())?;
  send(&mut session, b"i]ivgF")?;
  let mode = session.app().reader.mode();
  let cursor = session.app().reader.cursor;
  let scroll = session.app().reader.scroll;
  send(&mut session, b"\x1b[<65;60;8M")?;
  assert_eq!(session.app().input_owner(), InputOwner::Region);
  assert_eq!(session.app().reader.scroll, scroll);
  send(&mut session, b"\x1b[<0;60;8M\x1b[<0;60;8m")?;
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert_eq!(session.app().reader.mode(), mode);
  assert_eq!(session.app().reader.cursor, cursor);
  send(&mut session, b"gF\x17")?;
  assert_eq!(session.app().input_owner(), InputOwner::Content);
  assert_eq!(session.app().keys.awaiting(), Some("C-w"));
  send(&mut session, b"v")?;
  assert_eq!(session.tab().panes.len(), 2);
  assert_eq!(session.app().reader.mode(), Mode::Normal);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().reader.mode(), mode);
  assert_eq!(session.app().reader.cursor, cursor);
  assert!(session.app().overlays.region().is_none());
  Ok(())
}
