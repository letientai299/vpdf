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
      let Destination::Position(position) = row.destination else {
        panic!("Jump destination");
      };
      assert_eq!(position, positions[index]);
      assert_eq!(row.detail, app.doc.pages[position.page.0].label);
      assert_eq!(
        row.label,
        format!("Line {}, column {}", position.line + 1, position.glyph + 1)
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
  assert!(app.panel.is_none());
  app.key(
    KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
    &mut history,
  )?;
  assert_eq!(app.reader.cursor, origin);
  Ok(())
}
