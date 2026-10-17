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
  app.command("page 2")?;
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
