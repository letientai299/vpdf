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

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_shows_citation_target() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  for text_mode in [false, true] {
    let mut app =
      App::with_screen(&root.join("paper.pdf"), "", text_mode, screen)?;
    let links = app.doc.links(PageId(1))?;
    let link =
      links.iter().find(|link| !matches!(link.target, Target::Uri(_))).unwrap();
    let (x, y) = (
      (link.bounds.x0 + link.bounds.x1) / 2.0,
      (link.bounds.y0 + link.bounds.y1) / 2.0,
    );
    app.reader.cursor = app.doc.text(PageId(1))?.nearest(PageId(1), x, y);
    app.ensure_cursor()?;
    app.preview_link()?;
    assert!(app.peek.is_some());
    let mut out = Vec::new();
    app.draw(&mut out)?;
    let drawn = String::from_utf8_lossy(&out);
    assert_eq!(drawn.contains("z=2"), !text_mode);
    assert_eq!(drawn.contains('╭'), text_mode);
    assert_eq!(app.reader.cursor.page, PageId(1), "preview does not move");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn overview_places_thumbnails_and_opens_pages() -> Result<()> {
  use vpdf::keys::{Action, Input};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 137,
    rows: 36,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let press = |app: &mut App, action| {
    app.action(Input { action, count: 1, explicit: false, register: '"' })
  };
  press(&mut app, Action::Overview)?;
  assert!(app.overview.is_some());
  let mut out = Vec::new();
  app.draw(&mut out)?;
  let started = Instant::now();
  while app.graphics.bytes() == 0 {
    assert!(started.elapsed() < Duration::from_secs(10), "thumbnails render");
    app.raster_updates(&mut out)?;
    std::thread::sleep(Duration::from_millis(5));
  }
  press(&mut app, Action::Motion(vpdf::motion::Motion::Down))?;
  press(&mut app, Action::Follow)?;
  assert!(app.overview.is_none());
  assert!(app.reader.cursor.page.0 > 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn unfocused_side_is_dimmed() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
  app.open_panel(PanelKind::Outline)?;
  let mut out = Vec::new();
  app.draw(&mut out)?;
  let veil = format!(",c={},r=39", 120 - app.left);
  assert!(String::from_utf8_lossy(&out).contains(&veil), "PDF dims");
  app.panel.as_mut().unwrap().focused = false;
  out.clear();
  app.draw(&mut out)?;
  let drawn = String::from_utf8_lossy(&out);
  assert!(!drawn.contains(&veil) && drawn.contains("\x1b[0;2m"), "panel dims");
  Ok(())
}
