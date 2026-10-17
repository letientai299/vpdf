use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use vpdf::Result;
use vpdf::app::App;
use vpdf::decode::{Event, Parser};
use vpdf::layout::{Crop, Screen};
use vpdf::prompt::History;
use vpdf::reader::SelectionKind;
use vpdf::text::{Bounds, Glyph, Line, PageId, Position, Text};
const BAND: &str = "\x1b[0;100m";
const SELECTION: &str = "\x1b[0;7;35m";
const CURSOR: &str = "\x1b[0;7;34m";
const LANDING: &str = "\x1b[0;43m";
const SEARCH: &str = "\x1b[0;7;33m";

fn app(text: bool) -> Result<App> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  App::with_screen(
    &root.join("paper.pdf"),
    "",
    text,
    Screen {
      columns: 100,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (7, 3),
    },
  )
}

fn keys(app: &mut App, bytes: &[u8]) -> Result<()> {
  let mut events = VecDeque::new();
  Parser::default().feed(bytes, &mut events);
  for event in events {
    let Event::Key(key) = event else { panic!("Expected key") };
    app.key(key, &mut History::default())?;
  }
  Ok(())
}

fn marks(app: &mut App, screen: Screen) -> Result<Vec<Bounds>> {
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  app.marks(screen, &mut out)?;
  app.graphics.end(&mut out)?;
  let frame = String::from_utf8(out)?;
  let mut rects = Vec::new();
  for part in frame.split("\x1b7\x1b[").skip(1) {
    let (row, rest) = part.split_once(';').unwrap();
    let (column, rest) = rest.split_once('H').unwrap();
    let control =
      rest.split_once("\x1b_G").unwrap().1.split(';').next().unwrap();
    let field = |name: &str| {
      control
        .split(',')
        .find_map(|pair| pair.strip_prefix(name)?.parse::<u16>().ok())
    };
    if !control.contains("z=1,") && !control.contains("z=-1,") {
      continue;
    }
    let x0 = f32::from(column.parse::<u16>()? - 1 - screen.origin.0) * 8.0
      + f32::from(field("X=").unwrap_or(0));
    let y0 = f32::from(row.parse::<u16>()? - 1 - screen.origin.1) * 16.0
      + f32::from(field("Y=").unwrap_or(0));
    rects.push(Bounds {
      x0,
      y0,
      x1: x0 + f32::from(field("w=").unwrap()),
      y1: y0 + f32::from(field("h=").unwrap()),
    });
  }
  Ok(rects)
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_toggle_preserves_visual_and_copy() -> Result<()> {
  for text in [false, true] {
    let mut app = app(text)?;
    assert!(app.cursorline);
    keys(&mut app, b"vll:cursorline\r")?;
    let range = (app.reader.selection.unwrap().anchor, app.reader.cursor);
    assert!(!app.cursorline);
    keys(&mut app, b":cursorline\r")?;
    assert!(app.cursorline);
    assert_eq!(
      (app.reader.selection.unwrap().anchor, app.reader.cursor),
      range
    );
    keys(&mut app, b"y")?;
    assert!(app.reader.selection.is_none());
    let yank = app.reader.pending_yank.as_ref().unwrap().text.clone();
    app.command("cursorline")?;
    assert_eq!(app.reader.pending_yank.as_ref().unwrap().text, yank);
    if !text {
      let screen = app.layout.screen;
      assert!(marks(&mut app, screen)?.is_empty());
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_pdf_projects_and_clips() -> Result<()> {
  for rotation in [0, 90, 180, 270] {
    for zoom in [0.5, 2.0] {
      let mut app = app(false)?;
      app.screen.columns = 40;
      app.screen.rows = 15;
      app.relayout()?;
      app.layout.rotation = rotation;
      app.layout.zoom = zoom;
      app.layout.crop = Crop([30, 30, 30, 30]);
      app.layout.reflow(&app.doc.pages);
      let text = app.doc.text(PageId(0))?;
      let line =
        text.lines.iter().position(|line| line.glyphs.len() > 10).unwrap();
      app.reader.cursor = Position { page: PageId(0), line, glyph: 5 };
      app.reader.visual(SelectionKind::Character);
      app.ensure_cursor()?;
      app.left = 10;
      let screen = Screen { rows: 14, ..app.layout.screen };
      let on = marks(&mut app, screen)?;
      app.command("cursorline")?;
      let off = marks(&mut app, screen)?;
      assert!(on.len() > off.len(), "{rotation} {zoom}");
      assert_eq!(off.len(), 2, "selection and cursor remain");
      let band = on[0];
      assert!(band.width() > off[0].width() || band.height() > off[0].height());
      for rect in on {
        assert!(rect.x0 >= 80.0 && rect.x1 <= 80.0 + screen.width());
        assert!(rect.y0 >= 0.0 && rect.y1 <= screen.height());
      }
      app.reader.end_selection();
      app.command("cursorline")?;
      assert!(marks(&mut app, screen)?.is_empty());
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_text_fills_only_active_content_row() -> Result<()> {
  let mut app = app(true)?;
  app.screen.columns = 20;
  app.left = 4;
  app.reader.cursor = Position::default();
  let rows = ["abcd", "", "other"];
  app.doc.seed(
    PageId(0),
    Arc::new(Text {
      lines: rows
        .iter()
        .map(|row| Line {
          paragraph: false,
          glyphs: row
            .chars()
            .map(|ch| Glyph { ch, bounds: Bounds::default() })
            .collect(),
        })
        .collect(),
      truncated: false,
    }),
  );
  app.reader.visual(SelectionKind::Character);
  app.reader.cursor.glyph = 1;
  app.search.hits.push(vpdf::search::Hit {
    first: Position { glyph: 2, ..Position::default() },
    last: Position { glyph: 2, ..Position::default() },
    excerpt: "c".into(),
  });
  let mut out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  let fill = format!("{}{}", BAND, " ".repeat(16));
  assert_eq!(frame.matches(&fill).count(), 1);
  assert!(frame.starts_with(&format!(
    "{}{}",
    app.at.goto(0_usize, app.left),
    fill
  )));
  for style in [SELECTION, CURSOR, SEARCH] {
    assert!(frame.contains(style));
  }
  app.command("cursorline")?;
  let mut out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  assert!(!frame.contains(BAND));
  for style in [SELECTION, CURSOR, SEARCH] {
    assert!(frame.contains(style));
  }
  app.command("cursorline")?;
  app.reader.cursor.line = 1;
  out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  assert!(frame.contains(&format!(
    "{}{}",
    app.at.goto(1_usize, app.left),
    fill
  )));
  app.reader.end_selection();
  let mut out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  assert!(!frame.contains(BAND));
  assert!(!frame.contains(CURSOR));
  assert!(frame.contains(SEARCH));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_stays_below_existing_selection() -> Result<()> {
  let mut app = app(false)?;
  app.command("2")?;
  app.command("cursorline")?;
  app.reader.visual(SelectionKind::Character);
  let screen = app.layout.screen;
  marks(&mut app, screen)?;
  app.command("cursorline")?;
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  app.marks(screen, &mut out)?;
  app.graphics.end(&mut out)?;
  let frame = String::from_utf8(out)?;
  assert!(frame.contains("z=-1,"), "band stays below selection images");
  assert!(frame.contains("z=1,"), "selection remains above the band");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_jump_modes_and_fixed_destination() -> Result<()> {
  for text_mode in [false, true] {
    for mode in [b"".as_slice(), b"i", b"iv"] {
      let mut app = app(text_mode)?;
      keys(&mut app, mode)?;
      let initial_mode = app.reader.mode();
      let target = Position { line: 5, ..Position::default() };
      app.go_to(target, true)?;
      let target = app.reader.cursor;
      assert_eq!(app.landing.as_ref().unwrap().position, target);
      assert_eq!(app.reader.mode(), initial_mode);
      let until = app.landing.as_ref().unwrap().until;
      keys(&mut app, b"j")?;
      assert_eq!(app.landing.as_ref().unwrap().position, target);
      assert_eq!(app.landing.as_ref().unwrap().until, until);
      app.landing = None;
      keys(&mut app, b"j\x04")?;
      assert!(app.landing.is_none(), "ordinary motions must not flash");
      app.marks.insert('a', target);
      app.go_mark('a', true)?;
      assert_eq!(app.landing.as_ref().unwrap().position, target);
      app.landing = None;
      app.follow(&vpdf::document::Target::Page(PageId(1)))?;
      assert_eq!(app.landing.as_ref().unwrap().position, app.reader.cursor);
      assert_eq!(app.reader.cursor.page, PageId(1));
      app.landing = None;
      super::common::outline_loaded(&mut app)?;
      keys(&mut app, b"]]")?;
      assert_eq!(app.landing.as_ref().unwrap().position, app.reader.cursor);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_pdf_reuses_projected_clipped_band() -> Result<()> {
  for rotation in [0, 90, 180, 270] {
    for zoom in [0.5, 2.0] {
      let mut app = app(false)?;
      app.screen.columns = 40;
      app.screen.rows = 15;
      app.relayout()?;
      app.layout.rotation = rotation;
      app.layout.zoom = zoom;
      app.layout.crop = Crop([30, 30, 30, 30]);
      app.layout.reflow(&app.doc.pages);
      let text = app.doc.text(PageId(0))?;
      let line =
        text.lines.iter().position(|line| line.glyphs.len() > 10).unwrap();
      app.go_to(Position { page: PageId(0), line, glyph: 5 }, true)?;
      app.left = 10;
      let screen = Screen { rows: 14, ..app.layout.screen };
      let band = marks(&mut app, screen)?;
      assert!(!band.is_empty(), "Normal has landing feedback");
      app.landing = None;
      app.reader.visual(SelectionKind::Character);
      let visual = marks(&mut app, screen)?;
      let coords = |b: Bounds| (b.x0, b.y0, b.x1, b.y1);
      for (flash, cursorline) in band.iter().zip(&visual) {
        assert_eq!(coords(*flash), coords(*cursorline), "{rotation} {zoom}");
        assert!(flash.x0 >= 80.0 && flash.x1 <= 80.0 + screen.width());
        assert!(flash.y0 >= 0.0 && flash.y1 <= screen.height());
      }
      app.go_to(app.reader.cursor, true)?;
      let mut out = Vec::new();
      app.graphics.begin(&mut out)?;
      app.marks(screen, &mut out)?;
      app.graphics.end(&mut out)?;
      let frame = String::from_utf8(out)?;
      assert!(frame.contains("z=-1,"));
      assert!(frame.contains("z=1,"));
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_text_fills_row_below_glyph_styles() -> Result<()> {
  let mut app = app(true)?;
  app.left = 4;
  let position = Position { line: 5, glyph: 1, ..Position::default() };
  app.go_to(position, true)?;
  let position = app.reader.cursor;
  app.search.hits.push(vpdf::search::Hit {
    first: Position { glyph: 3, ..position },
    last: Position { glyph: 3, ..position },
    excerpt: String::new(),
  });
  let mut out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  let fill = format!("{}{}", LANDING, " ".repeat(96));
  assert_eq!(frame.matches(&fill).count(), 1);
  assert!(!frame.contains(CURSOR));
  assert!(frame.contains(SEARCH));
  keys(&mut app, b"ivl")?;
  out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  for style in [LANDING, CURSOR, SELECTION, SEARCH] {
    assert!(frame.contains(style), "missing {style:?}");
  }
  app.landing = None;
  out = Vec::new();
  app.text_canvas(&mut out)?;
  let frame = String::from_utf8(out)?;
  assert!(!frame.contains(LANDING));
  assert!(frame.contains(BAND));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_search_acceptance_and_cancellation() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app(text_mode)?;
    app.begin_search(false)?;
    app.incremental("attention")?;
    super::common::search_loaded(&mut app)?;
    assert!(app.landing.is_none(), "typing only previews");
    app.abort_search()?;
    assert!(app.landing.is_none(), "canceling never jumps");
    app.begin_search(false)?;
    app.incremental("attention")?;
    super::common::search_loaded(&mut app)?;
    app.finish_search("attention", false)?;
    assert_eq!(app.landing.as_ref().unwrap().position, app.reader.cursor);
    app.landing = None;
    app.next_result(false, 1)?;
    assert_eq!(app.landing.as_ref().unwrap().position, app.reader.cursor);
    app.landing = None;
    app.start_search("model", false, false)?;
    super::common::search_loaded(&mut app)?;
    assert_eq!(app.landing.as_ref().unwrap().position, app.reader.cursor);
    app.landing = None;
    app.start_search("qqqnothingzzznomatch", false, false)?;
    super::common::search_loaded(&mut app)?;
    assert!(app.landing.is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_reload_clears_feedback() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app(text_mode)?;
    app.go_to(Position { line: 5, ..Position::default() }, true)?;
    assert!(app.landing.is_some());
    app.reload_with("")?;
    assert!(app.landing.is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_search_early_commit_uses_current_query() -> Result<()> {
  for text_mode in [false, true] {
    for query in ["model", "qqqnothingzzznomatch"] {
      let mut app = app(text_mode)?;
      app.begin_search(false)?;
      app.incremental("attention")?;
      super::common::search_loaded(&mut app)?;
      let previous = app.reader.cursor;
      assert!(app.landing.is_none());
      app.incremental(query)?;
      assert!(matches!(
        app.search.phase,
        vpdf::find::Phase::Running { jump: true }
      ));
      app.finish_search(query, false)?;
      assert_eq!(app.reader.cursor, previous);
      assert!(app.landing.is_none(), "previous query must not flash");
      super::common::search_loaded(&mut app)?;
      assert_eq!(app.search.query.text, query);
      if query == "model" {
        let target = app.landing.as_ref().unwrap().position;
        assert_eq!(target, app.reader.cursor);
        assert!(app.search.hits.iter().any(|hit| hit.first == target));
      } else {
        assert!(app.search.hits.is_empty());
        assert!(app.landing.is_none(), "failed query must not flash");
      }
    }
  }
  Ok(())
}
