use super::common::SeedReader;
use super::common::keys;
use super::common::preview_frame as drawn;
use crate::harness::corpus;
use crate::harness::{cell_where, drag, mouse};
use mupdf::pdf::PdfAnnotationType;
use mupdf::{Point, Quad, Rect};
use vpdf::Result;
use vpdf::app::App;
use vpdf::solo::Solo;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Mode;
use vpdf_ui::input::InputOwner;
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::Destination;

fn annotated() -> Result<(tempfile::NamedTempFile, String)> {
  let mut source =
    vpdf_pdf::document::Document::open(&corpus("paper.pdf")?, "")?;
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
  crate::common::cells(120, 40)
}

fn position(app: &App, index: usize) -> Position {
  match &app.panel.as_ref().unwrap().view.rows()[index].destination {
    Destination::Annotation { position, .. } => *position,
    _ => panic!("Annotation destination"),
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_lists_marked_text_and_notes() -> Result<()> {
  let (file, words) = annotated()?;
  let mut app = Solo::open(file.path(), "", false, screen())?;
  app.open_panel(PanelKind::Annotations)?;
  crate::common::annotations_loaded(&mut app)?;
  let rows = &app.panel.as_ref().unwrap().view.rows();
  // A page heading, then marked text or the note.
  let labels: Vec<_> =
    rows.iter().take(3).map(|row| row.label.as_str()).collect();
  assert_eq!(labels, ["p2", words.as_str(), "margin note"]);
  assert_eq!(rows.len(), 6);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn enter_positions_cursor_and_note() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    for entry in [0, 1, 2] {
      let mut app = Solo::open(file.path(), "", text, screen())?;
      if entry > 0 {
        keys(&mut app, b"i")?;
      }
      if entry > 1 {
        keys(&mut app, b"v")?;
      }
      app.open_panel(PanelKind::Annotations)?;
      crate::common::annotations_loaded(&mut app)?;
      // The first annotation, under its page heading.
      let expected = position(&app, 1);
      keys(&mut app, b"\r")?;
      assert_eq!(app.reader.cursor, expected);
      assert_eq!(app.reader.target(), Some(expected));
      assert_eq!(app.reader.mode(), Mode::Cursor);
      assert_eq!(app.input_owner(), InputOwner::Peek);
      assert!(!app.panel_focused());
      assert_eq!(app.panel.as_ref().unwrap().view.rows().len(), 6);
      assert!(drawn(&mut app)?.contains("check this"));
      keys(&mut app, b"\x1b")?;
      assert!(app.overlays.preview().is_none());
      assert_eq!(app.reader.mode(), Mode::Cursor);
      keys(&mut app, b"K")?;
      assert!(drawn(&mut app)?.contains("check this"));
      let jumps = app.reader.jumps().count();
      keys(&mut app, b"\r")?;
      assert!(app.overlays.preview().is_none());
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
    let mut app = Solo::open(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    crate::common::annotations_loaded(&mut app)?;
    assert_eq!(position(&app, 2), position(&app, 3));
    let original = app.reader.cursor;
    app.panel.as_mut().unwrap().view.choose(3);
    keys(&mut app, b"K")?;
    // An off-screen source keeps it (U17).
    let shown = app.layout.visible(app.reader.scroll);
    assert!(!shown.contains(&position(&app, 3).page.0), "off screen");
    app.drop_stray_preview();
    let out = drawn(&mut app)?;
    assert!(out.contains("another note"));
    assert!(!out.contains("margin note"));
    assert_eq!(app.reader.cursor, original);
    assert!(app.panel_focused());
    keys(&mut app, b"\x1b")?;
    assert_eq!(app.input_owner(), InputOwner::Panel);
    let expected = position(&app, 3);
    keys(&mut app, b"\r")?;
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
    let mut app = Solo::open(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    crate::common::annotations_loaded(&mut app)?;
    app.panel.as_mut().unwrap().view.choose(2);
    keys(&mut app, b"\r\x1b")?;
    let cursor = app.reader.cursor;
    keys(&mut app, b"v")?;
    let selection = app.reader.selection.unwrap();
    keys(&mut app, b"K")?;
    assert!(drawn(&mut app)?.contains("margin note"));
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, selection.anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(selection.kind));
    keys(&mut app, b"\x1b")?;
    assert_eq!(app.reader.mode(), Mode::Visual(selection.kind));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn notes_scroll_and_resize() -> Result<()> {
  let (file, _) = annotated()?;
  for text in [false, true] {
    let mut app = Solo::open(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    crate::common::annotations_loaded(&mut app)?;
    app.panel.as_mut().unwrap().view.choose(4);
    keys(&mut app, b"\r")?;
    let cursor = app.reader.cursor;
    let first = drawn(&mut app)?;
    assert!(first.contains("BEGIN"));
    assert!(first.contains('↓'));
    if !text {
      let [r, g, b] = vpdf_model::theme::Colors::Terminal.palette().background;
      assert!(first.contains(&format!("48;2;{r};{g};{b}m")));
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
    let label = narrow.split_once('┌').unwrap().1.split('┐').next().unwrap();
    assert!(
      label.chars().count() + 2 <= usize::from(app.layout.screen().columns)
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
    let mut app = Solo::open(file.path(), "", text, screen())?;
    app.open_panel(PanelKind::Annotations)?;
    crate::common::annotations_loaded(&mut app)?;
    app.panel.as_mut().unwrap().view.choose(5);
    let expected = position(&app, 5);
    keys(&mut app, b"\r")?;
    assert_eq!(app.reader.cursor, expected);
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert!(app.overlays.preview().is_none());
    keys(&mut app, b"\t")?;
    app.panel.as_mut().unwrap().view.choose(2);
    app.action(Input::plain(Action::Swap))?;
    assert!(app.panel_focused());
    assert!(app.overlays.preview().is_none());
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
       /QuadPoints {} /C [0 0 0 0.5] >>",
      quads(189)
    ),
    format!(
      "<< /Type /Annot /Subtype /Highlight /Rect [20 141 230 165] \
       /QuadPoints {} /Contents (check this note please) /T (Ada) \
       /C [1 0.92 0.23] >>",
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

fn noted_session(
  directory: &std::path::Path,
) -> Result<vpdf::session::Session> {
  let path = directory.join("noted.pdf");
  noted_pdf(&path)?;
  crate::harness::session(&path, screen())
}

#[test]
fn hover_details_notes_rows_and_pages() -> Result<()> {
  use vpdf::session::Session;
  use vpdf::trace::Drawable;
  use vpdf_model::document::{Heading, Target};
  use vpdf_model::input::MouseEventKind;
  use vpdf_ui::pointer::{Hit, Region};
  let directory = tempfile::tempdir()?;
  let mut session = noted_session(directory.path())?;
  let on_page = |session: &mut Session, wanted: fn(Hit) -> bool| {
    cell_where(session.app(), Region::Page, 0..39, wanted)
  };
  let note =
    on_page(&mut session, |hit| matches!(hit, Hit::Note { text: true, .. }))?;
  let margin =
    on_page(&mut session, |hit| matches!(hit, Hit::Note { text: false, .. }))?;
  let link = on_page(&mut session, |hit| matches!(hit, Hit::Link(..)))?;
  let blank = on_page(&mut session, |hit| hit == Hit::Nothing)?;
  // (cell) -> frame, status text
  let hover =
    |session: &mut Session, (column, row)| -> Result<(bool, String)> {
      let frame = mouse(session, MouseEventKind::Moved, (column, row))?;
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
  crate::common::annotations_loaded(session.app())?;
  let focused = session.app().panel_focused();
  // Row 1 is the page heading.
  let (_, drawn) = hover(&mut session, (1, 3))?;
  let app = session.app();
  assert_eq!(app.hover.hit(), Hit::Row(2));
  let row = &app.panel.as_ref().unwrap().view.rows()[2].label;
  assert!(drawn.contains(row.as_str()), "{row}");
  assert!(drawn.contains("\x1b[0;4m"), "the row underlines");
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), 1);
  assert_eq!(app.panel_focused(), focused);
  // A short row adds nothing.
  hover(&mut session, (1, 4))?;
  assert_eq!(session.app().panel.as_ref().unwrap().view.hover_title(), None);
  session.app().close_panel()?;
  // A panel opened under a resting pointer.
  session.app().open_panel(PanelKind::Annotations)?;
  crate::common::annotations_loaded(session.app())?;
  session.draw_frame(&mut Vec::new())?;
  if session.redraw_due() {
    session.draw_frame(&mut Vec::new())?;
  }
  assert!(!session.redraw_due(), "one follow-up at most");
  assert_eq!(session.app().hover.hit(), Hit::Row(3));
  session.app().close_panel()?;
  // Thumbnails show their page label and section.
  let app = session.app();
  app.doc.outline.push(Heading {
    title: "Opening".into(),
    depth: 0,
    target: Target::Page(PageId(0)),
  });
  app.open_overview()?;
  let thumbnail = cell_where(session.app(), Region::Overview, 0..39, |hit| {
    matches!(hit, Hit::Thumbnail(_))
  })?;
  let (_, drawn) = hover(&mut session, thumbnail)?;
  assert_eq!(session.app().hover.hit(), Hit::Thumbnail(PageId(0)));
  assert!(drawn.contains("p. 1 · Opening"));
  Ok(())
}

/// U22: clipped outline rows show without their indent.
#[test]
fn outline_hovers_drop_the_indent() -> Result<()> {
  use vpdf::trace::Drawable;
  use vpdf_model::document::{Heading, Target};
  use vpdf_model::input::MouseEventKind;
  use vpdf_ui::pointer::{Hit, Region};
  let directory = tempfile::tempdir()?;
  let mut session = noted_session(directory.path())?;
  let title = "Applications of attention in our model, ".repeat(2);
  let title = title.trim_end();
  let app = session.app();
  for (title, depth) in [("Opening", 0), (title, 2)] {
    let target = Target::Page(PageId(0));
    app.doc.outline.push(Heading { title: title.into(), depth, target });
  }
  app.open_panel(PanelKind::Outline)?;
  session.draw_focused(&mut Vec::new())?;
  let app = session.app();
  let (column, row) = (0..app.screen.columns)
    .find_map(|column| {
      let hit = app.hovered(Region::Panel, column, 2, None).ok();
      (hit == Some(Hit::Row(1))).then_some((column, 2))
    })
    .expect("the indented row");
  mouse(&mut session, MouseEventKind::Moved, (column, row))?;
  let mut out = Vec::new();
  session.draw_frame(&mut out)?;
  let drawn = String::from_utf8_lossy(&out);
  assert_eq!(session.app().hover.hit(), Hit::Row(1));
  assert!(drawn.contains(title), "{drawn}");
  assert!(!drawn.contains(&format!(" {title}")), "{drawn}");
  Ok(())
}

/// A left click at a cell, at `now`.
fn click_at(
  session: &mut vpdf::session::Session,
  (column, row): (u16, u16),
  now: std::time::Instant,
) -> Result<()> {
  use vpdf_model::input::{Button, KeyModifiers, Mouse, MouseKind, Spot};
  for kind in [MouseKind::Press(Button::LEFT), MouseKind::Release(Button::LEFT)]
  {
    let at = Spot::Cell { column, row };
    let mouse = Mouse { kind, at, modifiers: KeyModifiers::NONE };
    session.mouse_report_at(mouse, now)?;
  }
  Ok(())
}

/// The page cells of `screen`.
fn grid() -> impl Iterator<Item = (u16, u16)> {
  (0..39).flat_map(|row| (0..120).map(move |column| (column, row)))
}

/// Notes whose page cells act as icons.
fn icon_notes(app: &mut App) -> Vec<usize> {
  use vpdf_ui::pointer::{Hit, Region};
  grid()
    .filter_map(|(column, row)| {
      match app.hovered(Region::Page, column, row, None) {
        Ok(Hit::Note { index, text: false, .. }) => Some(index),
        _ => None,
      }
    })
    .collect()
}

/// Notes open by mouse (U11).
#[test]
fn notes_open_by_mouse() -> Result<()> {
  use vpdf_model::input::{MouseButton, MouseEventKind};
  use vpdf_terminal::terminal::Shape;
  use vpdf_ui::pointer::{Hit, Region};
  let directory = tempfile::tempdir()?;
  let mut session = noted_session(directory.path())?;
  let mut find = |wanted: fn(Hit) -> bool| {
    cell_where(session.app(), Region::Page, 0..39, wanted)
  };
  let icon = find(|hit| matches!(hit, Hit::Note { text: false, .. }))?;
  let marked = find(|hit| matches!(hit, Hit::Note { text: true, .. }))?;
  // Icons tint and take the pointer shape.
  mouse(&mut session, MouseEventKind::Moved, icon)?;
  let app = session.app();
  assert_eq!(app.hover.hit().shape(), Shape::Pointer);
  assert!(app.hover_icon().is_some(), "a tint");
  // Highlights stay text (H3).
  let app = session.app();
  let Hit::Note { index: sticky, .. } = app.hover.hit() else {
    panic!("an icon")
  };
  assert!(icon_notes(app).iter().all(|index| *index == sticky));
  // Clicks open the note, then close it.
  let target = session.app().reader.target();
  let start = std::time::Instant::now();
  let later = |ms| start + std::time::Duration::from_millis(ms);
  click_at(&mut session, icon, later(0))?;
  let frame = crate::common::focused_frame(&mut session)?;
  assert!(frame.contains("margin note"));
  let app = session.app();
  assert_eq!(app.input_owner(), InputOwner::Peek);
  assert_eq!(app.reader.target(), target, "the cursor stays");
  let area = app.preview_cells().expect("a drawn popup");
  let near = |row: u16| row.abs_diff(icon.1) <= 5;
  assert!(near(area.row) || near(area.row + area.rows), "{area:?} {icon:?}");
  // Past the double-click window, a click closes it.
  click_at(&mut session, icon, later(1000))?;
  assert!(session.app().overlays.preview().is_none());
  // The menu's Show note sits by the icon too.
  crate::mouse::tap(&mut session, MouseButton::Right, icon)?;
  crate::mouse::choose(&mut session, "Show note")?;
  crate::common::focused_frame(&mut session)?;
  let app = session.app();
  let area = app.preview_cells().expect("a drawn popup");
  assert!(near(area.row) || near(area.row + area.rows), "{area:?} {icon:?}");
  app.overlays.close_preview();
  // Highlight clicks and icon drags select.
  crate::mouse::tap(&mut session, MouseButton::Left, marked)?;
  assert!(session.app().overlays.preview().is_none(), "a highlight click");
  drag(&mut session, icon, marked)?;
  let app = session.app();
  assert!(
    app.overlays.preview().is_none() && app.reader.selection.is_some(),
    "a drag"
  );
  app.reader.cancel();
  // (menu item) -> note shown, register
  let cases = [("Show note", true, false), ("Copy comment", false, true)];
  for (label, shown, copied) in cases {
    crate::mouse::tap(&mut session, MouseButton::Right, marked)?;
    let app = session.app();
    let menu = app.overlays.menu().expect("a menu");
    let index = menu.entries().iter().position(|item| item.label == label);
    let shortcuts: Vec<_> =
      menu.entries().iter().map(|item| item.key.as_str()).collect();
    assert_eq!(
      index.map(|index| shortcuts[index]),
      Some(if shown { "K" } else { "" })
    );
    app.overlays.menu_mut().unwrap().select(index.expect(label));
    keys(app, b"\r")?;
    let comment = "check this note please";
    let frame = crate::common::focused_frame(&mut session)?;
    assert_eq!(frame.contains(comment), shown, "{label}");
    let app = session.app();
    let register = app.reader.register('"').map(|text| &**text);
    if copied {
      assert_eq!(register, Some(comment), "{label}");
      assert_eq!(app.reader.message, "Copied comment");
    }
    app.overlays.close_preview();
  }
  Ok(())
}

/// A sticky note over text, a commented box.
fn shapes_pdf(path: &std::path::Path) -> Result<()> {
  let content = "BT /F1 18 Tf 24 TL 20 170 Td (Sticky over this line) Tj T* \
                 (Boxed second line) Tj ET";
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
     /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R \
     /Annots [6 0 R 7 0 R] >>"
      .to_owned(),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    "<< /Type /Annot /Subtype /Square /Rect [15 138 230 166] \
     /Contents (box comment) >>"
      .to_owned(),
    "<< /Type /Annot /Subtype /Text /Rect [60 165 80 185] \
     /Contents (sticky comment) >>"
      .to_owned(),
  ];
  crate::common::write_pdf(path, &objects, "")
}

/// Icons by kind; double clicks; scrolling closes (U11).
#[test]
fn note_icons_by_kind() -> Result<()> {
  use vpdf::trace::Drawable;
  use vpdf_ui::pointer::{Hit, Region};
  let (_directory, path) = crate::common::temp_pdf(shapes_pdf)?;
  for text_mode in [false, true] {
    let mut session = crate::harness::session(&path, screen())?;
    if text_mode {
      session.app().command("text")?;
    }
    let app = session.app();
    let hit = |app: &mut App, kind: &str| {
      grid().find_map(|(column, row)| {
        let hit = app.hovered(Region::Page, column, row, None).ok()?;
        let Hit::Note { page, index, text } = hit else { return None };
        let notes = app.doc.annotations(page).ok()?;
        (notes[index].kind == kind).then_some(((column, row), text))
      })
    };
    // (kind) -> an icon in the PDF view
    for (kind, icon) in [("Square", false), ("Note", !text_mode)] {
      let (_, text) = hit(app, kind).expect(kind);
      assert_eq!(!text, icon, "text={text_mode} {kind}");
    }
    let (sticky, _) = hit(app, "Note").expect("a sticky note");
    let (boxed, _) = hit(app, "Square").expect("a box");
    let start = std::time::Instant::now();
    let later = |ms| start + std::time::Duration::from_millis(ms);
    click_at(&mut session, boxed, later(0))?;
    let app = session.app();
    assert!(app.overlays.preview().is_none(), "text={text_mode}: a box click");
    let line = app.reader.target().map(|target| target.line);
    assert_eq!(line, Some(1), "text={text_mode}: the box click targets");
    // A double click keeps the note open.
    click_at(&mut session, sticky, later(1000))?;
    click_at(&mut session, sticky, later(1100))?;
    let open = session.app().overlays.preview().is_some();
    assert_eq!(open, !text_mode, "text={text_mode}: a double click");
    if text_mode {
      continue;
    }
    // Scrolling the icon away closes it (U17).
    session.app().reader.seed_reader(|reader| reader.scroll += 400.0);
    session.draw_frame(&mut Vec::new())?;
    assert!(session.app().overlays.preview().is_none(), "scrolled away");
  }
  Ok(())
}

#[test]
fn hover_detail_yields_to_messages_and_caches() -> Result<()> {
  let (_directory, path) = crate::common::temp_pdf(noted_pdf)?;
  // (error, message, covered) -> detail shown
  let cases = [
    (None, "", false, true),
    (Some("Failed"), "", false, false),
    (None, "Copied", false, false),
    (None, "Copied", true, true),
    (Some("Failed"), "", true, false),
  ];
  for (error, message, covered, shown) in cases {
    let mut app = Solo::open(&path, "", false, screen())?;
    if covered {
      app.show_recent(&[])?;
    }
    app.error = error.map(str::to_owned);
    app.reader.notice(message);
    let mut out = Vec::new();
    app.status(&mut out, 39, Some("DETAIL"))?;
    let drawn = String::from_utf8_lossy(&out);
    let at = format!("{error:?} {message:?} covered={covered}");
    assert_eq!(drawn.contains("DETAIL"), shown, "{at}");
  }
  // Trimming drops the annotation cache.
  let mut doc = vpdf_pdf::document::Document::open(&path, "")?;
  let first = doc.annotations(PageId(0))?;
  assert!(std::sync::Arc::ptr_eq(&first, &doc.annotations(PageId(0))?));
  doc.trim();
  let again = doc.annotations(PageId(0))?;
  assert!(!std::sync::Arc::ptr_eq(&first, &again));
  assert_eq!(again.len(), 3);
  Ok(())
}

/// Note rows' menus show and run `K`.
#[test]
fn note_row_menus_show_notes() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let mut session = noted_session(directory.path())?;
  session.app().open_panel(PanelKind::Annotations)?;
  crate::common::annotations_loaded(session.app())?;
  session.draw_focused(&mut Vec::new())?;
  // Row 1 is the page heading; row 2 the unnoted highlight.
  crate::mouse::right(&mut session, (1, 3))?;
  let app = session.app();
  let menu = app.overlays.menu_mut().expect("a menu");
  let index = menu.entries().iter().position(|item| item.label == "Show note");
  let index = index.expect("Show note");
  assert_eq!(menu.entries()[index].key, "K");
  menu.select(index);
  assert!(app.overlays.preview().is_none());
  keys(app, b"\r")?;
  assert!(app.overlays.menu().is_none() && app.overlays.preview().is_some());
  Ok(())
}

/// A PDF without annotations says so once its scan ends.
#[test]
fn annotations_panel_explains_empty() -> Result<()> {
  let (_directory, mut app) = crate::common::temp_solo(
    |path| crate::common::text_pdf(path, &["Plain text"]),
    false,
    crate::common::cells(100, 20),
  )?;
  app.open_panel(PanelKind::Annotations)?;
  crate::common::annotations_loaded(&mut app)?;
  let panel = &mut app.panel.as_mut().unwrap().view;
  assert!(panel.rows().is_empty());
  let mut scene = Vec::new();
  panel.scene_list(&mut scene, vpdf_model::geometry::At::default(), 40, 6);
  let text: String = scene
    .iter()
    .filter_map(|op| match op {
      vpdf_model::scene::Op::Text(text) => Some(text.as_str()),
      _ => None,
    })
    .collect();
  assert!(text.contains("No annotations in this PDF."), "{text}");
  Ok(())
}

/// Highlights quote their text with a swatch in the stroke color.
#[test]
fn highlight_rows_quote_marked_text() -> Result<()> {
  use vpdf::trace::Drawable;
  use vpdf_ui::panel::{Destination, RowKind as K};
  let directory = tempfile::tempdir()?;
  let mut session = noted_session(directory.path())?;
  let app = session.app();
  app.open_panel(PanelKind::Annotations)?;
  crate::common::annotations_loaded(app)?;
  let rows = app.panel.as_ref().unwrap().view.rows();
  let kinds: Vec<_> = rows.iter().map(|row| (row.depth, row.kind)).collect();
  assert_eq!(
    kinds,
    [(0, K::Plain), (1, K::Highlight), (1, K::Highlight), (1, K::Note)]
  );
  assert_eq!(rows[3].label, "margin note");
  let colors: Vec<_> = rows[1..3]
    .iter()
    .map(|row| match &row.destination {
      Destination::Annotation { annotation, .. } => annotation.color,
      _ => None,
    })
    .collect();
  assert_eq!(colors, [Some([128, 128, 128]), Some([255, 235, 59])]);
  assert!(!rows[2].label.contains("check this note"), "{}", rows[2].label);
  // The unselected highlight's swatch reaches the terminal.
  let mut out = Vec::new();
  session.draw_frame(&mut out)?;
  let drawn = String::from_utf8_lossy(&out);
  assert!(drawn.contains("\x1b[38;2;255;235;59m▇"), "{drawn}");
  Ok(())
}
