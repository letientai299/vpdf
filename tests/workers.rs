use std::path::PathBuf;
use std::time::Duration;
use vpdf::Result;
use vpdf::ocr::{OcrJob, parse_tsv};
use vpdf::search_worker::SearchWorker;
use vpdf::text::{Bounds, PageId};

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
  let worker = SearchWorker::new(&root.join("book.pdf"), "");
  worker.query("the", false, PageId(0), false)?;
  let latest =
    worker.query("vpdf-impossible-query-83729", false, PageId(0), false)?;
  loop {
    let event = worker.events.recv_timeout(Duration::from_secs(10))?;
    if event.generation != latest {
      continue;
    }
    assert!(event.error.is_none());
    assert!(event.hits.is_empty());
    if event.done {
      break;
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires corpus and Tesseract"]
fn real_ocr_and_cancel() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let job = OcrJob::start(&root.join("ocr.pdf"), "", PageId(0), "eng");
  let text = job
    .result
    .recv_timeout(Duration::from_secs(60))?
    .map_err(anyhow::Error::msg)?;
  assert!(text.plain().to_lowercase().contains("cheese"));
  let cancelled = OcrJob::start(&root.join("ocr.pdf"), "", PageId(0), "eng");
  cancelled.cancel();
  assert!(cancelled.result.recv_timeout(Duration::from_secs(10))?.is_err());
  Ok(())
}
