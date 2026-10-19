use super::common::keys;
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::Target;
use vpdf::layout::Screen;
use vpdf::solo::Solo;
use vpdf::text::{Bounds, Position};

fn app(text_mode: bool) -> Result<Solo> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut app = Solo::open(
    &root.join("paper.pdf"),
    "",
    text_mode,
    Screen {
      columns: 100,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  )?;
  super::common::outline_loaded(&mut app)?;
  Ok(app)
}

/// Text view rows of the first `pages` pages, with their breaks.
fn text_rows(app: &mut App, pages: usize) -> Result<usize> {
  let mut rows = 0;
  for page in 0..pages {
    rows += app.doc.text(vpdf::text::PageId(page))?.lines.len().max(1) + 1;
  }
  Ok(rows)
}

fn heading(app: &mut App, index: usize) -> Result<(Position, f32)> {
  let Target::Point { page, x, y } = app.doc.outline[index].target else {
    panic!("Expected heading point");
  };
  let position = app.doc.text(page)?.nearest(page, x, y);
  let point = Bounds { x0: x, x1: x, y0: y, y1: y };
  let scroll = app.layout.clamp_scroll(app.layout.project(page, point, 0.0).y0);
  Ok((position, scroll))
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_boundaries_and_counts() -> Result<()> {
  for text_mode in [false, true] {
    for visual in [false, true] {
      let mut app = app(text_mode)?;
      let (second, scroll) = heading(&mut app, 1)?;
      for (body, bytes, expected) in [
        (false, b"[[".as_slice(), 0),
        (true, b"[[", 1),
        (true, b"2[[", 0),
        (false, b"]]", 2),
        (true, b"]]", 2),
        (false, b"2]]", 3),
        (true, b"2]]", 3),
      ] {
        app.reader.cancel();
        let start =
          Position { line: second.line + if body { 2 } else { 0 }, ..second };
        app.target = Some(start);
        app.reader.cursor = start;
        app.reader.scroll = scroll + if body { 40.0 } else { 0.0 };
        if text_mode {
          app.go_to(start, true)?;
          keys(&mut app, b"zt")?;
        }
        if visual {
          keys(&mut app, b"v")?;
          assert_eq!(app.reader.selection.unwrap().anchor, start);
        } else {
          // The viewport must win over stale targets.
          app.reader.cursor = Position::default();
          app.target = Some(second);
        }
        app.reader.clear_history();
        let original = (app.reader.scroll, app.navigation_position());
        keys(&mut app, bytes)?;
        let (want, landing) = heading(&mut app, expected)?;
        assert_eq!(app.reader.cursor, want, "{text_mode} {visual} {bytes:?}");
        assert_eq!(app.reader.jumps().count(), 1);
        if visual {
          assert_eq!(app.reader.selection.unwrap().anchor, start);
        } else if !text_mode {
          assert_eq!(app.reader.scroll, landing);
        }
        keys(&mut app, b"\x0f")?;
        assert_eq!(app.reader.scroll, original.0);
        if text_mode && !visual {
          assert_eq!(app.navigation_position(), original.1);
        }
        keys(&mut app, b"\x1b[105;5u")?;
        assert_eq!(app.reader.cursor, want);
      }
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn outline_follows_reading_section() -> Result<()> {
  use vpdf::keys::{Action, Input, PanelKind};

  for text_mode in [false, true] {
    for mode in [b"".as_slice(), b"i", b"v"] {
      let mut app = app(text_mode)?;
      app.screen.rows = 8;
      app.open_panel(PanelKind::Outline)?;
      keys(&mut app, b"\t")?;
      if !mode.is_empty() {
        keys(&mut app, mode)?;
      }
      app.draw(&mut Vec::new())?;
      for title in ["Positional Encoding", "Regularization", "Introduction"] {
        let index =
          app.doc.outline.iter().position(|h| h.title.trim() == title).unwrap();
        let (position, scroll) = heading(&mut app, index)?;
        app.go_to(Position { line: position.line + 2, ..position }, true)?;
        match (text_mode, app.reader.viewport_mode()) {
          (true, true) => keys(&mut app, b"zt")?,
          (false, true) => app.reader.scroll = scroll + 1.0,
          _ => {}
        }
        app.draw(&mut Vec::new())?;
        let section = app.current_section.unwrap();
        assert!(app.reader.viewport_mode() || section == index, "{title}");
        let panel = app.panel.as_ref().unwrap();
        assert_eq!(panel.selected, section, "{text_mode} {mode:?} {title}");
        let visible = panel.top..panel.top + usize::from(app.screen.rows - 2);
        assert!(visible.contains(&section), "{title} visible");
      }
      keys(&mut app, b"\t2j")?;
      let selected = app.panel.as_ref().unwrap().selected;
      let (position, _) = heading(&mut app, 1)?;
      app.go_to(Position { line: position.line + 2, ..position }, true)?;
      app.viewport(Input::plain(Action::Align(0.0)))?;
      app.draw(&mut Vec::new())?;
      assert_eq!(app.panel.as_ref().unwrap().selected, selected);
      keys(&mut app, b"\t")?;
      app.draw(&mut Vec::new())?;
      assert_eq!(
        app.panel.as_ref().unwrap().selected,
        app.current_section.unwrap()
      );
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn outline_follows_after_loading() -> Result<()> {
  use vpdf::keys::PanelKind;

  let mut app = app(false)?;
  let (position, _) = heading(&mut app, 10)?;
  app.go_to(Position { line: position.line + 2, ..position }, true)?;
  keys(&mut app, b"i")?;
  app.doc.outline.clear();
  app.load_outline();
  app.open_panel(PanelKind::Outline)?;
  keys(&mut app, b"\t")?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.current_section, None);
  super::common::outline_loaded(&mut app)?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.current_section, Some(10));
  let panel = app.panel.as_ref().unwrap();
  assert_eq!(panel.selected, 10);
  assert!(!panel.focused);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_repeat_landing() -> Result<()> {
  let mut app = app(false)?;
  for zoom in [0.73, 1.0, 1.77] {
    app.layout.zoom = zoom;
    app.relayout()?;
    let (_, second) = heading(&mut app, 1)?;
    app.reader.scroll = second + 40.0;
    app.reader.cursor = Position::default();
    app.target = None;
    for (bytes, index) in [(b"[[", 1), (b"[[", 0), (b"]]", 1), (b"]]", 2)] {
      keys(&mut app, bytes)?;
      let (position, scroll) = heading(&mut app, index)?;
      assert_eq!(app.reader.cursor, position, "{zoom} {bytes:?}");
      assert_eq!(app.reader.scroll, scroll);
    }
    keys(&mut app, b"9999[[")?;
    let first = heading(&mut app, 0)?.0;
    assert_eq!(app.reader.cursor, first);
    app.reader.clear_history();
    let scroll = app.reader.scroll;
    keys(&mut app, b"[[")?;
    assert_eq!(app.reader.scroll, scroll);
    assert_eq!(app.reader.jumps().count(), 0);
    keys(&mut app, b"9999]]")?;
    let last = app.doc.outline.len() - 1;
    let last = heading(&mut app, last)?.0;
    assert_eq!(app.reader.cursor, last);
    app.reader.clear_history();
    keys(&mut app, b"]]")?;
    assert_eq!(app.reader.jumps().count(), 0);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_clipped_landings() -> Result<()> {
  let cases = [(false, false), (false, true), (true, false), (true, true)];
  for (text_mode, full) in cases {
    let mut app = app(text_mode)?;
    let height = app.layout.total - app.layout.pages[1].top;
    app.screen.rows = if full {
      u16::MAX
    } else if text_mode {
      // The second heading's row tops the end.
      let pages = app.doc.pages.len();
      let rows = text_rows(&mut app, pages)? - 1;
      let second = heading(&mut app, 1)?.0;
      let above = text_rows(&mut app, second.page.0)? + second.line;
      u16::try_from(rows - above + 1)?
    } else {
      (height / f32::from(app.screen.cell_height)).ceil() as u16 + 1
    };
    app.relayout()?;
    let first = heading(&mut app, 0)?.0;
    app.go_to(first, true)?;
    if !text_mode {
      app.reader.scroll = app.layout.clamp_scroll(f32::MAX);
    }
    app.reader.clear_history();
    for (bytes, expected) in [
      (b"]]".as_slice(), 1),
      (b"]]", 2),
      (b"[[", 1),
      (b"\x0f", 2),
      (b"\x1b[105;5u", 1),
      (b"2]]", 3),
    ] {
      keys(&mut app, bytes)?;
      let want = heading(&mut app, expected)?.0;
      assert_eq!(app.reader.cursor, want, "{text_mode} {full} {bytes:?}");
      if !text_mode {
        assert_eq!(app.reader.scroll, app.layout.clamp_scroll(f32::MAX));
      }
    }
    // A full view cannot scroll.
    if text_mode && !full {
      // A changed viewport invalidates the landing.
      app.scroll_text(-1)?;
      let top = app.navigation_position();
      keys(&mut app, b"]]")?;
      let mut next = 0;
      while heading(&mut app, next)?.0 <= top {
        next += 1;
      }
      let after_landing = 4;
      assert_ne!(next, after_landing);
      let want = heading(&mut app, next)?.0;
      assert_eq!(app.reader.cursor, want);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_scroll_invalidates_landing() -> Result<()> {
  let mut app = app(false)?;
  let (_, second) = heading(&mut app, 1)?;
  app.reader.scroll = second + 40.0;
  keys(&mut app, b"[[")?;
  let (want, second) = heading(&mut app, 1)?;
  assert_eq!(app.target, Some(want));
  keys(&mut app, b"3j")?;
  app.reader.scroll = app.animation.take().unwrap().target;
  keys(&mut app, b"[[")?;
  assert_eq!(app.reader.cursor, want);
  assert_eq!(app.reader.scroll, second);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_same_baseline_targets() -> Result<()> {
  for visual in [false, true] {
    let mut app = app(false)?;
    app.screen.rows = u16::MAX;
    app.relayout()?;
    let position = heading(&mut app, 1)?.0;
    let text = app.doc.text(position.page)?;
    let row = text.lines.iter().find(|row| row.glyphs.len() >= 20).unwrap();
    let bounds = row.glyphs[0].bounds;
    let y = f32::midpoint(bounds.y0, bounds.y1);
    let mut targets = Vec::new();
    for glyph in [0, 2, 4] {
      let bounds = row.glyphs[glyph].bounds;
      let x = f32::midpoint(bounds.x0, bounds.x1);
      targets.push(vpdf::document::Heading {
        title: row.glyphs[glyph].ch.to_string(),
        depth: 0,
        target: Target::Point { page: position.page, x, y },
      });
    }
    app.doc.outline = targets;
    let first = heading(&mut app, 0)?.0;
    app.go_to(first, true)?;
    app.reader.scroll = 0.0;
    if visual {
      keys(&mut app, b"v")?;
    }
    for (bytes, index) in [(b"]]", 1), (b"]]", 2), (b"[[", 1), (b"[[", 0)] {
      keys(&mut app, bytes)?;
      let want = heading(&mut app, index)?.0;
      assert_eq!(app.reader.cursor, want, "{visual} {bytes:?}");
      if visual {
        assert_eq!(app.reader.selection.unwrap().anchor, first);
      }
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_pan_invalidates_landing() -> Result<()> {
  let mut app = app(false)?;
  app.screen.rows = u16::MAX;
  app.layout.zoom = 2.0;
  app.relayout()?;
  app.target = None;
  keys(&mut app, b"]]")?;
  let first = heading(&mut app, 0)?.0;
  assert_eq!(app.reader.cursor, first);
  let pan = app.layout.pan;
  keys(&mut app, b"l")?;
  assert!(app.layout.pan > pan);
  keys(&mut app, b"]]")?;
  assert_eq!(app.reader.cursor, first);
  assert_eq!(app.layout.pan, pan);
  Ok(())
}
