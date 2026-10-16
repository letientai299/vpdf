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
  app.command("PdfPage 39")?;
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
  app.command("PdfReload")?;
  assert!(app.reader.last_selection.is_none());
  let mut output = Vec::new();
  app.raster_updates(&mut output)?;
  assert!(!app.reset_images);
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
  app.command("PdfPage 39")?;
  assert!(app.animation.is_none());
  assert_eq!(app.reader.cursor.page, PageId(38));
  Ok(())
}
