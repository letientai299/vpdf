use std::path::PathBuf;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::Target;
use vpdf::keys::{Action, Input, PanelKind};
use vpdf::layout::Screen;
use vpdf::text::PageId;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn reader_workflows() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  app.command("page 39")?;
  assert_eq!(app.reader.cursor.page, PageId(38));
  app.open_panel(PanelKind::Outline)?;
  assert!(!app.panel.as_ref().unwrap().rows.is_empty());
  app.close_panel()?;
  app.open_panel(PanelKind::Text)?;
  assert!(app.text_mode);
  app.action(Input {
    action: Action::Quit,
    count: 1,
    explicit: false,
    register: '"',
  })?;
  assert!(!app.text_mode && !app.quit);
  app.start_search("python", false, false)?;
  let start = Instant::now();
  while !matches!(app.search.phase, vpdf::find::Phase::Complete) {
    assert!(start.elapsed() < Duration::from_secs(15));
    app.search_updates()?;
    std::thread::sleep(Duration::from_millis(1));
  }
  assert!(!app.search.hits.is_empty());
  app.next_result(true, 2)?;
  app.open_panel(PanelKind::Results)?;
  assert_eq!(app.panel.as_ref().unwrap().rows.len(), app.search.hits.len());
  app.action(Input {
    action: Action::Cancel,
    count: 1,
    explicit: false,
    register: '"',
  })?;
  assert!(app.search.hidden);
  app.follow(&Target::Page(PageId(usize::MAX)))?;
  assert_eq!(app.reader.cursor.page.0, app.doc.pages.len() - 1);
  assert!(app.follow(&Target::Uri("file:///etc/passwd".into())).is_err());
  app.reader.visual(vpdf::reader::SelectionKind::Character);
  app.command("reload")?;
  assert!(app.reader.last_selection.is_none());
  let mut output = Vec::new();
  app.raster_updates(&mut output)?;
  assert_eq!(app.graphics.bytes(), 0);
  assert!(app.search.hits.is_empty());
  app.reader.history(false);
  assert!(app.reader.cursor.page.0 < app.doc.pages.len());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn scrolling_converges_and_jumps_interrupt() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  app.scroll(100.0)?;
  app.scroll(100.0)?;
  assert_eq!(app.animation.as_ref().unwrap().target, 200.0);
  assert_eq!(app.reader.scroll, 0.0);
  for _ in 0..30 {
    if let Some(animation) = &mut app.animation {
      animation.updated = Instant::now() - Duration::from_millis(16);
    }
    let previous = app.reader.scroll;
    app.animate();
    assert!(app.reader.scroll >= previous && app.reader.scroll <= 200.0);
  }
  assert!(app.animation.is_none());
  assert_eq!(app.reader.scroll, 200.0);
  for action in [Action::Cancel, Action::History(false), Action::Align(0.0)] {
    app.scroll(100.0)?;
    app.action(Input { action, count: 1, explicit: false, register: '"' })?;
    assert!(app.animation.is_none());
    let stopped = app.reader.scroll;
    assert!(!app.animate());
    assert_eq!(app.reader.scroll, stopped);
  }
  app.scroll(100.0)?;
  app.command("page 39")?;
  assert!(app.animation.is_none());
  assert_eq!(app.reader.cursor.page, PageId(38));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn links_keep_coordinates_and_text_scrolls_lines() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let target = (0..app.doc.pages.len())
    .find_map(|page| {
      app.doc.links(PageId(page)).ok()?.into_iter().find(
        |link| matches!(link.target, Target::Point { y, .. } if y > 100.0),
      )
    })
    .expect("real book has positioned internal links")
    .target;
  let Target::Point { page, x, y } = target else { unreachable!() };
  let expected = app.doc.text(page)?.nearest(page, x, y);
  app.follow(&target)?;
  assert_eq!(app.reader.cursor, expected);
  app.command("page 39")?;
  app.text_mode = true;
  app.scroll(48.0)?;
  assert_eq!(app.reader.cursor.line, 3);
  assert!(app.animation.is_none());
  let position = app.text_position(2, 1)?;
  assert_eq!(position.line, 1);
  assert_eq!(position.glyph, 2);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn changed_file_reloads_after_settling() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("live.pdf");
  std::fs::copy(root.join("paper.pdf"), &path)?;
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&path, "", false, screen)?;
  let pages = app.doc.pages.len();
  app.command("page 3")?;
  assert!(!app.watch()?);
  std::fs::copy(root.join("book.pdf"), &path)?;
  assert!(!app.watch()?, "a change waits for the write to settle");
  assert!(!app.watch()?);
  std::thread::sleep(std::time::Duration::from_millis(120));
  assert!(app.watch()?);
  assert_ne!(app.doc.pages.len(), pages);
  assert_eq!(app.reader.cursor.page, PageId(2));
  assert!(!app.watch()?);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn link_hints_follow_typed_labels() -> Result<()> {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
  app.command("page 2")?;
  app.start_hints(false)?;
  let hints = app.hints.as_ref().unwrap();
  assert!(hints.items.len() > 3);
  let label = hints.items[0].label.clone();
  for ch in label.chars() {
    app.hint_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
  }
  assert!(app.hints.is_none());
  assert!(app.reader.cursor.page.0 > 1, "citation jumps to the references");
  app.start_hints(false)?;
  let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
  app.hint_key(escape)?;
  assert!(app.hints.is_none());
  Ok(())
}
