use super::*;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vpdf_model::text::{Bounds, PageId, Text};
use vpdf_pdf::content_cache::{Kind, Record};
use vpdf_ui::panel::{Destination, Row};

/// A text note on each page.
const NOTE: &str =
  "/Annots[<</Type/Annot/Subtype/Text/Rect[10 10 30 30]/Contents(note)>>]";

fn app() -> Result<(tempfile::TempDir, App)> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first page", "second page"], NOTE)?;
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
  let kind = PanelKind::Annotations;
  app.open_panel(kind)?;
  assert!(app.sidebar_loading(kind));
  assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  let (send, stopped) = mpsc::channel();
  app.sidebar.annotations =
    Load::Running(OneShot::spawn(&app.wake, move |stop| {
      while !stop.load(Ordering::Relaxed) {
        std::thread::yield_now();
      }
      send.send(stop.load(Ordering::Relaxed)).unwrap();
      None
    }));
  app.cancel_sidebars();
  assert!(stopped.recv_timeout(Duration::from_secs(5))?);
  app.prepare_sidebars();
  assert!(!app.sidebar_pending(kind));
  app.open_panel(kind)?;
  loaded(&mut app, kind)?;
  assert_eq!(app.panel.as_ref().unwrap().view.rows().len(), 2);
  Ok(())
}

/// A worker holds its load's slot until its thread ends, across
/// reloads; requests meanwhile start once. Canceling drops a waiting
/// request.
#[test]
fn workers_wait_for_their_slot() -> Result<()> {
  let (_root, mut app) = app()?;
  let kind = PanelKind::Annotations;
  let held = hold_annotations(&app);
  app.want_sidebar(kind);
  app.refresh_sidebars()?;
  app.want_sidebar(kind);
  app.sidebar_updates()?;
  assert!(!app.sidebar_loading(kind));
  assert!(app.sidebar_pending(kind), "queued loads are pending");
  app.prepare_sidebars();
  assert!(!app.sidebars_deferred(), "queued loads let the loop sleep");
  drop(held);
  app.sidebar_updates()?;
  assert!(app.sidebar_loading(kind));
  loaded(&mut app, kind)?;
  assert_eq!(app.sidebar_rows(kind).len(), 2);
  app.refresh_sidebars()?;
  let held = hold_annotations(&app);
  app.want_sidebar(kind);
  app.cancel_sidebars();
  drop(held);
  app.sidebar_updates()?;
  assert!(!app.sidebar_loading(kind));
  Ok(())
}

#[test]
fn reload_discards_old_rows() -> Result<()> {
  let (root, mut app) = app()?;
  let kind = PanelKind::Annotations;
  app.open_panel(kind)?;
  loaded(&mut app, kind)?;
  assert_eq!(app.sidebar_rows(kind).len(), 2);
  let replacement = root.path().join("replacement.pdf");
  crate::fixtures::pdf(&replacement, &["no notes", "second page"], "")?;
  std::fs::rename(replacement, &app.path)?;
  app.reload_with("")?;
  assert!(app.sidebar_loading(kind));
  assert!(app.sidebar_rows(kind).is_empty());
  loaded(&mut app, kind)?;
  assert!(app.sidebar_rows(kind).is_empty());
  Ok(())
}

#[test]
fn cached_rows_skip_native() -> Result<()> {
  let root = tempfile::tempdir()?;
  let source = root.path().join("source.pdf");
  std::fs::write(&source, b"not a native PDF")?;
  let cache = Record::open(
    root.path().into(),
    &source,
    "",
    Kind::Annotations,
    &code(),
    || true,
  )
  .unwrap();
  cache.write(
    br#"[1,[{
    "label":"note","detail":"",
    "content":{"Annotation":[{"page":0,"line":0,"glyph":0},
      {"kind":"Text","areas":[],"contents":"note","author":""}]}
  }]]"#,
  )?;
  let version = content_cache::Version::read(&source).unwrap();
  let memo = content_cache::recall(root.path(), version);
  assert_eq!(memo.is_some(), content_cache::exact(&source));
  let rows = worker::scan(
    &source,
    "",
    vec![Bounds::default()],
    Vec::new(),
    &AtomicBool::new(false),
    Some(root.path().into()),
  )?;
  assert_eq!(rows[0].label, "note");
  assert!(matches!(
    &rows[0].destination,
    Destination::Annotation { annotation, .. } if annotation.contents == "note"
  ));
  assert!(
    worker::scan(
      &source,
      "",
      vec![Bounds::default()],
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
      vec![Bounds::default(); 2],
      Vec::new(),
      &AtomicBool::new(false),
      Some(root.path().into())
    )
    .is_err()
  );
  let empty = tempfile::tempdir()?;
  assert!(
    worker::scan(
      &source,
      "",
      vec![Bounds::default()],
      Vec::new(),
      &AtomicBool::new(true),
      Some(empty.path().into())
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
  let bounds: Vec<_> = source.pages.iter().map(|page| page.bounds).collect();
  let mut page = source.native.load_pdf_page(0)?;
  let mut note = page.create_annotation(mupdf::pdf::PdfAnnotationType::Text)?;
  note.set_rect(mupdf::Rect::new(10., 10., 30., 30.))?;
  note.set_contents("review this section")?;
  drop((note, page));
  let annotated = root.path().join("annotated.pdf");
  source.native.save(&annotated.to_string_lossy())?;
  let scan = |cache| {
    worker::scan(
      &annotated,
      "",
      bounds.clone(),
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
  Ok(())
}

#[test]
fn page_count_change_fails_scan() -> Result<()> {
  let (_root, app) = app()?;
  let scan = worker::scan(
    &app.path,
    "",
    vec![Bounds::default(); 3],
    Vec::new(),
    &AtomicBool::new(false),
    None,
  );
  assert!(scan.is_err_and(|e| e.to_string().contains("page count changed")));
  Ok(())
}

#[test]
fn outline_notes_follow_jobs() -> Result<()> {
  let (_root, mut app) = app()?;
  app.open_panel(PanelKind::Outline)?;
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "loading");
  app.cancel_jobs()?;
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "");
  app.open_panel(PanelKind::Outline)?;
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "loading");
  let start = Instant::now();
  while app.outline_pending() {
    app.outline_updates()?;
    anyhow::ensure!(
      start.elapsed() < Duration::from_secs(5),
      "Outline timeout"
    );
    std::thread::sleep(Duration::from_millis(1));
  }
  assert_eq!(app.panel.as_ref().unwrap().view.note(), "inferred");
  Ok(())
}
