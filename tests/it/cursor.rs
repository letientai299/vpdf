use super::focus::{screen, send};
use super::modes::{app, keys, settle};
use vpdf::Result;
use vpdf::app::InputOwner;
use vpdf::document::Target;
use vpdf::hints::{Hint, Hints};
use vpdf::keys::PanelKind;
use vpdf::panel::{Destination, Row};
use vpdf::reader::{Mode, SelectionKind};
use vpdf::session::Session;
use vpdf::text::{PageId, Position};

const ESC: &[u8] = b"\x1b[27u";

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_entry_positions_visual_ranges() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    keys(&mut app, b"10j")?;
    settle(&mut app);
    let entry = app.reading_position()?;
    let view = app.reader.scroll;
    keys(&mut app, b"i")?;
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert_eq!(app.reader.cursor, entry);
    assert_eq!(app.reader.scroll, view);
    app.after_key();
    assert!(app.keys.awaiting().is_none() && app.clue_wait().is_none());
    keys(&mut app, b"ia2l")?;
    assert_eq!(app.reader.cursor.glyph, entry.glyph + 2);
    keys(&mut app, b"2hjkl")?;
    assert_eq!(app.reader.cursor.glyph, entry.glyph + 1);
    let positioned = app.reader.cursor;
    keys(&mut app, b"v2l")?;
    assert_eq!(app.reader.selection.unwrap().anchor, positioned);
    assert_eq!(app.reader.mode(), Mode::Visual(SelectionKind::Character));
    keys(&mut app, ESC)?;
    assert_eq!(app.reader.mode(), Mode::Cursor);
    keys(&mut app, b"Vy")?;
    assert!(app.reader.pending_yank.take().unwrap().text.ends_with('\n'));
    assert_eq!(app.reader.mode(), Mode::Cursor);
    keys(&mut app, ESC)?;
    assert_eq!(app.reader.mode(), Mode::Normal);
    keys(&mut app, b"v")?;
    keys(&mut app, ESC)?;
    assert_eq!(app.reader.mode(), Mode::Normal);
    keys(&mut app, b"Vyl")?;
    assert_eq!(app.reader.mode(), Mode::Normal);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_objects_require_visual_or_copy() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    app.command("2")?;
    keys(&mut app, b"i0w")?;
    let positioned = app.reader.cursor;
    let view = app.reader.scroll;
    keys(&mut app, b"yiw")?;
    let word = app.reader.pending_yank.take().unwrap().text;
    assert!(!word.is_empty() && !word.contains(char::is_whitespace));
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert_eq!(app.reader.cursor, positioned);
    assert_eq!(app.reader.scroll, view);
    keys(&mut app, b"y2aw")?;
    assert!(app.reader.pending_yank.take().unwrap().text.len() > word.len());
    keys(&mut app, b"viwy")?;
    assert_eq!(app.reader.pending_yank.take().unwrap().text, word);
    assert_eq!(app.reader.mode(), Mode::Cursor);
    keys(&mut app, b"gv")?;
    assert!(app.reader.selection.is_some());
    keys(&mut app, ESC)?;
    keys(&mut app, ESC)?;
    keys(&mut app, b"yiw")?;
    assert_eq!(app.reader.mode(), Mode::Normal);
    assert!(app.reader.pending_yank.take().is_some());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_escape_cancels_only_active_layer() -> Result<()> {
  let mut app = app("paper.pdf", false)?;
  keys(&mut app, b"i")?;
  for prefix in [b"3".as_slice(), b"g", b"\"", b"\"a", b"y", b"yi"] {
    keys(&mut app, prefix)?;
    keys(&mut app, ESC)?;
    assert_eq!(app.reader.mode(), Mode::Cursor, "{prefix:?}");
    assert!(app.keys.pending().is_empty() && app.reader.operator.is_none());
  }
  keys(&mut app, b"vg")?;
  keys(&mut app, ESC)?;
  assert!(app.reader.selection.is_some());
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  keys(&mut app, b":i")?;
  assert_eq!(app.prompt.as_ref().unwrap().text, "i");
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  app.open_panel(PanelKind::Help)?;
  keys(&mut app, b"ia")?;
  assert_eq!(app.input_owner(), InputOwner::Panel);
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  let mut out = Vec::new();
  app.draw(&mut out)?;
  keys(&mut app, b"\t")?;
  assert_eq!(app.input_owner(), InputOwner::Content);
  keys(&mut app, b"gp")?;
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Normal);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_targets_and_hidden_entry() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    app.command("2")?;
    let target = app.reader.cursor;
    keys(&mut app, b"i2l")?;
    let positioned = app.reader.cursor;
    assert_ne!(positioned, target);
    app.set_mark('i')?;
    assert_eq!(app.marks[&'i'], positioned);
    assert_eq!(app.navigation_position(), positioned);
    assert_eq!(app.reading_position()?, positioned);
    app.word_search()?;
    assert_ne!(app.search.query.text, "");
    assert_eq!(app.search.origin, positioned);
    keys(&mut app, b"/partial")?;
    keys(&mut app, ESC)?;
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert_eq!(app.reader.cursor, positioned);
    keys(&mut app, ESC)?;
    app.scroll(2000.0)?;
    settle(&mut app);
    let visible = app.reading_position()?;
    assert_ne!(visible, positioned);
    let view = app.reader.scroll;
    keys(&mut app, b"i")?;
    assert_eq!(app.reader.cursor, visible);
    assert_eq!(app.reader.scroll, view);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_scroll_and_history_keep_position() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    app.command("2")?;
    keys(&mut app, b"i2l")?;
    let positioned = app.reader.cursor;
    let view = app.reader.scroll;
    keys(&mut app, b"\x04\x05")?;
    settle(&mut app);
    assert_eq!(app.reader.cursor, positioned);
    assert_eq!(app.reader.mode(), Mode::Cursor);
    if !text {
      assert_ne!(app.reader.scroll, view);
    }
    keys(&mut app, b"zt")?;
    assert_eq!(app.reader.cursor, positioned);
    keys(&mut app, b"2 ")?;
    let destination = app.reader.cursor;
    assert_ne!(destination.page, positioned.page);
    keys(&mut app, b"\x0f")?;
    assert_eq!(app.reader.cursor, positioned);
    keys(&mut app, b"\x1b[105;5u")?;
    assert_eq!(app.reader.cursor, destination);
    assert_eq!(app.reader.mode(), Mode::Cursor);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_hint_labels_keep_owner() -> Result<()> {
  let mut app = app("paper.pdf", false)?;
  keys(&mut app, b"i")?;
  app.hints = Some(Hints {
    items: vec![Hint {
      label: "ii".into(),
      target: Target::Page(PageId(1)),
      cell: (1, 1),
      column: 0,
    }],
    typed: String::new(),
    copy: false,
    selected: None,
  });
  keys(&mut app, b"i")?;
  assert_eq!(app.hints.as_ref().unwrap().typed, "i");
  assert_eq!(app.input_owner(), InputOwner::Hints);
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  keys(&mut app, b"\x1b[105;5u")?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_focus_reuses_durable_mode() -> Result<()> {
  let paper = app("paper.pdf", false)?.path;
  let book = app("book.pdf", false)?.path;
  let mut session = Session::new(None);
  session.open(&paper, true, screen())?;
  send(&mut session, b"i2lgo\ti\t")?;
  assert!(session.app().panel_focused());
  assert_eq!(session.app().reader.mode(), Mode::Cursor);
  send(&mut session, b"\x1b[27u\t")?;
  let pane = session.app().id;
  let cursor = session.app().reader.cursor;
  session.open(&book, true, screen())?;
  assert_eq!(session.app().reader.mode(), Mode::Normal);
  for command in ["e", "tabe", "split", "vsplit"] {
    send(&mut session, format!(":{command} {}\r", paper.display()).as_bytes())?;
    assert_eq!(session.app().id, pane);
    assert_eq!(session.app().reader.mode(), Mode::Cursor);
    assert_eq!(session.app().reader.cursor, cursor);
    assert_eq!(session.tabs.len(), 2);
    session.open(&book, true, screen())?;
  }
  session.open(&paper, true, screen())?;
  send(&mut session, b":vsplit\r")?;
  assert_eq!(session.app().reader.mode(), Mode::Normal);
  send(&mut session, b"\x17p")?;
  assert_eq!(session.app().id, pane);
  assert_eq!(session.app().reader.mode(), Mode::Cursor);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_rendering_keeps_visual_band_separate() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    let hit = Position { glyph: 5, ..app.reading_position()? };
    app.search.hits.push(vpdf::search::Hit {
      first: hit,
      last: hit,
      excerpt: String::new(),
    });
    keys(&mut app, b"i")?;
    let mut out = Vec::new();
    app.status(&mut out, app.screen.rows - 1)?;
    assert!(String::from_utf8_lossy(&out).contains(" CURSOR "));
    out.clear();
    if text {
      app.text_canvas(&mut out)?;
      let frame = String::from_utf8_lossy(&out);
      assert!(frame.contains("\x1b[0;7;34m"));
      assert!(!frame.contains("\x1b[0;100m"));
      assert!(frame.contains("\x1b[0;7;33m"));
    } else {
      app.graphics.begin(&mut out)?;
      app.marks(app.layout.screen, &mut out)?;
      app.graphics.end(&mut out)?;
      let frame = String::from_utf8_lossy(&out);
      assert!(frame.contains("z=1,"));
      assert!(!frame.contains("z=-1,"));
    }
    keys(&mut app, b"v")?;
    out.clear();
    app.draw(&mut out)?;
    if text {
      assert!(String::from_utf8_lossy(&out).contains("\x1b[0;100m"));
    }
    keys(&mut app, ESC)?;
    keys(&mut app, ESC)?;
    assert_eq!(app.reader.mode(), Mode::Normal);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_sections_use_position_after_scrolling() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    super::common::outline_loaded(&mut app)?;
    let Target::Point { page, x, y } = app.doc.outline[1].target else {
      panic!("Expected heading point");
    };
    let heading = app.doc.text(page)?.nearest(page, x, y);
    app.go_to(Position { line: heading.line + 2, ..heading }, true)?;
    keys(&mut app, b"i")?;
    app.scroll(-500.0)?;
    settle(&mut app);
    app.track_outline();
    assert_eq!(app.current_section, Some(1));
    keys(&mut app, b"[[")?;
    assert_eq!(app.reader.cursor, heading);
    assert_eq!(app.reader.mode(), Mode::Cursor);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursor_panel_enter_returns_normal() -> Result<()> {
  for cross_document in [false, true] {
    let paper = app("paper.pdf", false)?.path;
    let book = app("book.pdf", false)?.path;
    let mut session = Session::new(None);
    session.open(&paper, true, screen())?;
    send(&mut session, b"i2l")?;
    let pane = session.app().id;
    let cursor = session.app().reader.cursor;
    let destination = if cross_document {
      session.open(&book, true, screen())?;
      Destination::File(paper)
    } else {
      Destination::Position(cursor)
    };
    let source = session.app().id;
    session.app().show_panel(
      PanelKind::Links,
      vec![Row::new("target".into(), String::new(), destination)],
    )?;
    send(&mut session, b"o")?;
    assert_eq!(session.app().id, source);
    assert_eq!(session.app().input_owner(), InputOwner::Panel);
    let target = session
      .tabs
      .iter()
      .flat_map(|tab| &tab.panes)
      .find(|app| app.id == pane)
      .unwrap();
    assert_eq!(target.reader.mode(), Mode::Cursor);
    assert_eq!(target.reader.cursor, cursor);
    send(&mut session, b"\r")?;
    assert_eq!(session.app().id, pane);
    assert_eq!(session.app().input_owner(), InputOwner::Content);
    assert_eq!(session.app().reader.mode(), Mode::Normal);
    assert_eq!(session.app().reader.cursor, cursor);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn control_c_keeps_reading_pane() -> Result<()> {
  for mode in [b"iV".as_slice(), b"v", b"i", b""] {
    let mut app = app("paper.pdf", false)?;
    if !mode.is_empty() {
      keys(&mut app, mode)?;
    }
    keys(&mut app, b"\x03")?;
    assert!(!app.quit);
    assert!(app.reader.selection.is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn counted_overview_boundaries() -> Result<()> {
  let mut app = app("paper.pdf", false)?;
  keys(&mut app, b"gp")?;
  for command in [b"3gg".as_slice(), b"3G"] {
    keys(&mut app, command)?;
    assert_eq!(app.overview.as_ref().unwrap().selected, 2);
  }
  keys(&mut app, b"999G")?;
  assert_eq!(app.overview.as_ref().unwrap().selected, app.doc.pages.len() - 1);
  keys(&mut app, b"gg")?;
  assert_eq!(app.overview.as_ref().unwrap().selected, 0);
  Ok(())
}
