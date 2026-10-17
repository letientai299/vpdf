use super::focus::{screen, send};
use std::collections::VecDeque;
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::{App, InputOwner};
use vpdf::decode::{Event, Parser};
use vpdf::document::Target;
use vpdf::prompt::History;
use vpdf::reader::{Mode, SelectionKind};
use vpdf::session::Session;
use vpdf::text::PageId;

fn key(app: &mut App, bytes: &[u8]) -> Result<()> {
  let mut events = VecDeque::new();
  Parser::default().feed(bytes, &mut events);
  for event in events {
    let Event::Key(key) = event else { panic!("key bytes") };
    app.key(key, &mut History::default())?;
  }
  Ok(())
}

fn open(text: bool) -> Result<App> {
  let path = PathBuf::from(std::env::var("VPDF_CORPUS")?).join("paper.pdf");
  let mut app = App::with_screen(&path, "", text, screen())?;
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
  Ok(app)
}

fn target_page(target: &Target) -> PageId {
  match *target {
    Target::Page(page) | Target::Point { page, .. } => page,
    Target::Uri(_) => unreachable!(),
  }
}

fn drawn(app: &mut App) -> Result<String> {
  app.graphics.begin(&mut std::io::sink())?;
  app.draw_preview(&mut std::io::sink())?;
  app.graphics.end(&mut std::io::sink())?;
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  app.draw_preview(&mut out)?;
  app.graphics.end(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_scroll_clamps_without_moving_target() -> Result<()> {
  for text in [false, true] {
    for entry in [b"".as_slice(), b"i", b"iv"] {
      let mut app = open(text)?;
      key(&mut app, entry)?;
      key(&mut app, b"K")?;
      let target = format!("{:?}", app.peek.as_ref().unwrap().target);
      let page = target_page(&app.peek.as_ref().unwrap().target);
      let label = app.doc.pages[page.0].label.clone();
      let cursor = app.reader.cursor;
      let range = app.reader.selection.map(|s| (s.anchor, s.kind));
      let view = (app.reader.scroll, app.text_position(0, 0)?);
      key(&mut app, b"9999k")?;
      let first = drawn(&mut app)?;
      assert!(first.contains(&format!("╭ {label} ")));
      assert!(!first.contains('↑'));
      assert!(first.contains('↓'));
      key(&mut app, b"3j")?;
      let advanced = drawn(&mut app)?;
      assert!(advanced.contains('↑'));
      assert_ne!(advanced, first);
      key(&mut app, b"3k")?;
      assert_eq!(drawn(&mut app)?, first);
      key(&mut app, b"2\x1b[100;5u")?;
      assert!(drawn(&mut app)?.contains('↑'));
      key(&mut app, b"2\x1b[117;5u")?;
      assert_eq!(drawn(&mut app)?, first);
      key(&mut app, b"9999j")?;
      let last = drawn(&mut app)?;
      assert!(!last.contains('↓'));
      key(&mut app, b"9999j")?;
      assert_eq!(drawn(&mut app)?, last);
      assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
      assert_eq!(app.reader.cursor, cursor);
      assert_eq!(app.reader.selection.map(|s| (s.anchor, s.kind)), range);
      assert_eq!((app.reader.scroll, app.text_position(0, 0)?), view);
      key(&mut app, b"\r")?;
      assert!(app.peek.is_none());
      let destination = target_page(&app.link_target(cursor)?.unwrap());
      assert_eq!(app.reader.cursor.page, destination);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_prefix_prompt_and_escape_keep_range() -> Result<()> {
  for text in [false, true] {
    let mut app = open(text)?;
    key(&mut app, b"ivK")?;
    let cursor = app.reader.cursor;
    let anchor = app.reader.selection.unwrap().anchor;
    let target = format!("{:?}", app.peek.as_ref().unwrap().target);
    key(&mut app, b"g")?;
    assert_eq!(app.keys.awaiting(), Some("g"));
    assert_eq!(app.input_owner(), InputOwner::Peek);
    key(&mut app, b"\x1b[27u")?;
    assert!(app.peek.is_some() && app.keys.awaiting().is_none());
    key(&mut app, b"gglwi")?;
    assert_eq!(app.reader.cursor, cursor);
    for prompt in [b":\x1b[27u".as_slice(), b":\r", b":nohlsearch\r"] {
      key(&mut app, prompt)?;
      assert_eq!(app.input_owner(), InputOwner::Peek);
      assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
      assert_eq!(app.reader.cursor, cursor);
      assert_eq!(app.reader.selection.unwrap().anchor, anchor);
    }
    assert!(key(&mut app, b":unknown\r").is_err());
    assert_eq!(app.input_owner(), InputOwner::Peek);
    assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
    key(&mut app, b"\x1b[27u")?;
    assert!(app.peek.is_none());
    assert_eq!(app.reader.mode(), Mode::Visual(SelectionKind::Character));
    key(&mut app, b"\x1b[27u")?;
    assert_eq!(app.reader.mode(), Mode::Cursor);
    key(&mut app, b"\x1b[27u")?;
    assert_eq!(app.reader.mode(), Mode::Normal);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_pane_chord_dispatches_once() -> Result<()> {
  let app = open(false)?;
  let cursor = app.reader.cursor;
  let mut session = Session::new(None);
  session.open(&app.path, true, screen())?;
  session.app().go_to(cursor, true)?;
  send(&mut session, b"K3g")?;
  assert_eq!(session.app().input_owner(), InputOwner::Peek);
  send(&mut session, b"t")?;
  assert!(session.app().peek.is_none());
  session.app().go_to(cursor, true)?;
  send(&mut session, b"K\x1b[119;5u")?;
  assert!(session.app().peek.is_none());
  assert_eq!(session.app().keys.awaiting(), Some("C-w"));
  send(&mut session, b"v")?;
  assert_eq!(session.tabs[session.active].panes.len(), 2);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_border_stays_inside_resized_pane() -> Result<()> {
  use unicode_width::UnicodeWidthStr;
  use vpdf::keys::PanelKind;
  use vpdf::layout::Crop;
  for text in [false, true] {
    let mut app = open(text)?;
    let cursor = app.reader.cursor;
    app.open_panel(PanelKind::Help)?;
    app.panel.as_mut().unwrap().focused = false;
    app.go_to(cursor, true)?;
    key(&mut app, b"K")?;
    let target = format!("{:?}", app.peek.as_ref().unwrap().target);
    app.at = vpdf::terminal::At { column: 7, row: 3 };
    app.screen.origin = (7, 3);
    for rotation in [0, 90, 180, 270] {
      app.screen.columns = 78;
      app.screen.rows = 19;
      app.layout.rotation = rotation;
      app.layout.crop = Crop([50, 50, 50, 50]);
      app.layout.zoom = 1.5;
      app.relayout()?;
      let out = drawn(&mut app)?;
      assert_eq!(out.matches('╭').count(), 1);
      assert_eq!(out.matches('╰').count(), 1);
      let label = out.split_once('╭').unwrap().1.split('╮').next().unwrap();
      assert!(label.width() + 2 <= usize::from(app.layout.screen.columns));
      for part in out.split("\x1b[") {
        let Some((position, _)) = part.split_once('H') else { continue };
        let Some((row, column)) = position.split_once(';') else { continue };
        let (Ok(row), Ok(column)) = (row.parse::<u16>(), column.parse::<u16>())
        else {
          continue;
        };
        assert!(row > app.at.row && row < app.at.row + app.screen.rows);
        assert!(column > app.at.column + app.left);
        assert!(column <= app.at.column + app.screen.columns);
      }
      assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_reload_discards_target() -> Result<()> {
  let corpus = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("reload.pdf");
  for text in [false, true] {
    let mut app = open(text)?;
    std::fs::copy(corpus.join("paper.pdf"), &path)?;
    app.path.clone_from(&path);
    key(&mut app, b"ivK")?;
    let target = format!("{:?}", app.peek.as_ref().unwrap().target);
    let cursor = app.reader.cursor;
    let range = app.reader.selection.unwrap();
    std::fs::remove_file(&path)?;
    assert!(app.command("reload").is_err());
    assert_eq!(format!("{:?}", app.peek.as_ref().unwrap().target), target);
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, range.anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(range.kind));
    std::fs::copy(corpus.join("form.pdf"), &path)?;
    let shorter = vpdf::document::Document::open(&path, "")?.pages.len();
    assert!(shorter <= target_page(&app.peek.as_ref().unwrap().target).0);
    app.command("reload")?;
    assert!(app.peek.is_none());
    assert_eq!(app.doc.pages.len(), shorter);
    assert_eq!(app.reader.cursor.page, PageId(cursor.page.0.min(shorter - 1)));
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert_eq!(drawn(&mut app)?, "");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_invalid_keeps_reader() -> Result<()> {
  let corpus = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for text in [false, true] {
    let mut app = open(text)?;
    key(&mut app, b"ivK")?;
    let cursor = app.reader.cursor;
    let range = app.reader.selection.unwrap();
    app.doc = vpdf::document::Document::open(&corpus.join("form.pdf"), "")?;
    app.layout.reflow(&app.doc.pages);
    assert_eq!(drawn(&mut app)?, "");
    assert!(app.peek.is_none());
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, range.anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(range.kind));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_label_keeps_arrow() -> Result<()> {
  use unicode_width::UnicodeWidthStr;
  for text in [false, true] {
    let mut app = open(text)?;
    key(&mut app, b"K9999k3j")?;
    let page = target_page(&app.peek.as_ref().unwrap().target);
    app.doc.pages[page.0].label =
      format!("界{}", app.doc.pages[page.0].label).repeat(200);
    let out = drawn(&mut app)?;
    let border = out.split_once('╭').unwrap().1.split('╮').next().unwrap();
    assert!(border.contains('↑'));
    assert!(border.width() + 2 <= usize::from(app.layout.screen.columns));
  }
  Ok(())
}
