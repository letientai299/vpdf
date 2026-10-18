use serde::Serialize;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::content_cache::{alias, hex};
use vpdf::document::{Document, extract};
use vpdf::find::Query;
use vpdf::layout::Screen;
use vpdf::motion::Motion;
use vpdf::search::{self, Index};
use vpdf::search_worker::SearchWorker;
use vpdf::text::PageId;
use vpdf::tiles::{self, TileKey};

#[derive(Serialize)]
struct Timing {
  samples: usize,
  p50_ms: f64,
  p95_ms: f64,
  max_ms: f64,
}

fn measure(
  mut run: impl FnMut() -> Result<()>,
  count: usize,
) -> Result<Timing> {
  let mut times = Vec::with_capacity(count);
  for _ in 0..count {
    let started = Instant::now();
    run()?;
    times.push(started.elapsed().as_secs_f64() * 1000.0);
  }
  times.sort_by(f64::total_cmp);
  Ok(Timing {
    samples: count,
    p50_ms: times[count / 2],
    p95_ms: times[(count * 95 / 100).min(count - 1)],
    max_ms: times[count - 1],
  })
}

#[derive(Serialize)]
struct Report {
  document: String,
  sha256: String,
  page: usize,
  pages: usize,
  open: Timing,
  extract: Timing,
  tile: Timing,
  index_build: Timing,
  index_search: Timing,
  motion: Timing,
  frame: Timing,
  search_frame: Timing,
  warm_scroll_upload_bytes: u64,
  cold_search_ms: f64,
  warm_search_ms: f64,
  first_hit_ms: f64,
}

fn document(path: &Path, page: usize) -> Result<Report> {
  let hash = hex(&alias(&mut std::fs::File::open(path)?)?);
  let open = measure(
    || {
      black_box(Document::open(path, "")?);
      Ok(())
    },
    10,
  )?;
  let mut doc = Document::open(path, "")?;
  let native = doc.native.load_page(page as i32)?;
  let extract = measure(
    || {
      black_box(extract(&native)?);
      Ok(())
    },
    100,
  )?;
  let key = TileKey {
    page: PageId(page),
    scale: 2048,
    rotation: 0,
    x: 128,
    y: 256,
    width: 512,
    height: 256,
    crop: vpdf::layout::Crop::default(),
  };
  tiles::render(&mut doc, key)?;
  let tile = measure(
    || {
      black_box(tiles::render(&mut doc, key)?);
      Ok(())
    },
    100,
  )?;
  let text = doc.text(PageId(page))?;
  let index_build = measure(
    || {
      black_box(Index::new(PageId(page), &text));
      Ok(())
    },
    100,
  )?;
  let index = Index::new(PageId(page), &text);
  let query = search::compile("the", false)?;
  let index_search = measure(
    || {
      black_box(index.matches(&query, 1000)?);
      Ok(())
    },
    100,
  )?;
  let mut app = App::with_screen(
    path,
    "",
    false,
    Screen {
      columns: 160,
      rows: 60,
      cell_width: 9,
      cell_height: 18,
      origin: (0, 0),
    },
  )?;
  app.reader.edge(PageId(page), false, &mut app.doc)?;
  app.ensure_cursor()?;
  let motion = measure(
    || {
      app.reader.motion(Motion::Right, 1, false, &mut app.doc)?;
      app.reader.motion(Motion::Left, 1, false, &mut app.doc)?;
      Ok(())
    },
    1000,
  )?;
  let (frame, uploaded) = frames(&mut app)?;
  app.search.hits = index.matches(&query, 1000)?;
  let (search_frame, _) = frames(&mut app)?;
  let (cold_search_ms, first_hit_ms, warm_search_ms) = searches(path)?;
  Ok(Report {
    document: path.file_name().unwrap().to_string_lossy().into(),
    sha256: hash,
    page: page + 1,
    pages: doc.pages.len(),
    open,
    extract,
    tile,
    index_build,
    index_search,
    motion,
    frame,
    search_frame,
    warm_scroll_upload_bytes: uploaded,
    cold_search_ms,
    first_hit_ms,
    warm_search_ms,
  })
}

fn frames(app: &mut App) -> Result<(Timing, u64)> {
  let base = app.reader.scroll;
  let mut out = io::sink();
  for frame in 0..40 {
    let scroll = app.layout.clamp_scroll(base + frame as f32 * 4.0);
    for key in tiles::visible(&app.layout, scroll) {
      if !app.graphics.contains(&key) {
        let tile = tiles::render(&mut app.doc, key)?;
        app.graphics.upload(&tile, &mut out)?;
      }
    }
  }
  let uploaded = app.graphics.uploaded;
  let mut frame = 0;
  let mut serialized = Vec::with_capacity(64 * 1024);
  let timing = measure(
    || {
      app.reader.scroll =
        app.layout.clamp_scroll(base + (frame % 40) as f32 * 4.0);
      serialized.clear();
      app.draw(&mut serialized)?;
      black_box(&serialized);
      frame += 1;
      Ok(())
    },
    200,
  )?;
  app.reader.scroll = base;
  Ok((timing, app.graphics.uploaded - uploaded))
}

fn searches(path: &Path) -> Result<(f64, f64, f64)> {
  let pages = Document::open(path, "")?.pages.len();
  let worker = SearchWorker::new(path, "", pages);
  let mut total = Vec::new();
  let mut first = 0.0;
  for run in 0..2 {
    let started = Instant::now();
    let generation = worker
      .lookup(&Query { text: "the".into(), ..Query::default() }, PageId(0))?;
    loop {
      let event = worker.events.recv_timeout(Duration::from_secs(30))?;
      if event.generation != generation {
        continue;
      }
      if let Some(error) = event.error {
        anyhow::bail!(error);
      }
      if first == 0.0 && !event.hits.is_empty() {
        first = started.elapsed().as_secs_f64() * 1000.0;
      }
      if event.done {
        break;
      }
    }
    total.push(started.elapsed().as_secs_f64() * 1000.0);
    if run == 0 {
      std::thread::sleep(Duration::from_millis(20));
    }
  }
  Ok((total[0], first, total[1]))
}

fn main() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut reports = Vec::new();
  for (name, page) in
    [("book.pdf", 38), ("paper.pdf", 2), ("form.pdf", 0), ("ocr.pdf", 0)]
  {
    reports.push(document(&root.join(name), page)?);
  }
  let output = serde_json::to_string_pretty(&reports)?;
  if let Ok(path) = std::env::var("VPDF_BENCH_OUTPUT") {
    std::fs::write(path, &output)?;
  }
  writeln!(io::stdout(), "{output}")?;
  if std::env::var_os("VPDF_PERF_CHECK").is_some() {
    for report in &reports {
      anyhow::ensure!(
        report.frame.p95_ms < 8.33 && report.search_frame.p95_ms < 8.33,
        "Warm frame exceeds 8.33 ms"
      );
      anyhow::ensure!(report.motion.p95_ms < 1.0, "Warm motion exceeds 1 ms");
      anyhow::ensure!(
        report.warm_scroll_upload_bytes == 0,
        "Warm scroll uploaded base images"
      );
    }
  }
  Ok(())
}
