use super::*;
use crate::SeedReader;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vpdf_model::document::Target;
use vpdf_model::text::{PageId, Text};
use vpdf_pdf::content_cache::Record;
use vpdf_ui::panel::{Destination, Row};

fn app() -> Result<(tempfile::TempDir, App)> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["https://example.org", "second page"], "")?;
  let app = App::with_screen(&path, "", false, crate::screen::cells(120, 40))?;
  Ok((root, app))
}

fn loaded(app: &mut App, kind: PanelKind) -> Result<()> {
  let start = Instant::now();
  while app.sidebar_pending(kind) {
    app.sidebar_updates()?;
    anyhow::ensure!(
      start.elapsed() < Duration::from_secs(5),
      "Sidebar timeout"
    );
    std::thread::sleep(Duration::from_millis(1));
  }
  Ok(())
}

#[test]
fn opening_is_cancelable() -> Result<()> {
  let (_root, mut app) = app()?;
  app.open_panel(PanelKind::Links)?;
  assert!(app.sidebar_loading(PanelKind::Links));
  assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  let (send, stopped) = mpsc::channel();
  app.sidebar.links = Load::Running(OneShot::spawn(&app.wake, move |stop| {
    while !stop.load(Ordering::Relaxed) {
      std::thread::yield_now();
    }
    send.send(stop.load(Ordering::Relaxed)).unwrap();
    None
  }));
  app.cancel_sidebars();
  assert!(stopped.recv_timeout(Duration::from_secs(5))?);
  app.prepare_sidebars();
  assert!(!app.sidebar_pending(PanelKind::Links));
  assert!(!app.sidebar_pending(PanelKind::Annotations));
  app.reader.seed_reader(vpdf_reader::reader::Reader::enter_cursor);
  app.go_to_page(PageId(1))?;
  assert!(!app.sidebar_pending(PanelKind::Links));
  app.go_to_page(PageId(0))?;
  app.open_panel(PanelKind::Links)?;
  loaded(&mut app, PanelKind::Links)?;
  assert_eq!(app.panel.as_ref().unwrap().view.rows().len(), 1);
  app.panel.as_mut().unwrap().view.focus(false);
  app.reader.seed_reader(vpdf_reader::reader::Reader::enter_cursor);
  app.go_to_page(PageId(1))?;
  assert_eq!(app.navigation_position().page, PageId(1));
  app.open_panel(PanelKind::Links)?;
  assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  loaded(&mut app, PanelKind::Links)?;
  assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  Ok(())
}

#[test]
fn reload_discards_old_rows() -> Result<()> {
  let (root, mut app) = app()?;
  app.open_panel(PanelKind::Links)?;
  loaded(&mut app, PanelKind::Links)?;
  assert_eq!(app.sidebar_rows(PanelKind::Links).len(), 1);
  let replacement = root.path().join("replacement.pdf");
  crate::fixtures::pdf(&replacement, &["no links", "second page"], "")?;
  std::fs::rename(replacement, &app.path)?;
  app.reload_with("")?;
  assert!(app.sidebar_loading(PanelKind::Links));
  assert!(app.sidebar_rows(PanelKind::Links).is_empty());
  loaded(&mut app, PanelKind::Links)?;
  assert!(app.sidebar_rows(PanelKind::Links).is_empty());
  Ok(())
}

#[test]
fn cached_rows_skip_native() -> Result<()> {
  let root = tempfile::tempdir()?;
  let source = root.path().join("source.pdf");
  std::fs::write(&source, b"not a native PDF")?;
  let request = Request::Links(PageId(0));
  let cache = Record::open(
    root.path().into(),
    &source,
    "",
    Kind::Links,
    &request.code(),
    || true,
  )
  .unwrap();
  cache.write(
    br#"[1,[{
    "label":"example","detail":"",
    "content":{"Link":{"Uri":"https://example.org"}}
  }]]"#,
  )?;
  let rows = worker::scan(
    &source,
    "",
    1,
    request,
    Vec::new(),
    &AtomicBool::new(false),
    Some(root.path().into()),
  )?;
  assert_eq!(rows[0].label, "example");
  assert!(
    rows[0].destination
      == Destination::Link(Target::Uri("https://example.org".into()))
  );
  assert!(
    worker::scan(
      &source,
      "",
      1,
      request,
      vec![(PageId(0), Arc::new(Text::default()))],
      &AtomicBool::new(false),
      Some(root.path().into())
    )
    .is_err()
  );
  assert!(
    worker::scan(
      &source,
      "",
      2,
      request,
      Vec::new(),
      &AtomicBool::new(false),
      Some(root.path().into())
    )
    .is_err()
  );
  assert!(
    worker::scan(
      &source,
      "",
      1,
      request,
      Vec::new(),
      &AtomicBool::new(true),
      Some(root.path().into())
    )
    .is_err()
  );
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn real_sidebar_cache_parity() -> Result<()> {
  let corpus = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let root = tempfile::tempdir()?;
  let path = corpus.join("paper.pdf");
  let source = vpdf_pdf::document::Document::open(&path, "")?;
  let pages = source.pages.len();
  let mut page = source.native.load_pdf_page(0)?;
  let mut note = page.create_annotation(mupdf::pdf::PdfAnnotationType::Text)?;
  note.set_rect(mupdf::Rect::new(10., 10., 30., 30.))?;
  note.set_contents("review this section")?;
  drop((note, page));
  let annotated = root.path().join("annotated.pdf");
  source.native.save(&annotated.to_string_lossy())?;
  for request in [Request::Annotations, Request::Links(PageId(1))] {
    let scan = |cache| {
      worker::scan(
        &annotated,
        "",
        pages,
        request,
        Vec::new(),
        &AtomicBool::new(false),
        cache,
      )
    };
    let baseline = scan(None)?;
    assert!(!baseline.is_empty());
    let cold = scan(Some(root.path().into()))?;
    let warm = scan(Some(root.path().into()))?;
    let signature = |rows: &[Row]| {
      rows
        .iter()
        .map(|r| (r.label.clone(), r.detail.clone(), r.destination.clone()))
        .collect::<Vec<_>>()
    };
    assert!(signature(&baseline) == signature(&cold));
    assert!(signature(&cold) == signature(&warm));
  }
  Ok(())
}

#[test]
fn interrupted_open_releases_handle() -> Result<()> {
  let (_root, app) = app()?;
  let mut pages = 0;
  let opened = vpdf_pdf::document::Document::open_with(&app.path, "", || {
    pages += 1;
    pages < 2
  });
  assert!(opened.is_err());
  assert_eq!(pages, 2);
  assert_eq!(vpdf_pdf::document::Document::open(&app.path, "")?.pages.len(), 2);
  Ok(())
}
