//! Manual hot paths; arguments select cases.
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_model::geometry::{Screen, page_matrix, transform};
use vpdf_model::text::PageId;
use vpdf_pdf::document::{Document, Handle};
use vpdf_pdf::jobs::{Wake, search::SearchWorker};
use vpdf_reader::motion::Motion;
use vpdf_search::search::{Index, compile};
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Layout;

#[path = "support/files.rs"]
mod corpus;
#[path = "support/hash.rs"]
mod hash;
#[path = "../tests/support/screen.rs"]
mod screen;
mod support;
use support::measure;

const SCREEN: Screen = screen::cells(120, 50);

fn files() -> Result<Vec<PathBuf>> {
  corpus::files(&["../huge/sdm.pdf", "../huge/postgres.pdf"])
}

fn open() -> Result<()> {
  for path in files()? {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    measure(&format!("Handle::open {name}"), || {
      black_box(Handle::open(&path, "").expect("manual handle"));
    })?;
    measure(&format!("Document::open {name}"), || {
      black_box(Document::open(&path, "").expect("manual document"));
    })?;
  }
  Ok(())
}

fn search() -> Result<()> {
  for path in files()? {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut doc = Document::open(&path, "")?;
    let pages = doc.pages.len();
    let text = corpus::spread(pages)
      .map(|page| doc.text(PageId(page)))
      .collect::<Result<Vec<_>>>()?;
    measure(&format!("Index::new {name} 8 pages"), || {
      for (page, text) in text.iter().enumerate() {
        black_box(Index::new(PageId(page), black_box(text)));
      }
    })?;
    let indexes: Vec<_> = text
      .iter()
      .enumerate()
      .map(|(page, text)| Index::new(PageId(page), text))
      .collect();
    let mut bytes = Vec::new();
    for index in &indexes {
      index.encode(&mut bytes);
    }
    println!("{name} index sha256 {}", hash::digest(&bytes));
    let queries =
      std::env::var("VPDF_QUERIES").unwrap_or_else(|_| "the,qzxjvw,The".into());
    let whole = std::env::var_os("VPDF_WHOLE_WORD").is_some();
    for query in queries.split(',') {
      let pattern = compile(query, whole)?;
      let mut hits = Vec::new();
      for index in &indexes {
        for hit in index.matches(&pattern, 1000)? {
          hits.push((hit.first, hit.last, hit.excerpt));
        }
      }
      println!(
        "{name} {query} hits {} sha256 {}",
        hits.len(),
        hash::digest(&serde_json::to_vec(&hits)?)
      );
      measure(&format!("matches {name} {query}"), || {
        for index in &indexes {
          black_box(index.matches(black_box(&pattern), 1000).expect("query"));
        }
      })?;
    }
    println!(
      "{name} retained index bytes {}",
      indexes.iter().map(Index::weight).sum::<usize>()
    );
  }
  Ok(())
}

fn scan() -> Result<()> {
  for path in files()? {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let pages = Handle::open(&path, "")?.pages;
    let worker = SearchWorker::new(&path, "", pages, &Wake::none());
    for (label, query) in [("cold", "qzxjvw"), ("warm", "vwjxzq")] {
      let query = vpdf_search::query::Query {
        text: query.into(),
        ..vpdf_search::query::Query::default()
      };
      let started = Instant::now();
      let generation = worker.lookup(&query, PageId(0))?;
      loop {
        anyhow::ensure!(
          started.elapsed() < Duration::from_secs(60),
          "Search scan timed out"
        );
        let event = match worker.wait(Duration::from_secs(1)) {
          Ok(event) => event,
          Err(RecvTimeoutError::Timeout) => continue,
          Err(error) => return Err(error.into()),
        };
        anyhow::ensure!(
          event.generation == generation,
          "Search generation differs"
        );
        anyhow::ensure!(
          event.error.is_none(),
          "Search failed: {:?}",
          event.error
        );
        anyhow::ensure!(event.hits.is_empty(), "No-hit query matched");
        if event.done {
          break;
        }
      }
      println!("worker {name} {label} {} ns", started.elapsed().as_nanos());
    }
  }
  Ok(())
}

fn geometry() -> Result<()> {
  for path in files()? {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let doc = Document::open(&path, "")?;
    measure(&format!("geometry {name}"), || {
      for page in &doc.pages {
        let bounds = black_box(page.bounds);
        black_box(transform(bounds, &page_matrix(bounds, 1024, 0)));
      }
    })?;
    let mut layout = Layout::new(doc.pages.iter().map(|p| p.bounds), SCREEN);
    measure(&format!("reflow {name}"), || {
      layout.reflow(doc.pages.iter().map(|p| p.bounds));
      black_box(&layout);
    })?;
  }
  Ok(())
}

fn navigation() -> Result<()> {
  for path in files()? {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut doc = Document::open(&path, "")?;
    let mut picked = None;
    let pages = doc.pages.len();
    for page in corpus::spread(pages).take(pages) {
      let text = doc.text(PageId(page))?;
      if let Some(line) = text
        .lines
        .windows(2)
        .position(|lines| lines.iter().all(|line| !line.glyphs.is_empty()))
      {
        picked = Some((text, line));
        break;
      }
    }
    let Some((text, line)) = picked else {
      println!("motion {name}: skipped, no adjacent text lines");
      continue;
    };
    let mut text = vec![text];
    let mut reader = vpdf_reader::reader::Reader::default();
    reader.cursor.line = line;
    let origin = reader.cursor;
    reader.motion(Motion::Down, 1, false, &mut text)?;
    anyhow::ensure!(reader.cursor.line != line, "Navigation never moved");
    reader.motion(Motion::Up, 1, false, &mut text)?;
    anyhow::ensure!(reader.cursor == origin, "Navigation origin differs");
    measure(&format!("motion {name}"), || {
      for motion in [Motion::Down, Motion::Up] {
        reader.motion(motion, 1, false, &mut text).expect("manual motion");
      }
      black_box(reader.cursor);
    })?;
  }
  Ok(())
}

fn tick(app: &mut Solo, frame: &mut Vec<u8>) -> Result<()> {
  app.raster_updates();
  app.outline_updates()?;
  frame.clear();
  app.draw(frame)
}

fn overview() -> Result<()> {
  for path in files()? {
    let started = Instant::now();
    let mut app = Solo::open(&path, "", false, SCREEN)?;
    app.action(Input::plain(Action::Overview))?;
    let mut frame = Vec::new();
    loop {
      tick(&mut app, &mut frame)?;
      if app.complete {
        break;
      }
      anyhow::ensure!(
        started.elapsed() < Duration::from_secs(30),
        "Overview timeout"
      );
      std::thread::sleep(Duration::from_millis(1));
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    println!(
      "{name} overview ready {:.3} ms",
      started.elapsed().as_secs_f64() * 1000.0
    );
    let grid = vpdf_ui::overview::Grid::new(SCREEN, app.doc.pages[0].bounds);
    let (columns, rows) = grid.shape();
    if app.doc.pages.len() <= columns * rows {
      println!("overview {name}: skipped, fits screen");
      continue;
    }
    println!("{}", scroll(&mut app, &mut frame, &name)?);
  }
  Ok(())
}

fn scroll(
  app: &mut Solo,
  frame: &mut Vec<u8>,
  name: &str,
) -> Result<serde_json::Value> {
  let mut input = Vec::new();
  let mut draws = Vec::new();
  let mut incomplete = 0;
  let mut gap: Option<Instant> = None;
  let mut longest = Duration::ZERO;
  let started = Instant::now();
  let mut sent = 0;
  let mut direction = 1;
  while sent < 120 {
    if started.elapsed() >= Duration::from_micros(33333) * sent {
      let overview = app.overlays.overview().expect("Overview open");
      let (selected, top) = (overview.selected(), overview.top());
      if selected == 0 {
        direction = 1;
      } else if selected == app.doc.pages.len() - 1 {
        direction = -1;
      }
      let now = Instant::now();
      app.action(Input::plain(Action::Screen(direction)))?;
      let elapsed = now.elapsed().as_secs_f64() * 1000.0;
      if app.overlays.overview().expect("Overview open").top() != top {
        input.push(elapsed);
      }
      anyhow::ensure!(
        app.overlays.overview().expect("Overview open").selected() != selected,
        "Overview selection never moved"
      );
      sent += 1;
    }
    let now = Instant::now();
    tick(app, frame)?;
    draws.push(now.elapsed().as_secs_f64() * 1000.0);
    if app.complete {
      if let Some(since) = gap.take() {
        longest = longest.max(since.elapsed());
      }
    } else {
      incomplete += 1;
      gap.get_or_insert(now);
    }
    std::thread::sleep(Duration::from_micros(8333));
  }
  let settle = Instant::now();
  while !app.complete {
    tick(app, frame)?;
    if app.complete {
      break;
    }
    anyhow::ensure!(
      settle.elapsed() < Duration::from_secs(30),
      "Overview timeout"
    );
    std::thread::sleep(Duration::from_millis(1));
  }
  if let Some(since) = gap {
    longest = longest.max(since.elapsed());
  }
  anyhow::ensure!(!input.is_empty(), "Overview never scrolled");
  input.sort_by(f64::total_cmp);
  draws.sort_by(f64::total_cmp);
  Ok(serde_json::json!({
    "document": name,
    "inputs": sent,
    "moving_inputs": input.len(),
    "input_p50_ms": input[input.len() / 2],
    "input_p95_ms": support::percentile::tail(&input, 95),
    "input_p99_ms": support::percentile::tail(&input, 99),
    "draw_p50_ms": draws[draws.len() / 2],
    "draw_p95_ms": support::percentile::tail(&draws, 95),
    "draw_p99_ms": support::percentile::tail(&draws, 99),
    "incomplete_frames": incomplete,
    "frames": draws.len(),
    "longest_gap_ms": longest.as_secs_f64() * 1000.0,
    "settle_ms": settle.elapsed().as_secs_f64() * 1000.0,
    "duration_ms": started.elapsed().as_secs_f64() * 1000.0,
  }))
}

fn fidelity() -> Result<()> {
  for path in files()? {
    let mut doc = Document::open(&path, "")?;
    let pages = doc.pages.len();
    let mut bytes = Vec::new();
    for page in &doc.pages {
      for value in
        [page.bounds.x0, page.bounds.y0, page.bounds.x1, page.bounds.y1]
      {
        bytes.extend_from_slice(&value.to_le_bytes());
      }
    }
    println!("{} bounds {}", path.display(), hash::digest(&bytes));
    for page in corpus::spread(pages).map(PageId) {
      vpdf_pdf::raster::visit_page(&mut doc, page, 256, |w, h, pixels| {
        println!(
          "{} page {} {w}x{h} pixels {}",
          path.display(),
          page.0 + 1,
          hash::digest(pixels)
        );
        Ok(())
      })?;
    }
  }
  Ok(())
}

fn retained() -> Result<()> {
  use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
  if !std::env::args().any(|arg| arg == "--retained-child") {
    let status = std::process::Command::new(std::env::current_exe()?)
      .args(["retained", "--retained-child"])
      .status()?;
    anyhow::ensure!(status.success(), "Memory benchmark failed");
    return Ok(());
  }
  let mut system = System::new();
  let pid = Pid::from_u32(std::process::id());
  let kind = ProcessRefreshKind::nothing().with_memory().without_tasks();
  let mut rss = || {
    system.refresh_processes_specifics(
      ProcessesToUpdate::Some(&[pid]),
      true,
      kind,
    );
    system.process(pid).map_or(0, sysinfo::Process::memory)
  };
  let before = rss();
  let started = Instant::now();
  let docs = files()?
    .iter()
    .map(|path| Document::open(path, ""))
    .collect::<Result<Vec<_>>>()?;
  let elapsed = started.elapsed();
  let after = rss();
  println!(
    "retained {} documents {} pages; RSS {} baseline {} delta {}; {:.3} ms",
    docs.len(),
    docs.iter().map(|doc| doc.pages.len()).sum::<usize>(),
    after,
    before,
    after.saturating_sub(before),
    elapsed.as_secs_f64() * 1000.0
  );
  black_box(docs);
  Ok(())
}

fn main() -> Result<()> {
  support::run(&[
    ("open", open),
    ("search", search),
    ("scan", scan),
    ("geometry", geometry),
    ("navigation", navigation),
    ("overview", overview),
    ("retained", retained),
    ("fidelity", fidelity),
  ])
}
