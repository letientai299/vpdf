use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::App;
use vpdf::keys::PanelKind;
use vpdf::layout::Screen;
use vpdf::panel::Destination;
use vpdf::prompt::History;
use vpdf::reader::Reader;
use vpdf::text::{PageId, Position};

#[test]
fn jumps_include_both_history_directions() {
  let mut reader = Reader::default();
  assert_eq!(reader.jumps().count(), 0);
  for page in 0..3 {
    reader.cursor.page = PageId(page);
    reader.remember();
  }
  reader.cursor.page = PageId(3);
  for (forward, expected) in [
    (false, vec![0, 1, 3]),
    (false, vec![0, 2, 3]),
    (false, vec![1, 2, 3]),
    (true, vec![0, 2, 3]),
    (true, vec![0, 1, 3]),
    (true, vec![0, 1, 2]),
  ] {
    reader.history(forward);
    assert_eq!(reader.jumps().map(|p| p.page.0).collect::<Vec<_>>(), expected);
  }
  reader.clear_history();
  assert_eq!(reader.jumps().count(), 0);
}

fn command(app: &mut App, history: &mut History, text: &str) -> Result<()> {
  for code in std::iter::once(KeyCode::Char(':'))
    .chain(text.chars().map(KeyCode::Char))
    .chain(std::iter::once(KeyCode::Enter))
  {
    app.key(KeyEvent::new(code, KeyModifiers::NONE), history)?;
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn jumps_panel_empty_and_recorded_destinations() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let mut history = History::default();
  command(&mut app, &mut history, "jumps")?;
  let panel = app.panel.as_ref().unwrap();
  assert_eq!(panel.kind, PanelKind::Jumps);
  assert!(panel.rows.is_empty());
  let origin = app.reader.cursor;
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  assert_eq!(app.reader.cursor, origin);
  app.close_panel()?;

  let mut positions = vec![origin];
  for page in [9, 19] {
    app.go_to(Position { page: PageId(page), line: 2, glyph: 3 }, true)?;
    positions.push(app.reader.cursor);
  }
  for (forward, expected) in [(false, [0, 2]), (false, [1, 2]), (true, [0, 2])]
  {
    let code = KeyCode::Char(if forward { 'i' } else { 'o' });
    app.key(KeyEvent::new(code, KeyModifiers::CONTROL), &mut history)?;
    command(&mut app, &mut history, "jumps")?;
    let rows = &app.panel.as_ref().unwrap().rows;
    assert_eq!(rows.len(), expected.len());
    for (row, index) in rows.iter().zip(expected) {
      let Destination::Jump { position, .. } = row.destination else {
        panic!("Jump destination");
      };
      assert_eq!(position, positions[index]);
      assert!(row.detail.contains(&format!(
        "L{} C{}",
        position.line + 1,
        position.glyph + 1
      )));
      let text = app.doc.text(position.page)?;
      let line = text.lines.get(position.line).unwrap();
      assert_eq!(
        row.label,
        line.text().split_whitespace().collect::<Vec<_>>().join(" ")
      );
    }
    app.close_panel()?;
  }
  command(&mut app, &mut history, "jumps")?;
  let origin = app.reader.cursor;
  app.panel.as_mut().unwrap().selected = 1;
  // History changes must preserve recorded destinations.
  app.reader.clear_history();
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  assert_eq!(app.reader.cursor, positions[2]);
  assert!(app.panel.is_some() && !app.panel_focused());
  app.key(
    KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
    &mut history,
  )?;
  assert_eq!(app.reader.cursor, origin);
  Ok(())
}

fn text_app(lines: &[&str]) -> Result<(tempfile::TempDir, App)> {
  let dir = tempfile::tempdir()?;
  let path = dir.path().join("jumps.pdf");
  super::common::text_pdf(&path, lines)?;
  let mut app = App::with_screen(
    &path,
    "",
    true,
    Screen {
      columns: 120,
      rows: 50,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  )?;
  super::common::outline_loaded(&mut app)?;
  app.doc.outline.clear();
  Ok((dir, app))
}

fn at(line: usize, glyph: usize) -> Position {
  Position { page: PageId(0), line, glyph }
}

fn history_key(app: &mut App, forward: bool) -> Result<()> {
  app.key(
    KeyEvent::new(
      KeyCode::Char(if forward { 'i' } else { 'o' }),
      KeyModifiers::CONTROL,
    ),
    &mut History::default(),
  )?;
  Ok(())
}

#[test]
fn fallback_context() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "na\\357ve words,"])?;
  app.doc.pages[0].label = "iv".into();
  for (glyph, word) in
    [(0, "“naïve”"), (5, "whitespace"), (6, "“words”"), (11, "“,”")]
  {
    app.go_to(at(1, glyph), true)?;
    app.reader.clear_history();
    app.go_to(at(0, 0), true)?;
    let rows = app.panel_rows(PanelKind::Jumps)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "naïve words,");
    assert_eq!(
      rows[0].detail,
      format!("p. iv (#1) · L2 C{} · word {word}", glyph + 1)
    );
  }
  let (_dir, mut app) = text_app(&[""])?;
  app.reader.remember();
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows[0].label, "No text");
  assert_eq!(rows[0].detail, "p. 1 · L1 C1 · word —");
  Ok(())
}

fn heading(app: &mut App, line: usize, title: &str) -> Result<()> {
  use vpdf::document::{Heading, Target};
  let bounds = app.doc.text(PageId(0))?.lines[line].glyphs[0].bounds;
  let (x, y) = bounds.centre();
  app.doc.outline.push(Heading {
    title: title.into(),
    depth: 0,
    target: Target::Point { page: PageId(0), x, y },
  });
  Ok(())
}

#[test]
fn semantic_history() -> Result<()> {
  let (_dir, mut app) = text_app(&[
    "Opening",
    "First section",
    "Second section",
    "needle target",
    "End",
  ])?;
  heading(&mut app, 1, "First section")?;
  heading(&mut app, 2, "Second section")?;
  let mut history = History::default();
  app.open_panel(PanelKind::Outline)?;
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  app.close_panel()?;
  for _ in 0..2 {
    app.key(
      KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
      &mut history,
    )?;
  }
  app.start_search("needle", false, false)?;
  super::common::search_loaded(&mut app)?;
  app.go_to(at(4, 0), true)?;
  let labels = |app: &mut App| -> Result<Vec<String>> {
    Ok(
      app
        .panel_rows(PanelKind::Jumps)?
        .into_iter()
        .map(|row| row.label)
        .collect(),
    )
  };
  let expected = vec![
    "Opening",
    "Heading: First section",
    "Heading: Second section",
    "Search [1/1] /needle/",
  ];
  assert_eq!(labels(&mut app)?, expected);
  // Later searches cannot relabel earlier visits.
  app.search.query.text = "other".into();
  history_key(&mut app, false)?;
  history_key(&mut app, true)?;
  assert_eq!(labels(&mut app)?, expected);
  app.open_panel(PanelKind::Jumps)?;
  app.panel.as_mut().unwrap().selected = 3;
  app.reader.clear_history();
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  assert_eq!(app.reader.cursor, at(3, 0));
  app.close_panel()?;
  app.go_to(at(4, 0), true)?;
  assert_eq!(labels(&mut app)?.last().unwrap(), "Search [1/1] /needle/");
  Ok(())
}

#[test]
fn committed_searches() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "needle one", "needle two"])?;
  app.begin_search(false)?;
  app.incremental("needle")?;
  super::common::search_loaded(&mut app)?;
  assert_eq!(app.reader.jumps().count(), 0);
  app.finish_search("needle", false)?;
  app.prompt = None;
  app.next_result(false, 1)?;
  app.go_to(at(0, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  let labels: Vec<_> = rows
    .iter()
    .filter(|row| row.label.starts_with("Search ["))
    .map(|row| row.label.as_str())
    .collect();
  assert_eq!(labels, ["Search [1/2] /needle/", "Search [2/2] /needle/"]);
  app.open_panel(PanelKind::Results)?;
  app.panel.as_mut().unwrap().selected = 1;
  app.key(
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    &mut History::default(),
  )?;
  app.close_panel()?;
  app.go_to(at(0, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows.last().unwrap().label, "Search [2/2] /needle/");
  app.start_search("needle", true, false)?;
  super::common::search_loaded(&mut app)?;
  app.go_to(at(0, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows.last().unwrap().label, "Search [2/2] ?needle?");
  Ok(())
}

#[test]
fn followed_links() -> Result<()> {
  use vpdf::document::Target;
  let (_dir, mut app) = text_app(&["Opening", "Reference title", "End"])?;
  let bounds = app.doc.text(PageId(0))?.lines[1].glyphs[0].bounds;
  let (x, y) = bounds.centre();
  app.follow(&Target::Point { page: PageId(0), x, y })?;
  app.go_to(at(2, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert!(rows[1].label.starts_with("Link: Reference title"));
  assert!(matches!(rows[1].destination,
    Destination::Jump { position, .. } if position == at(1, 0)));
  Ok(())
}

#[test]
fn outline_preserves_selected_title() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "Shared destination", "End"])?;
  heading(&mut app, 1, "Chapter")?;
  heading(&mut app, 1, "Section")?;
  app.open_panel(PanelKind::Outline)?;
  app.panel.as_mut().unwrap().selected = 1;
  app.key(
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    &mut History::default(),
  )?;
  app.close_panel()?;
  app.go_to(at(2, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows.last().unwrap().label, "Heading: Section");
  Ok(())
}
