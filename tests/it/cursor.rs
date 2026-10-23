use super::common::keys;
use super::focus::{screen, send};
use super::modes::{app, settle};
use vpdf::Result;
use vpdf::session::Session;
use vpdf_model::document::Target;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::{Mode, SelectionKind};
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::{Destination, Row};

const ESC: &[u8] = b"\x1b[27u";

fn short_session(text: bool) -> Result<(tempfile::TempDir, Session)> {
  let (directory, mut session) = crate::mouse::session(text)?;
  session.resize(vpdf_ui::layout::Screen {
    rows: if text { 4 } else { 8 },
    columns: 60,
    ..screen()
  })?;
  Ok((directory, session))
}

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
    assert!(
      app.keys.awaiting().is_none()
        && app.clue_wait(std::time::Instant::now()).is_none()
    );
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
    assert!(app.reader.take_yank().unwrap().text.ends_with('\n'));
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
    let word = app.reader.take_yank().unwrap().text;
    assert!(!word.is_empty() && !word.contains(char::is_whitespace));
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert_eq!(app.reader.cursor, positioned);
    assert_eq!(app.reader.scroll, view);
    keys(&mut app, b"y2aw")?;
    assert!(app.reader.take_yank().unwrap().text.len() > word.len());
    keys(&mut app, b"viwy")?;
    assert_eq!(app.reader.take_yank().unwrap().text, word);
    assert_eq!(app.reader.mode(), Mode::Cursor);
    keys(&mut app, b"gv")?;
    assert!(app.reader.selection.is_some());
    keys(&mut app, ESC)?;
    keys(&mut app, ESC)?;
    keys(&mut app, b"yiw")?;
    assert_eq!(app.reader.mode(), Mode::Normal);
    assert!(app.reader.take_yank().is_some());
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
  assert_eq!(app.overlays.prompt().unwrap().text(), "i");
  keys(&mut app, ESC)?;
  assert_eq!(app.reader.mode(), Mode::Cursor);
  app.open_panel(PanelKind::Version)?;
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
    assert_ne!(app.search.query().text, "");
    assert_eq!(app.search.origin(), positioned);
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
fn cursor_scroll_and_history_use_visible_position() -> Result<()> {
  for text in [false, true] {
    let mut app = app("paper.pdf", text)?;
    app.command("2")?;
    keys(&mut app, b"i2l")?;
    let positioned = app.reader.cursor;
    let view = app.reader.scroll;
    keys(&mut app, b"\x04\x05")?;
    settle(&mut app);
    assert_ne!(app.reader.cursor, positioned);
    let positioned = app.reader.cursor;
    assert_eq!(app.reader.mode(), Mode::Cursor);
    if !text {
      assert_ne!(app.reader.scroll, view);
    }
    keys(&mut app, b"zt")?;
    assert_eq!(app.reader.cursor, positioned);
    keys(&mut app, b"4G")?;
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
  app.overlays.open_hints(super::common::hints(
    400,
    (1, 1),
    &Target::Page(PageId(1)),
  ));
  keys(&mut app, b"i")?;
  assert_eq!(app.overlays.hints().unwrap().typed(), "i");
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
  let paper = app("paper.pdf", false)?.app.path;
  let book = app("book.pdf", false)?.app.path;
  let mut session = crate::harness::session(&paper, screen())?;
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
  assert_eq!(session.app().id, pane);
  assert_eq!(session.app().reader.mode(), Mode::Cursor);
  assert_eq!(session.app().reader.cursor, cursor);
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
    app.command("cursorline off")?;
    let hit = Position { glyph: 5, ..app.reading_position()? };
    let hit = vpdf_search::search::Hit {
      first: hit,
      last: hit,
      excerpt: String::new(),
    };
    app.search.show(vpdf_search::query::Query::default(), vec![hit]);
    keys(&mut app, b"i")?;
    let mut out = Vec::new();
    app.status(&mut out, app.screen.rows - 1, None)?;
    assert!(String::from_utf8_lossy(&out).contains(" CURSOR "));
    out.clear();
    if text {
      vpdf::presentation::try_draw(&mut out, None, |s| app.scene_text(s))?;
      let frame = String::from_utf8_lossy(&out);
      assert!(frame.contains("\x1b[0;7;34m"));
      assert!(!frame.contains("\x1b[0;100m"));
      assert!(frame.contains("\x1b[0;7;33m"));
    } else {
      app.graphics.begin(&mut out)?;
      {
        let (app, graphics) = app.parts();
        let screen = app.layout.screen();
        vpdf::presentation::try_draw(&mut out, Some(graphics), |s| {
          app.scene_marks(s, screen)
        })?;
      }
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
    assert!(app.reader.cursor < heading);
    assert_eq!(app.outline.current(), Some(0));
    let section = app.outline.current().unwrap();
    let Target::Point { page, x, y } = app.doc.outline[section].target else {
      panic!("Expected heading point");
    };
    let heading = app.doc.text(page)?.nearest(page, x, y);
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
    let paper = app("paper.pdf", false)?.app.path;
    let book = app("book.pdf", false)?.app.path;
    let mut session = crate::harness::session(&paper, screen())?;
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
    let target = crate::harness::pane_id(&mut session, pane);
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
    assert_eq!(app.overlays.overview().unwrap().selected(), 2);
  }
  keys(&mut app, b"999G")?;
  assert_eq!(
    app.overlays.overview().unwrap().selected(),
    app.doc.pages.len() - 1
  );
  keys(&mut app, b"gg")?;
  assert_eq!(app.overlays.overview().unwrap().selected(), 0);
  Ok(())
}

#[test]
fn count_cancel_preserves_cursor() -> Result<()> {
  for text in [false, true] {
    for cancel in [b"\x1b[99;5u".as_slice(), b"\x1b[99;69u", b"\x1b[99;133u"] {
      let (_directory, mut session) = crate::mouse::session(text)?;
      let app = session.app();
      keys(app, b"i3")?;
      assert_eq!(app.reader.mode(), Mode::Cursor);
      keys(app, cancel)?;
      assert_eq!(app.reader.mode(), Mode::Cursor, "{cancel:?}");
      assert_eq!(app.keys.pending(), "");
      keys(app, cancel)?;
      assert_eq!(app.reader.mode(), Mode::Normal);
    }
  }
  Ok(())
}

#[test]
fn cancel_layer_preserves_visual() -> Result<()> {
  for text in [false, true] {
    for layer in [b"/results".as_slice(), b"gp", b"f"] {
      if text && layer == b"gp" {
        continue;
      }
      for cancel in [ESC, b"\x03"] {
        let (_directory, mut session) = crate::mouse::session(text)?;
        let app = session.app();
        keys(app, b"ivw")?;
        let range = (app.reader.cursor, app.reader.selection);
        keys(app, layer)?;
        assert_ne!(app.input_owner(), InputOwner::Content);
        keys(app, cancel)?;
        assert_eq!(app.input_owner(), InputOwner::Content);
        assert_eq!((app.reader.cursor, app.reader.selection), range);
        assert!(!app.quit);
        keys(app, cancel)?;
        assert!(app.reader.selection.is_none());
      }
    }
  }
  Ok(())
}

#[test]
fn rejected_keys_cancel_yanks() -> Result<()> {
  for rejected in [b"x".as_slice(), b"+", b"gZ"] {
    let (_directory, mut session) = crate::mouse::session(false)?;
    let app = session.app();
    app.reader.store('a', "keep".into())?;
    app.reader.take_yank();
    keys(app, b"\"ay")?;
    assert!(app.reader.operator.is_some());
    keys(app, rejected)?;
    assert!(app.reader.operator.is_none());
    keys(app, b"j")?;
    assert_eq!(app.reader.register('a').map(|value| &**value), Some("keep"));
    assert!(app.reader.pending_yank.is_none());
  }
  for valid in [b"\"a2y3w".as_slice(), b"\"ayiw", b"\"ayp"] {
    let (_directory, mut session) = crate::mouse::session(false)?;
    let app = session.app();
    keys(app, valid)?;
    assert!(app.reader.operator.is_none());
    assert!(app.reader.register('a').is_some());
  }
  Ok(())
}

#[test]
fn search_yank_finishes_at_original_range() -> Result<()> {
  for preview in [false, true] {
    for (start, search, expected) in [
      (b"i0".as_slice(), b"\"ay/the".as_slice(), "See "),
      (b"i0$".as_slice(), b"\"ay?the".as_slice(), "the result"),
      (b"i0".as_slice(), b"\"a2y/the".as_slice(), "See the results\nVisit "),
    ] {
      let (_directory, mut session) = crate::mouse::session(false)?;
      let app = session.app();
      keys(app, start)?;
      let origin = app.reader.cursor;
      keys(app, search)?;
      if preview {
        crate::common::search_loaded(app)?;
      }
      keys(app, b"\r")?;
      crate::common::search_loaded(app)?;
      assert_eq!(app.reader.cursor, origin);
      assert!(app.reader.operator.is_none());
      assert_eq!(&**app.reader.register('a').unwrap(), expected);
    }
  }
  Ok(())
}

#[test]
fn failed_search_yanks_preserve_register() -> Result<()> {
  for query in
    [b"/absent\r".as_slice(), b"/\\v[\r", b"/the\x1b[27u", b"/the\rj"]
  {
    let (_directory, mut session) = crate::mouse::session(false)?;
    let app = session.app();
    app.reader.store('a', "keep".into())?;
    app.reader.take_yank();
    keys(app, b"i\"ay")?;
    let _ = keys(app, query);
    if !matches!(app.search.phase(), vpdf_ui::find::Phase::Idle) {
      crate::common::search_loaded(app)?;
    }
    assert!(app.reader.operator.is_none());
    assert_eq!(&**app.reader.register('a').unwrap(), "keep");
    assert!(app.reader.pending_yank.is_none());
  }
  Ok(())
}

#[test]
fn scroll_then_motion_uses_visible_cursor() -> Result<()> {
  for text in [false, true] {
    for scroll in [b"\x04".as_slice(), b"\x06", b"8\x05"] {
      let (_directory, mut session) = short_session(text)?;
      let app = session.app();
      keys(app, b"i")?;
      let origin = app.reader.cursor;
      keys(app, scroll)?;
      let moved = app.reader.cursor;
      assert!(moved > origin, "text={text}, scroll={scroll:?}");
      // No animation tick between the scroll and the next motion.
      let target = app.reader.animation().target();
      let cell = app.text().cell(moved)?;
      keys(app, b"l")?;
      assert_eq!(app.reader.cursor.line, moved.line);
      assert_eq!(app.reader.cursor.page, moved.page);
      if text {
        assert_eq!(app.text().cell(moved)?, cell);
      }
      if let Some(target) = target {
        assert!(
          (app.reader.scroll - target).abs()
            <= f32::from(app.screen.cell_height)
        );
      }
      keys(app, b"j")?;
      assert!(app.reader.cursor >= moved);
    }
  }
  Ok(())
}

#[test]
fn visual_keyboard_scroll_moves_only_endpoint() -> Result<()> {
  for text in [false, true] {
    let (_directory, mut session) = short_session(text)?;
    let app = session.app();
    keys(app, b"iv")?;
    let anchor = app.reader.selection;
    let origin = app.reader.cursor;
    keys(app, b"\x04")?;
    let endpoint = app.reader.cursor;
    assert!(endpoint > origin);
    assert_eq!(app.reader.selection, anchor);
    app.scroll(100.0)?;
    assert_eq!(app.reader.cursor, endpoint);
    assert_eq!(app.reader.selection, anchor);
    keys(app, b"j")?;
    assert!(app.reader.cursor >= endpoint);
    assert_eq!(app.reader.selection, anchor);
  }
  Ok(())
}

#[test]
fn cursor_wheel_clamps_without_changing_mode() -> Result<()> {
  for text in [false, true] {
    let (_directory, mut session) = short_session(text)?;
    let app = session.app();
    keys(app, b"i")?;
    let origin = app.reader.cursor;
    app.scroll(160.0)?;
    assert!(
      app.reader.cursor > origin,
      "text={text}, origin={origin:?}, cursor={:?}, target={:?}",
      app.reader.cursor,
      app.reader.animation().target()
    );
    let moved = app.reader.cursor;
    keys(app, b"l")?;
    assert_eq!(app.reader.cursor.page, moved.page);
    assert_eq!(app.reader.cursor.line, moved.line);
    assert_eq!(app.reader.mode(), Mode::Cursor);
    app.scroll(-160.0)?;
    assert!(app.reader.cursor <= moved);
  }
  Ok(())
}
