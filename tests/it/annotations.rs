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
    app.action(Input::plain(Action::Swap))?;
    assert!(app.panel_focused());
    assert!(app.peek.is_none());
  }
  Ok(())
}

/// Highlights, a link, a margin note.
fn noted_pdf(path: &std::path::Path) -> Result<()> {
  let content = "BT /F1 18 Tf 24 TL 20 170 Td (Hover over these lines) Tj T* \
                 (a second line) Tj ET";
  let quads = |top: u32| {
    let bottom = top - 24;
    format!("[20 {top} 230 {top} 20 {bottom} 230 {bottom}]")
  };
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
     /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R \
     /Annots [6 0 R 7 0 R 8 0 R 9 0 R] >>"
      .to_owned(),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    format!(
      "<< /Type /Annot /Subtype /Highlight /Rect [20 165 230 189] \
       /QuadPoints {} >>",
      quads(189)
    ),
    format!(
      "<< /Type /Annot /Subtype /Highlight /Rect [20 141 230 165] \
       /QuadPoints {} /Contents (check this note please) /T (Ada) >>",
      quads(165)
    ),
    "<< /Type /Annot /Subtype /Link /Rect [20 141 40 165] \
     /A << /S /URI /URI (https://example.com) >> >>"
      .to_owned(),
    "<< /Type /Annot /Subtype /Text /Rect [260 150 280 170] \
     /Contents (margin note) >>"
      .to_owned(),
  ];
  crate::common::write_pdf(path, &objects, "")
}

#[test]
fn hover_details_notes_rows_and_pages() -> Result<()> {
  use crossterm::event::{MouseEvent, MouseEventKind};
  use vpdf::document::{Heading, Target};
  use vpdf::pointer::{Hit, Region};
  use vpdf::session::Session;
  use vpdf::trace::Drawable;
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("noted.pdf");
  noted_pdf(&path)?;
  let mut session = Session::new(None);
  session.open(&path, true, screen())?;
  let cells: Vec<_> =
    (0..39).flat_map(|row| (0..120).map(move |column| (column, row))).collect();
  let find = |session: &mut Session, region, wanted: &dyn Fn(Hit) -> bool| {
    let app = session.app();
    let cell = cells.iter().copied().find(|&(column, row)| {
      app.hovered(region, column, row, None).is_ok_and(wanted)
    });
    cell.expect("a cell with that hit")
  };
  let on_page = |session: &mut Session, wanted: &dyn Fn(Hit) -> bool| {
    find(session, Region::Page, wanted)
  };
  let note =
    on_page(&mut session, &|hit| matches!(hit, Hit::Note { text: true, .. }));
  let margin =
    on_page(&mut session, &|hit| matches!(hit, Hit::Note { text: false, .. }));
  let link = on_page(&mut session, &|hit| matches!(hit, Hit::Link(..)));
  let blank = on_page(&mut session, &|hit| hit == Hit::Nothing);
  // (cell) -> frame, status text
  let hover =
    |session: &mut Session, (column, row)| -> Result<(bool, String)> {
      let kind = MouseEventKind::Moved;
      let modifiers = KeyModifiers::NONE;
      let frame = session.mouse(MouseEvent { kind, column, row, modifiers })?;
      let mut out = Vec::new();
      session.draw_frame(&mut out)?;
      Ok((frame, String::from_utf8_lossy(&out).into_owned()))
    };
  let ada = "Highlight · Ada: check this note please";
  // (cell, frame, shown, hidden)
  let steps = [
    (note, true, Some(ada), None),
    (note, false, Some(ada), None),
    (margin, true, Some("Note: margin note"), Some(" · ")),
    // The link wins over its note.
    (link, true, None, Some(ada)),
    (blank, true, None, Some(ada)),
  ];
  for (step, (cell, frame, shown, hidden)) in steps.into_iter().enumerate() {
    let (framed, drawn) = hover(&mut session, cell)?;
    assert_eq!(framed, frame, "step {step}");
    if let Some(shown) = shown {
      assert!(drawn.contains(shown), "step {step}: {shown}");
    }
    if let Some(hidden) = hidden {
      assert!(!drawn.contains(hidden), "step {step}: {hidden}");
    }
  }
  // Clipped rows show whole; selection stays.
  session.app().open_panel(PanelKind::Annotations)?;
  let focused = session.app().panel_focused();
  let (_, drawn) = hover(&mut session, (1, 2))?;
  let app = session.app();
  assert_eq!(app.hover, Hit::Row(1));
  let row = &app.panel.as_ref().unwrap().rows[1].label;
  assert!(drawn.contains(row.as_str()), "{row}");
  assert!(drawn.contains("\x1b[0;4m"), "the row underlines");
  assert_eq!(app.panel.as_ref().unwrap().selected, 0);
  assert_eq!(app.panel_focused(), focused);
  // A short row adds nothing.
  let (_, drawn) = hover(&mut session, (1, 3))?;
  assert!(!drawn.contains("Note: margin note"), "{drawn}");
  session.app().close_panel()?;
  // A panel opened under a resting pointer.
  session.app().open_panel(PanelKind::Annotations)?;
  session.draw_frame(&mut Vec::new())?;
  if session.redraw_due() {
    session.draw_frame(&mut Vec::new())?;
  }
  assert!(!session.redraw_due(), "one follow-up at most");
  assert_eq!(session.app().hover, Hit::Row(2));
  session.app().close_panel()?;
  // Thumbnails show their page label and section.
  let app = session.app();
  app.doc.outline.push(Heading {
    title: "Opening".into(),
    depth: 0,
    target: Target::Page(PageId(0)),
  });
  app.open_overview()?;
  let thumbnail = find(&mut session, Region::Overview, &|hit| {
    matches!(hit, Hit::Thumbnail(_))
  });
  let (_, drawn) = hover(&mut session, thumbnail)?;
  assert_eq!(session.app().hover, Hit::Thumbnail(PageId(0)));
  assert!(drawn.contains("p. 1 · Opening"));
  Ok(())
}

#[test]
fn hover_detail_yields_to_messages_and_caches() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("noted.pdf");
  noted_pdf(&path)?;
  // (error, message, covered) -> detail shown
  let cases = [
    (None, "", false, true),
    (Some("Failed"), "", false, false),
    (None, "Copied", false, false),
    (None, "Copied", true, true),
    (Some("Failed"), "", true, false),
  ];
  for (error, message, covered, shown) in cases {
    let mut app = App::with_screen(&path, "", false, screen())?;
    if covered {
      app.show_recent(&[])?;
    }
    app.error = error.map(str::to_owned);
    app.reader.message = message.into();
    let mut out = Vec::new();
    app.status_line(&mut out, 39, Some("DETAIL"))?;
    let drawn = String::from_utf8_lossy(&out);
    let at = format!("{error:?} {message:?} covered={covered}");
    assert_eq!(drawn.contains("DETAIL"), shown, "{at}");
  }
  // Trimming drops the annotation cache.
  let mut doc = vpdf::document::Document::open(&path, "")?;
  let first = doc.annotations(PageId(0))?;
  assert!(std::sync::Arc::ptr_eq(&first, &doc.annotations(PageId(0))?));
  doc.trim();
  let again = doc.annotations(PageId(0))?;
  assert!(!std::sync::Arc::ptr_eq(&first, &again));
  assert_eq!(again.len(), 3);
  Ok(())
}
