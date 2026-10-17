use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mupdf::pdf::PdfAnnotationType;
use mupdf::{Point, Quad, Rect};
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::{App, InputOwner};
use vpdf::keys::{Action, Input, PanelKind};
use vpdf::layout::Screen;
use vpdf::panel::Destination;
use vpdf::prompt::History;
use vpdf::reader::Mode;
use vpdf::text::{PageId, Position};

fn annotated() -> Result<(tempfile::NamedTempFile, String)> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut source = vpdf::document::Document::open(&root.join("paper.pdf"), "")?;
  let text = source.text(PageId(1))?;
  let line = &text.lines[2];
  let first = line.glyphs.first().unwrap().bounds;
  let last = line.glyphs.last().unwrap().bounds;
  let mut page = source.native.load_pdf_page(1)?;
  let mut highlight = page.create_annotation(PdfAnnotationType::Highlight)?;
  highlight.set_quad_points(Quad::new(
    Point::new(first.x0, first.y0),
    Point::new(last.x1, first.y0),
    Point::new(first.x0, last.y1),
    Point::new(last.x1, last.y1),
  ))?;
  highlight.set_contents("check this")?;
  for (contents, y) in [
    ("margin note".to_owned(), 10.0),
    ("another note".to_owned(), 10.0),
    (
      format!(
        "BEGIN {} END",
        "Long reviewer note with more context. ".repeat(40)
      ),
      400.0,
    ),
  ] {
    let mut note = page.create_annotation(PdfAnnotationType::Text)?;
    note.set_rect(Rect::new(10.0, y, 30.0, y + 20.0))?;
    note.set_contents(&contents)?;
  }
  let mut empty = page.create_annotation(PdfAnnotationType::Underline)?;
  let line = &text.lines[6];
  let first = line.glyphs.first().unwrap().bounds;
  let last = line.glyphs.last().unwrap().bounds;
  empty.set_quad_points(Quad::new(
    Point::new(first.x0, first.y0),
    Point::new(last.x1, first.y0),
    Point::new(first.x0, last.y1),
    Point::new(last.x1, last.y1),
  ))?;
  drop((empty, highlight, page));
  let file = tempfile::NamedTempFile::new()?;
  source.native.save(&file.path().to_string_lossy())?;
  Ok((
    file,
    text.lines[2].text().split_whitespace().collect::<Vec<_>>().join(" "),
  ))
}

fn screen() -> Screen {
  Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  }
}

fn press(app: &mut App, code: KeyCode) -> Result<()> {
  app.key(KeyEvent::new(code, KeyModifiers::NONE), &mut History::default())
}

fn drawn(app: &mut App) -> Result<String> {
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  app.draw_preview(&mut out)?;
  app.graphics.end(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

fn position(app: &App, index: usize) -> Position {
  match &app.panel.as_ref().unwrap().rows[index].destination {
    Destination::Annotation { position, .. } => *position,
    _ => panic!("Annotation destination"),
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_lists_marked_text_and_notes() -> Result<()> {
  let (file, words) = annotated()?;
  let mut app = App::with_screen(file.path(), "", false, screen())?;
  app.open_panel(PanelKind::Annotations)?;
  let rows = &app.panel.as_ref().unwrap().rows;
  let labels: Vec<_> =
    rows.iter().take(2).map(|row| row.label.as_str()).collect();
  assert_eq!(
    labels,
    [format!("Highlight: {words} — check this"), "Note: margin note".into()]
  );
  assert_eq!(rows.len(), 5);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn enter_positions_cursor_and_note() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    for entry in [0, 1, 2] {
      let mut app = App::with_screen(file.path(), "", text, screen())?;
      if entry > 0 {
        press(&mut app, KeyCode::Char('i'))?;
      }
      if entry > 1 {
        press(&mut app, KeyCode::Char('v'))?;
      }
      app.open_panel(PanelKind::Annotations)?;
      let expected = position(&app, 0);
      press(&mut app, KeyCode::Enter)?;
      assert_eq!(app.reader.cursor, expected);
      assert_eq!(app.target, Some(expected));
      assert_eq!(app.reader.mode(), Mode::Cursor);
      assert_eq!(app.input_owner(), InputOwner::Peek);
      assert!(!app.panel_focused());
      assert_eq!(app.panel.as_ref().unwrap().rows.len(), 5);
      assert!(drawn(&mut app)?.contains("check this"));
      press(&mut app, KeyCode::Esc)?;
      assert!(app.peek.is_none());
      assert_eq!(app.reader.mode(), Mode::Cursor);
      press(&mut app, KeyCode::Char('K'))?;
      assert!(drawn(&mut app)?.contains("check this"));
      let jumps = app.reader.jumps().count();
      press(&mut app, KeyCode::Enter)?;
      assert!(app.peek.is_none());
      assert_eq!(app.reader.cursor, expected);
      assert_eq!(app.reader.mode(), Mode::Cursor);
      assert_eq!(app.reader.jumps().count(), jumps);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn k_previews_selected_note() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    let mut app = App::with_screen(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    assert_eq!(position(&app, 1), position(&app, 2));
    let original = app.reader.cursor;
    app.panel.as_mut().unwrap().selected = 2;
    press(&mut app, KeyCode::Char('K'))?;
    let out = drawn(&mut app)?;
    assert!(out.contains("another note"));
    assert!(!out.contains("margin note"));
    assert_eq!(app.reader.cursor, original);
    assert!(app.panel_focused());
    press(&mut app, KeyCode::Esc)?;
    assert_eq!(app.input_owner(), InputOwner::Panel);
    let expected = position(&app, 2);
    press(&mut app, KeyCode::Enter)?;
    assert_eq!(app.reader.cursor, expected);
    assert!(drawn(&mut app)?.contains("another note"));
    assert_eq!(app.reader.mode(), Mode::Cursor);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn k_preserves_visual_range() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    let mut app = App::with_screen(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    app.panel.as_mut().unwrap().selected = 1;
    press(&mut app, KeyCode::Enter)?;
    press(&mut app, KeyCode::Esc)?;
    let cursor = app.reader.cursor;
    press(&mut app, KeyCode::Char('v'))?;
    let selection = app.reader.selection.unwrap();
    press(&mut app, KeyCode::Char('K'))?;
    assert!(drawn(&mut app)?.contains("margin note"));
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, selection.anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(selection.kind));
    press(&mut app, KeyCode::Esc)?;
    assert_eq!(app.reader.mode(), Mode::Visual(selection.kind));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn notes_scroll_and_resize() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    let mut app = App::with_screen(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    app.panel.as_mut().unwrap().selected = 3;
    press(&mut app, KeyCode::Enter)?;
    let cursor = app.reader.cursor;
    let first = drawn(&mut app)?;
    assert!(first.contains("BEGIN"));
    assert!(first.contains('↓'));
    if !text {
      assert!(first.contains("48;2;255;255;255m"));
    }
    app.scroll_preview(9999.0)?;
    let last = drawn(&mut app)?;
    assert!(last.contains("END"));
    assert!(last.contains('↑'));
    assert!(!last.contains('↓'));
    app.screen.columns = 54;
    app.relayout()?;
    app.scroll_preview(-9999.0)?;
    let narrow = drawn(&mut app)?;
    assert!(narrow.contains("BEGIN"));
    assert_ne!(first, narrow);
    let label = narrow.split_once('╭').unwrap().1.split('╮').next().unwrap();
    assert!(
      label.chars().count() + 2 <= usize::from(app.layout.screen.columns)
    );
    assert_eq!(app.reader.cursor, cursor);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn empty_notes_and_panel_focus() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    let mut app = App::with_screen(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    app.panel.as_mut().unwrap().selected = 4;
    let expected = position(&app, 4);
    press(&mut app, KeyCode::Enter)?;
    assert_eq!(app.reader.cursor, expected);
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert!(app.peek.is_none());
    press(&mut app, KeyCode::Tab)?;
    app.panel.as_mut().unwrap().selected = 1;
    app.action(Input {
      action: Action::Swap,
      count: 1,
      explicit: false,
      register: '"',
    })?;
    assert!(app.panel_focused());
    assert!(app.peek.is_none());
  }
  Ok(())
}
