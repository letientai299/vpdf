use super::common::keys;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::solo::Solo;
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_reader::reader::SelectionKind;
use vpdf_ui::keys::PanelKind;
use vpdf_ui::layout::Screen;

pub(super) fn app(file: &str, text: bool) -> Result<Solo> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  Solo::open(
    &root.join(file),
    "",
    text,
    Screen {
      columns: 100,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  )
}

pub(super) fn settle(app: &mut App) {
  for _ in 0..40 {
    if let Some(target) = app.reader.animation().target() {
      app.reader.animate_to(
        target,
        Instant::now().checked_sub(Duration::from_millis(16)).unwrap(),
      );
    }
    if !app.animate() {
      return;
    }
  }
  assert!(!app.reader.animation().active());
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn normal_keys_move_view_without_text_extraction() -> Result<()> {
  let mut app = app("paper.pdf", false)?;
  app.doc.trim();
  let cursor = app.reader.cursor;
  assert_eq!(app.doc.cache_bytes(), 0);
  keys(&mut app, b"10j")?;
  settle(&mut app);
  assert_eq!(app.reader.scroll, 160.0);
  keys(&mut app, b"\x1b[B\x1b[A2k")?;
  settle(&mut app);
  assert_eq!(app.reader.scroll, 128.0);
  keys(&mut app, b"wbe0^${}")?;
  assert_eq!(app.reader.scroll, 128.0);
  assert_eq!(app.reader.cursor, cursor);
  assert_eq!(app.doc.cache_bytes(), 0);
  keys(&mut app, b"9999k")?;
  settle(&mut app);
  assert_eq!(app.reader.scroll, 0.0);
  keys(&mut app, b"9999j")?;
  settle(&mut app);
  assert_eq!(app.reader.scroll, app.layout.clamp_scroll(f32::MAX));
  keys(&mut app, b"l\x1b[C")?;
  assert_eq!(app.layout.pan(), 0.0);
  keys(&mut app, b"+")?;
  let pan = app.layout.pan();
  keys(&mut app, b"3l\x1b[D")?;
  assert_eq!(app.layout.pan(), pan + 16.0);
  keys(&mut app, b"9999h")?;
  assert_eq!(app.layout.pan(), 0.0);
  assert_eq!(app.doc.cache_bytes(), 0);
  let scroll = app.reader.scroll;
  keys(&mut app, b"gv")?;
  assert_eq!(app.reader.scroll, scroll);
  let screen = app.layout.screen();
  {
    let (app, graphics) = app.parts();
    app.marks(graphics, screen, &mut Vec::new())?;
  }
  app.track_outline();
  app.status(&mut Vec::new(), app.screen.rows - 1)?;
  assert_eq!(app.doc.cache_bytes(), 0);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn visible_entry_selection_and_temporary_copy() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app("paper.pdf", text_mode)?;
    keys(&mut app, b"10j")?;
    settle(&mut app);
    let entry = app.reading_position()?;
    let view = (app.reader.scroll, app.text().position(0, 0)?);
    keys(&mut app, b"v")?;
    assert_eq!(app.reader.cursor, entry);
    assert_eq!(app.reader.selection.unwrap().kind, SelectionKind::Character);
    assert_eq!(app.reader.scroll, view.0);
    keys(&mut app, b"weo")?;
    let selection = app.reader.selection.unwrap();
    let cursor = app.reader.cursor;
    keys(&mut app, b"6\x05")?;
    settle(&mut app);
    app.draw(&mut Vec::new())?;
    assert!(app.reader.cursor >= cursor);
    let cursor = app.reader.cursor;
    assert_eq!(app.reader.selection.unwrap().anchor, selection.anchor);
    keys(&mut app, b"\x1b[27u")?;
    assert!(app.reader.selection.is_none());
    keys(&mut app, b"gv")?;
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, selection.anchor);
    keys(&mut app, b"y")?;
    assert!(app.reader.take_yank().is_some());
    assert!(app.reader.selection.is_none());
    let view = app.reader.scroll;
    keys(&mut app, b"yw")?;
    assert!(app.reader.take_yank().is_some());
    assert!(app.reader.operator.is_none());
    assert_eq!(app.reader.scroll, view);
    keys(&mut app, b"yy")?;
    assert!(app.reader.take_yank().unwrap().text.ends_with('\n'));
    keys(&mut app, b"V")?;
    assert_eq!(app.reader.selection.unwrap().kind, SelectionKind::Line);
    app.draw(&mut Vec::new())?;
    let viewport = (app.reader.scroll, app.text().position(0, 0)?);
    keys(&mut app, b"\x1b[27uj")?;
    settle(&mut app);
    app.draw(&mut Vec::new())?;
    assert!(app.reader.selection.is_none());
    assert_ne!((app.reader.scroll, app.text().position(0, 0)?), viewport);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn explicit_targets_expire_outside_view() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app("paper.pdf", text_mode)?;
    for bytes in [b"K".as_slice(), b"\r", b"*"] {
      keys(&mut app, bytes)?;
      assert!(app.reader.message.contains("Click text"));
      assert!(app.overlays.preview().is_none());
      assert_eq!(app.search.query().text, "");
    }
    let target = app.reading_position()?;
    app.go_to(target, true)?;
    keys(&mut app, b"v")?;
    assert_eq!(app.reader.cursor, target);
    keys(&mut app, b"we")?;
    let endpoint = app.reader.cursor;
    app.follow_cursor()?;
    app.word_search()?;
    assert_eq!(app.reader.cursor, endpoint);
    app.command("cancel")?;
    keys(&mut app, b"\x1b[27u50j")?;
    settle(&mut app);
    keys(&mut app, b"*")?;
    assert!(app.reader.message.contains("Click text"));
    let visible = app.reading_position()?;
    assert_ne!(visible, target);
    keys(&mut app, b"v")?;
    assert_eq!(app.reader.cursor, visible);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn layout_keeps_view_anchor_and_visual_endpoints() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app("paper.pdf", text_mode)?;
    app.command("5")?;
    keys(&mut app, b"12j")?;
    settle(&mut app);
    let cursor = app.reader.cursor;
    let top = app.text().position(0, 0)?;
    for bytes in [b"+".as_slice(), b"zr", b"zc", b"-"] {
      let page = app.layout.page_at(app.reader.scroll);
      let point = app
        .layout
        .unproject(
          page,
          (app.layout.screen().width() / 2.0, 0.0),
          app.reader.scroll,
        )
        .unwrap();
      keys(&mut app, bytes)?;
      let projected = app.layout.project(
        page,
        Bounds { x0: point.0, x1: point.0, y0: point.1, y1: point.1 },
        app.reader.scroll,
      );
      assert!(projected.y0.abs() < 1.0, "{bytes:?}: {projected:?}");
      assert_eq!(app.reader.cursor, cursor);
      if text_mode {
        assert_eq!(app.text().position(0, 0)?, top);
      }
    }
    let page = app.layout.page_at(app.reader.scroll);
    let point = app
      .layout
      .unproject(
        page,
        (app.layout.screen().width() / 2.0, 0.0),
        app.reader.scroll,
      )
      .unwrap();
    app.open_panel(PanelKind::Outline)?;
    app.close_panel()?;
    app.screen.columns = 80;
    app.screen.rows = 28;
    app.relayout()?;
    let after = app
      .layout
      .unproject(
        page,
        (app.layout.screen().width() / 2.0, 0.0),
        app.reader.scroll,
      )
      .unwrap();
    assert!((point.1 - after.1).abs() < 1.0);
    assert_eq!(app.reader.cursor, cursor);
    if text_mode {
      assert_eq!(app.text().position(0, 0)?, top);
    }
    keys(&mut app, b"vwo")?;
    let cursor = app.reader.cursor;
    let anchor = app.reader.selection.unwrap().anchor;
    app.scroll(400.0)?;
    settle(&mut app);
    app.draw(&mut Vec::new())?;
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, anchor);
    let mut split = self::app("paper.pdf", text_mode)?;
    split.mirror(&app)?;
    assert!(split.reader.selection.is_none());
    assert!(split.reader.target().is_none());
    assert_eq!(split.reader.scroll, app.reader.scroll);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn escape_preserves_focus_and_jobs_cancel_is_explicit() -> Result<()> {
  let mut app = app("paper.pdf", false)?;
  keys(&mut app, b"vwy")?;
  app.start_search("attention", false, false)?;
  app.start_ocr("eng")?;
  let generation = app.search.generation();
  app.open_panel(PanelKind::Outline)?;
  keys(&mut app, b"\x1b[27u\x1b[27u")?;
  assert!(app.panel_focused());
  assert_eq!(app.search.generation(), generation);
  assert!(!app.search.hidden());
  assert!(app.ocr.is_some());
  app.close_panel()?;
  keys(&mut app, b"vy\x1b[27u")?;
  assert!(app.reader.selection.is_none() && app.reader.operator.is_none());
  assert!(app.ocr.is_some());
  keys(&mut app, b":cancel\r")?;
  assert!(app.ocr.is_none() && !app.outline_pending());
  assert!(app.search.hidden());
  assert!(matches!(app.search.phase(), vpdf_ui::find::Phase::Idle));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn scan_selection_fails_without_ocr_or_view_motion() -> Result<()> {
  use mupdf::{ColorParams, Colorspace, DocumentWriter, Image, Matrix, Rect};
  let source = app("ocr.pdf", false)?;
  let bounds = source.doc.pages[0].bounds;
  let pixels = source.doc.native.load_page(0)?.to_pixmap(
    &Matrix::new_scale(1.0, 1.0),
    &Colorspace::device_rgb(),
    false,
    false,
  )?;
  let image = Image::from_pixmap(&pixels)?;
  let file = tempfile::Builder::new().suffix(".pdf").tempfile()?;
  let mut writer =
    DocumentWriter::new(file.path().to_str().unwrap(), "pdf", "")?;
  let device =
    writer.begin_page(Rect::new(bounds.x0, bounds.y0, bounds.x1, bounds.y1))?;
  device.fill_image(
    &image,
    &Matrix::new(
      bounds.width(),
      0.0,
      0.0,
      bounds.height(),
      bounds.x0,
      bounds.y0,
    ),
    1.0,
    ColorParams::default(),
  )?;
  device.end()?;
  drop(writer);
  for text_mode in [false, true] {
    let mut app = Solo::open(file.path(), "", text_mode, source.screen)?;
    assert!(app.doc.text(PageId(0))?.lines.is_empty());
    for bytes in [b"i".as_slice(), b"v", b"V", b"yw", b"yy"] {
      let scroll = app.reader.scroll;
      keys(&mut app, bytes)?;
      assert_eq!(app.reader.scroll, scroll);
      assert!(app.reader.selection.is_none() && app.reader.operator.is_none());
      assert!(app.ocr.is_none());
      assert_eq!(app.reader.mode(), vpdf_reader::reader::Mode::Normal);
      assert!(app.reader.message.contains("Selection unavailable"));
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn normal_marks_search_and_history_use_reading_area() -> Result<()> {
  let mut app = app("book.pdf", false)?;
  app.command("39")?;
  keys(&mut app, b"20j")?;
  settle(&mut app);
  let reading = app.reading_position()?;
  assert_ne!(reading, app.reader.cursor);
  keys(&mut app, b"ma")?;
  assert_eq!(app.marks[&'a'], reading);
  keys(&mut app, b"/")?;
  assert_eq!(app.search.prompt_origin().unwrap().cursor, reading);
  keys(&mut app, b"\x1b[27u")?;
  let scroll = app.reader.scroll;
  app.go_to(Position { page: PageId(60), ..Position::default() }, true)?;
  keys(&mut app, b"\x0f")?;
  assert_eq!(app.reader.scroll, scroll);
  assert_eq!(app.reader.cursor, reading);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn prompt_cancel_and_history_restore_viewport() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app("paper.pdf", text_mode)?;
    app.screen.columns = 32;
    app.relayout()?;
    app.command("4")?;
    keys(&mut app, b"+10j4l")?;
    settle(&mut app);
    app.draw(&mut Vec::new())?;
    let origin =
      (app.reader.scroll, app.layout.pan(), app.text().position(0, 0)?);
    let target = app.reader.target();
    keys(&mut app, b"/attention")?;
    super::common::search_loaded(&mut app)?;
    keys(&mut app, b"\x1b[27u")?;
    app.draw(&mut Vec::new())?;
    let restored =
      (app.reader.scroll, app.layout.pan(), app.text().position(0, 0)?);
    assert!((origin.0 - restored.0).abs() < 0.01);
    assert!((origin.1 - restored.1).abs() < 0.01);
    if text_mode {
      assert_eq!(restored.2, origin.2);
    }
    assert_eq!(app.reader.target(), target);
    let page = app.layout.page_at(app.reader.scroll);
    let point = app
      .layout
      .unproject(
        page,
        (app.layout.screen().width() / 2.0, 0.0),
        app.reader.scroll,
      )
      .unwrap();
    app.command("8")?;
    keys(&mut app, b"+\x0f")?;
    app.draw(&mut Vec::new())?;
    if text_mode {
      assert_eq!(app.text().position(0, 0)?, origin.2);
    } else {
      let projected = app.layout.project(
        page,
        Bounds { x0: point.0, x1: point.0, y0: point.1, y1: point.1 },
        app.reader.scroll,
      );
      assert!(projected.y0.abs() < 1.0);
      assert_eq!(app.navigation_position().page, page);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_normal_counts_pan_clamps_and_cursor_style() -> Result<()> {
  let mut app = app("paper.pdf", true)?;
  app.screen.columns = 32;
  app.relayout()?;
  let cursor = app.reader.cursor;
  keys(&mut app, b"10j\x1b[B2k3l\x1b[D")?;
  let mut frame = Vec::new();
  app.draw(&mut frame)?;
  assert!(!String::from_utf8_lossy(&frame).contains("\x1b[0;7;34m"));
  let top = app.text().position(0, 0)?;
  assert_eq!(top.page, PageId(0));
  assert_eq!(top.line, 9);
  assert_eq!(top.glyph, 2);
  assert_eq!(app.reader.cursor, cursor);
  keys(&mut app, b"v")?;
  frame.clear();
  app.draw(&mut frame)?;
  assert!(String::from_utf8_lossy(&frame).contains("\x1b[0;7;34m"));
  keys(&mut app, b"\x1b[27u9999h9999k")?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?, Position::default());
  keys(&mut app, b"9999j")?;
  app.draw(&mut Vec::new())?;
  let last = PageId(app.doc.pages.len() - 1);
  assert_eq!(app.text().position(0, 0)?.page, last);
  let top = app.text().position(0, 0)?;
  keys(&mut app, b"j")?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?, top);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn horizontal_keys_preserve_queued_scroll() -> Result<()> {
  for (bytes, expected) in [
    (b"jh".as_slice(), 16.0),
    (b"10j9999h".as_slice(), 160.0),
    (b"\x1b[B\x1b[C".as_slice(), 16.0),
  ] {
    let mut app = app("paper.pdf", false)?;
    keys(&mut app, bytes)?;
    assert_eq!(app.reader.animation().target().unwrap(), expected);
    settle(&mut app);
    assert_eq!(app.reader.scroll, expected);
    assert_eq!(app.layout.pan(), 0.0);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_offsets_clamp_after_rows_pages_and_resize() -> Result<()> {
  let mut app = app("paper.pdf", true)?;
  app.screen.columns = 12;
  app.screen.rows = 2;
  app.relayout()?;
  let text = app.doc.text(PageId(0))?;
  let rows: Vec<_> = text
    .lines
    .iter()
    .enumerate()
    .filter(|(_, line)| {
      !line.glyphs.is_empty() && line.glyphs.iter().all(|g| g.ch.is_ascii())
    })
    .map(|(line, row)| (line, row.glyphs.len()))
    .collect();
  let &(wide, width) = rows.iter().max_by_key(|(_, width)| *width).unwrap();
  let &(narrow, _) = rows.iter().min_by_key(|(_, width)| *width).unwrap();
  assert!(width > 12 && narrow != wide);
  let position = Position { page: PageId(0), line: wide, glyph: 0 };
  app.go_to(position, true)?;
  keys(&mut app, b"zt9999l")?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?.glyph, width - 12);
  let motion = format!(
    "{}{}",
    wide.abs_diff(narrow),
    if narrow > wide { 'j' } else { 'k' }
  );
  keys(&mut app, motion.as_bytes())?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?.line, narrow);
  keys(&mut app, b"v")?;
  assert!(app.reader.selection.is_some());
  keys(&mut app, b"\x1b[27u")?;
  app.go_to(position, true)?;
  keys(&mut app, b"zt9999l9999j")?;
  app.draw(&mut Vec::new())?;
  let top = app.text().position(0, 0)?;
  assert_eq!(top.page.0, app.doc.pages.len() - 1);
  assert_eq!(top.glyph, 0, "the final page footer fits");
  keys(&mut app, b"v\x1b[27u")?;
  assert!(app.reader.last_selection.is_some());
  app.go_to(position, true)?;
  keys(&mut app, b"zt9999l")?;
  app.screen.columns = u16::try_from(width + 1)?;
  app.relayout()?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?.glyph, 0);
  keys(&mut app, b"v")?;
  assert!(app.reader.selection.is_some());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn reload_clamps_text_view_and_invalidates_hit_testing() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("reload.pdf");
  std::fs::copy(root.join("book.pdf"), &path)?;
  let mut app = Solo::open(&path, "", true, app("paper.pdf", true)?.screen)?;
  app.command("100")?;
  keys(&mut app, b"100j10l")?;
  app.draw(&mut Vec::new())?;
  assert!(app.text().position(0, 0)?.page.0 > 99);
  app.begin_search(false)?;
  std::fs::copy(root.join("ocr.pdf"), &path)?;
  app.command("reload")?;
  assert_eq!(app.text().position(0, 0)?.page, PageId(0));
  app.cancel_prompt()?;
  assert_eq!(app.text().position(0, 0)?.page, PageId(0));
  app.draw(&mut Vec::new())?;
  let before = app.text().position(0, 0)?;
  app.command("reload")?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?, before);
  assert!(app.ocr.is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn recent_cover_preserves_view_and_explicit_job_control() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app("paper.pdf", text_mode)?;
    keys(&mut app, b"10j+3l")?;
    settle(&mut app);
    app.draw(&mut Vec::new())?;
    let view = (app.reader.scroll, app.layout.pan());
    let top = app.text().position(0, 0)?;
    let columns = app.layout.screen().columns;
    app.start_search("attention", false, false)?;
    app.start_ocr("eng")?;
    let generation = app.search.generation();
    app.show_recent(&[])?;
    keys(&mut app, b"\x1b[27u")?;
    assert!(app.covered() && app.panel_focused());
    assert_eq!(app.layout.screen().columns, columns);
    assert_eq!((app.reader.scroll, app.layout.pan()), view);
    assert_eq!(app.search.generation(), generation);
    assert!(!app.search.hidden() && app.ocr.is_some());
    app.command("cancel")?;
    assert!(app.covered() && app.panel_focused());
    assert!(app.ocr.is_none());
    assert!(matches!(app.search.phase(), vpdf_ui::find::Phase::Idle));
    app.close_panel()?;
    app.draw(&mut Vec::new())?;
    assert_eq!((app.reader.scroll, app.layout.pan()), view);
    if text_mode {
      assert_eq!(app.text().position(0, 0)?, top);
    }
    assert!(app.reader.selection.is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn space_scrolls_with_context() -> Result<()> {
  for text in [false, true] {
    for mode in [b"".as_slice(), b"i", b"v"] {
      let mut app = app("paper.pdf", text)?;
      if !mode.is_empty() {
        keys(&mut app, mode)?;
      }
      let cursor = app.reader.cursor;
      let anchor = app.reader.selection.map(|s| s.anchor);
      let top = app.text().position(0, 0)?;
      let overlap = {
        let (app, _) = app.parts();
        let row = app.layout.screen().rows - 3;
        app.text().position(0, row)?
      };
      let rows = app.layout.screen().rows.saturating_sub(3);
      let distance = f32::from(rows * app.screen.cell_height);
      keys(&mut app, b" ")?;
      settle(&mut app);
      if text {
        assert_eq!(app.text().position(0, 0)?, overlap);
      } else {
        assert_eq!(app.reader.scroll, distance);
      }
      if mode.is_empty() {
        assert_eq!(app.reader.cursor, cursor);
      } else {
        assert_ne!(app.reader.cursor, cursor);
      }
      assert_eq!(app.reader.selection.map(|s| s.anchor), anchor);
      keys(&mut app, b"\x1b[32;2u")?;
      settle(&mut app);
      assert_eq!(app.reader.scroll, 0.0);
      if text {
        assert_eq!(app.text().position(0, 0)?, top);
      }
      keys(&mut app, b"2 ")?;
      settle(&mut app);
      if !text {
        assert_eq!(app.reader.scroll, distance * 2.0);
      }
      keys(&mut app, b"999\x1b[32;2u")?;
      settle(&mut app);
      assert_eq!(app.reader.scroll, 0.0);
      if text {
        assert_eq!(app.text().position(0, 0)?, top);
      }
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn space_scrolls_short_screens() -> Result<()> {
  let mut app = app("paper.pdf", false)?;
  app.screen.rows = 3;
  app.relayout()?;
  keys(&mut app, b" ")?;
  settle(&mut app);
  assert_eq!(app.reader.scroll, f32::from(app.screen.cell_height));
  keys(&mut app, b"\x1b[32;2u")?;
  settle(&mut app);
  assert_eq!(app.reader.scroll, 0.0);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn space_accumulates_queued_scroll() -> Result<()> {
  for mode in [b"".as_slice(), b"i", b"v"] {
    let mut app = app("paper.pdf", false)?;
    if !mode.is_empty() {
      keys(&mut app, mode)?;
    }
    let anchor = app.reader.selection.map(|s| s.anchor);
    keys(&mut app, b"  ")?;
    let distance = 2.0
      * f32::from(
        app.layout.screen().rows.saturating_sub(3) * app.screen.cell_height,
      );
    assert_eq!(app.reader.scroll, 0.0);
    assert_eq!(app.reader.animation().target().unwrap(), distance);
    keys(&mut app, b"\x1b[32;2u")?;
    settle(&mut app);
    assert_eq!(app.reader.scroll, distance / 2.0);
    assert_eq!(app.reader.selection.map(|s| s.anchor), anchor);
    keys(&mut app, b"9999 ")?;
    settle(&mut app);
    assert_eq!(app.reader.scroll, app.layout.clamp_scroll(f32::MAX));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn counted_text_space_matches_repeats() -> Result<()> {
  for mode in [b"".as_slice(), b"i", b"v"] {
    let mut app = app("paper.pdf", true)?;
    if !mode.is_empty() {
      keys(&mut app, mode)?;
    }
    let top = app.text().position(0, 0)?;
    keys(&mut app, b"  ")?;
    let repeated = app.text().position(0, 0)?;
    assert_ne!(repeated, top);
    keys(&mut app, b"2\x1b[32;2u")?;
    assert_eq!(app.text().position(0, 0)?, top);
    keys(&mut app, b"2 ")?;
    assert_eq!(app.text().position(0, 0)?, repeated);
  }
  Ok(())
}
