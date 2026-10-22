use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf_model::text::{Bounds, PageId};
use vpdf_pdf::document::{Document, Handle};
use vpdf_pdf::jobs::Wake;
use vpdf_pdf::jobs::ocr::{OcrJob, parse_tsv};
use vpdf_pdf::jobs::search::SearchWorker;
use vpdf_search::query::Query;

#[test]
fn ocr_geometry_preserves_unicode() -> Result<()> {
  let tsv = concat!(
    "level\tpage\tblock\tpar\tline\tword\tleft\ttop\t",
    "width\theight\tconf\ttext\n",
    "5\t1\t1\t1\t1\t1\t10\t20\t30\t10\t99\tcafé\n",
    "5\t1\t1\t1\t1\t2\t50\t20\t40\t10\t99\tStraße\n"
  );
  let text = parse_tsv(tsv, Bounds::default(), 2.0)?;
  assert_eq!(text.plain(), "café Straße");
  assert!(text.lines[0].paragraph);
  assert_eq!(text.lines[0].glyphs[0].bounds.x0, 5.0);
  assert_eq!(text.lines[0].glyphs[0].bounds.y0, 10.0);
  assert!(parse_tsv(tsv, Bounds::default(), 0.0).is_err());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn superseded_search_completes_latest_query() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let path = root.join("book.pdf");
  let pages = Handle::open(&path, "")?.pages;
  let (wake, woken) = counted();
  let worker = SearchWorker::new(&path, "", pages, &wake);
  worker
    .lookup(&Query { text: "the".into(), ..Query::default() }, PageId(0))?;
  let latest = worker.lookup(
    &Query { text: "vpdf-impossible-query-83729".into(), ..Query::default() },
    PageId(0),
  )?;
  loop {
    let event = worker.wait(Duration::from_secs(10))?;
    if event.generation != latest {
      continue;
    }
    assert!(event.error.is_none());
    assert!(event.hits.is_empty());
    if event.done {
      break;
    }
  }
  assert!(woke(&woken, 1), "results wake the loop");
  Ok(())
}

/// Waits for `count` wakes; each follows its send.
fn woke(woken: &AtomicUsize, count: usize) -> bool {
  let started = Instant::now();
  while woken.load(Ordering::Relaxed) < count {
    if started.elapsed() > Duration::from_secs(5) {
      return false;
    }
    std::thread::sleep(Duration::from_millis(1));
  }
  true
}

#[test]
fn search_results_wake_the_loop() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("words.pdf");
  crate::common::text_pdf(&path, &["needle in a haystack"])?;
  let (wake, woken) = counted();
  let worker = SearchWorker::new(&path, "", 1, &wake);
  let query = Query { text: "needle".into(), ..Query::default() };
  let generation = worker.lookup(&query, PageId(0))?;
  let mut hits = 0;
  loop {
    let event = worker.wait(Duration::from_secs(10))?;
    if event.generation != generation {
      continue;
    }
    hits += event.hits.len();
    if event.done {
      break;
    }
  }
  assert!(woke(&woken, 1));
  assert_eq!(hits, 1);
  Ok(())
}

/// A wake that counts its calls.
fn counted() -> (Wake, Arc<AtomicUsize>) {
  let woken = Arc::new(AtomicUsize::new(0));
  let count = Arc::clone(&woken);
  let wake = Wake::new(move || {
    count.fetch_add(1, Ordering::Relaxed);
  });
  (wake, woken)
}

/// Workers skip the page walk yet place every page where the pane does.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn handles_read_document_bounds() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for file in ["paper.pdf", "book.pdf", "form.pdf", "ocr.pdf"] {
    let doc = Document::open(&root.join(file), "")?;
    let handle = Handle::open(&root.join(file), "")?;
    assert_eq!(handle.pages, doc.pages.len(), "{file}");
    let corners = |b: Bounds| [b.x0, b.y0, b.x1, b.y1];
    for (index, page) in doc.pages.iter().enumerate() {
      let bounds = handle.bounds(PageId(index))?;
      assert_eq!(corners(bounds), corners(page.bounds), "{file} {index}");
    }
    assert!(handle.bounds(PageId(handle.pages)).is_err());
  }
  Ok(())
}

#[test]
#[ignore = "requires corpus and Tesseract"]
fn real_ocr_and_cancel() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  automatic_scan(&root.join("scan.pdf"))?;
  let (wake, woken) = counted();
  let job = OcrJob::start(&root.join("ocr.pdf"), "", PageId(0), "eng", &wake);
  let text = job
    .work
    .result
    .recv_timeout(Duration::from_secs(60))?
    .map_err(anyhow::Error::msg)?
    .expect("explicit OCR returns text");
  assert!(text.plain().to_lowercase().contains("cheese"));
  assert!(woke(&woken, 1));
  let cancelled =
    OcrJob::start(&root.join("ocr.pdf"), "", PageId(0), "eng", &wake);
  cancelled.cancel();
  assert!(
    cancelled.work.result.recv_timeout(Duration::from_secs(10))?.is_err()
  );
  assert!(woke(&woken, 2), "cancellation wakes too");
  let scan = OcrJob::start(&root.join("scan.pdf"), "", PageId(9), "eng", &wake);
  let text = scan
    .work
    .result
    .recv_timeout(Duration::from_secs(60))?
    .map_err(anyhow::Error::msg)?
    .expect("explicit OCR returns text");
  assert!(text.plain().contains("computing devices"));
  let mut slopes: Vec<_> = text
    .lines
    .iter()
    .filter_map(|line| line.ocr)
    .filter_map(|q| {
      let width = q.0[1].0 - q.0[0].0;
      (width > 100.0).then(|| (q.0[1].1 - q.0[0].1) / width)
    })
    .collect();
  slopes.sort_by(f32::total_cmp);
  assert!(slopes.len() > 20);
  assert!((-0.016..-0.009).contains(&slopes[slopes.len() / 2]));
  assert!(
    text.lines.iter().flat_map(|line| &line.glyphs).all(|g| g.quad.is_none())
  );
  Ok(())
}

#[test]
fn auto_ocr_defers_and_skips_native_pages() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("native.pdf");
  crate::common::text_pdf(&path, &["native text"])?;
  let mut app =
    vpdf::solo::Solo::open(&path, "", true, crate::common::cells(80, 24))?;
  assert!(app.ocr.is_none(), "open starts no OCR");
  app.start_deferred();
  assert!(app.ocr.is_none(), "unpresented view starts no OCR");
  app.draw(&mut Vec::new())?;
  assert!(app.complete);
  assert!(app.ocr.is_none(), "frame construction starts no OCR");
  app.start_deferred();
  assert!(app.ocr.as_ref().unwrap().is_automatic());
  let started = Instant::now();
  while app.ocr.is_some() {
    app.ocr_updates()?;
    assert!(started.elapsed() < Duration::from_secs(10));
    std::thread::sleep(Duration::from_millis(1));
  }
  assert!(!app.doc.has_ocr(PageId(0)), "native text is not replaced");
  app.start_deferred();
  assert!(app.ocr.is_none(), "native detection happens once");
  assert!(app.doc.text(PageId(0))?.plain().contains("native text"));
  Ok(())
}

#[test]
fn auto_ocr_cancellation_keeps_frames_independent() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("native.pdf");
  crate::common::text_pdf(&path, &["native text"])?;
  let mut app =
    vpdf::solo::Solo::open(&path, "", true, crate::common::cells(80, 24))?;
  app.draw(&mut Vec::new())?;
  app.start_deferred();
  assert!(app.ocr.is_some(), "automatic worker queued");
  app.draw(&mut Vec::new())?;
  assert!(app.complete, "OCR never holds frame readiness");
  app.complete = false;
  app.start_deferred();
  assert!(app.ocr.is_none(), "new rendering cancels automatic work");
  app.complete = true;
  app.start_deferred();
  assert!(app.ocr.is_some(), "presented page can retry");
  app.command("cancel")?;
  app.start_deferred();
  assert!(app.ocr.is_none(), "cancel persists across frames");
  Ok(())
}

fn automatic_scan(path: &std::path::Path) -> Result<()> {
  for _ in 0..2 {
    let mut app =
      vpdf::solo::Solo::open(path, "", false, crate::common::cells(80, 24))?;
    assert!(app.ocr.is_none());
    app.command("10")?;
    let started = Instant::now();
    loop {
      app.raster_updates();
      app.draw(&mut Vec::new())?;
      assert!(app.ocr.is_none(), "rendering never schedules OCR");
      if app.complete {
        break;
      }
      assert!(started.elapsed() < Duration::from_secs(20));
      std::thread::sleep(Duration::from_millis(2));
    }
    app.start_deferred();
    assert!(app.ocr.as_ref().unwrap().is_automatic());
    while app.ocr.is_some() {
      app.ocr_updates()?;
      assert!(started.elapsed() < Duration::from_secs(60));
      std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.doc.text(PageId(9))?.plain().contains("computing devices"));
    assert!(app.doc.has_ocr(PageId(9)));
    app.start_deferred();
    assert!(app.ocr.is_none(), "resident OCR is not repeated");
    app.start_search("computing devices", false, false)?;
    crate::common::search_loaded(&mut app)?;
    assert!(!app.search.hits().is_empty(), "automatic text seeds search");
  }
  Ok(())
}
