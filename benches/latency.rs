//! Cold-path latency: open, jumps, zoom, and paced scrolling through the
//! reader with a real render worker. Frames go to memory, not a terminal.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::keys::{Action, Input};
use vpdf::layout::Screen;
use vpdf::tiles;

const FRAME: Duration = Duration::from_micros(8333);
const SETTLE: Duration = Duration::from_secs(10);

#[derive(Default, Serialize)]
struct Timing {
  samples: usize,
  p50_ms: f64,
  p95_ms: f64,
  max_ms: f64,
}

fn timing(mut times: Vec<f64>) -> Timing {
  if times.is_empty() {
    return Timing::default();
  }
  times.sort_by(f64::total_cmp);
  let count = times.len();
  Timing {
    samples: count,
    p50_ms: times[count / 2],
    p95_ms: times[(count * 95 / 100).min(count - 1)],
    max_ms: times[count - 1],
  }
}

fn ms(duration: Duration) -> f64 {
  duration.as_secs_f64() * 1000.0
}

/// Pending visible tiles at the current scroll.
fn missing(app: &mut App) -> usize {
  let keys = tiles::visible(&app.layout, app.reader.scroll);
  keys.iter().filter(|key| !app.graphics.contains(key)).count()
}

/// One loop iteration of the session: results, animation, frame.
struct Frame {
  build: f64,
  bytes: usize,
  missing: usize,
}

struct Sim {
  app: App,
  frame: Vec<u8>,
  uploaded: usize,
}

impl Sim {
  fn tick(&mut self) -> Result<Frame> {
    self.app.raster_updates();
    self.app.animate();
    self.frame.clear();
    let before = self.app.graphics.uploaded;
    let started = Instant::now();
    self.app.draw(&mut self.frame)?;
    let build = ms(started.elapsed());
    self.uploaded += (self.app.graphics.uploaded - before) as usize;
    let missing = missing(&mut self.app);
    Ok(Frame { build, bytes: self.frame.len(), missing })
  }

  /// Ticks at the frame rate until every visible tile is resident;
  /// returns milliseconds to the first tile and to completion.
  fn settle(&mut self, started: Instant) -> Result<(f64, f64)> {
    let total = missing(&mut self.app);
    let mut first = None;
    loop {
      let frame = self.tick()?;
      if first.is_none() && frame.missing < total {
        first = Some(ms(started.elapsed()));
      }
      if frame.missing == 0 && self.app.animation.is_none() {
        let done = ms(started.elapsed());
        return Ok((first.unwrap_or(done), done));
      }
      anyhow::ensure!(started.elapsed() < SETTLE, "Viewport never settled");
      std::thread::sleep(FRAME);
    }
  }

  fn act(&mut self, action: Action) -> Result<f64> {
    let started = Instant::now();
    self.app.action(Input {
      action,
      count: 1,
      explicit: false,
      register: '"',
    })?;
    Ok(ms(started.elapsed()))
  }
}

#[derive(Serialize)]
struct Jumps {
  input: Timing,
  first_tile: Timing,
  complete: Timing,
}

#[derive(Serialize)]
struct Motion {
  name: &'static str,
  inputs: usize,
  frames: usize,
  incomplete_frames: usize,
  longest_gap_ms: f64,
  input: Timing,
  build: Timing,
  frame_bytes: Timing,
  upload_mib: f64,
  settle_ms: f64,
}

#[derive(Serialize)]
struct Report {
  document: String,
  pages: usize,
  screen: (u16, u16),
  open_ms: f64,
  open_complete_ms: f64,
  jumps: Jumps,
  zoom: Jumps,
  motions: Vec<Motion>,
}

/// Distinct pseudo-random pages, reproducible across runs.
fn pages(count: usize, total: usize) -> Vec<usize> {
  let mut seed = 0x9e37_79b9_u64;
  let mut picked = Vec::new();
  while picked.len() < count.min(total) {
    seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let page = (seed >> 33) as usize % total;
    if !picked.contains(&page) {
      picked.push(page);
    }
  }
  picked
}

fn jumps(sim: &mut Sim, count: usize) -> Result<Jumps> {
  let (mut input, mut first, mut complete) = (vec![], vec![], vec![]);
  for page in pages(count, sim.app.doc.pages.len()) {
    let started = Instant::now();
    sim.app.command(&(page + 1).to_string())?;
    input.push(ms(started.elapsed()));
    let (one, all) = sim.settle(started)?;
    first.push(one);
    complete.push(all);
  }
  Ok(Jumps {
    input: timing(input),
    first_tile: timing(first),
    complete: timing(complete),
  })
}

fn zooms(sim: &mut Sim) -> Result<Jumps> {
  let (mut input, mut first, mut complete) = (vec![], vec![], vec![]);
  for action in [1, 1, 1, -1, -1, -1].map(Action::Zoom) {
    let started = Instant::now();
    input.push(sim.act(action)?);
    let (one, all) = sim.settle(started)?;
    first.push(one);
    complete.push(all);
  }
  sim.act(Action::Fit)?;
  sim.settle(Instant::now())?;
  Ok(Jumps {
    input: timing(input),
    first_tile: timing(first),
    complete: timing(complete),
  })
}

/// Sends `action` every `period` for `inputs` inputs while drawing at
/// the frame rate, as the session loop does.
fn motion(
  sim: &mut Sim,
  name: &'static str,
  action: Action,
  period: Duration,
  inputs: usize,
) -> Result<Motion> {
  let (mut input, mut build, mut bytes) = (vec![], vec![], vec![]);
  let (mut frames, mut incomplete, mut gap) = (0, 0, 0.0_f64);
  let mut blank_since: Option<Instant> = None;
  let uploaded = sim.uploaded;
  let started = Instant::now();
  let mut sent = 0;
  while sent < inputs {
    if started.elapsed() >= period * sent as u32 {
      input.push(sim.act(action)?);
      sent += 1;
    }
    let frame = sim.tick()?;
    frames += 1;
    build.push(frame.build);
    bytes.push(frame.bytes as f64);
    if frame.missing > 0 {
      incomplete += 1;
      blank_since.get_or_insert_with(Instant::now);
    } else if let Some(since) = blank_since.take() {
      gap = gap.max(ms(since.elapsed()));
    }
    std::thread::sleep(FRAME);
  }
  let settle = Instant::now();
  sim.settle(settle)?;
  if let Some(since) = blank_since {
    gap = gap.max(ms(since.elapsed()));
  }
  Ok(Motion {
    name,
    inputs,
    frames,
    incomplete_frames: incomplete,
    longest_gap_ms: gap,
    input: timing(input),
    build: timing(build),
    frame_bytes: timing(bytes),
    upload_mib: (sim.uploaded - uploaded) as f64 / 1_048_576.0,
    settle_ms: ms(settle.elapsed()),
  })
}

fn motions(sim: &mut Sim) -> Result<Vec<Motion>> {
  let start = sim.app.doc.pages.len() / 3;
  sim.app.command(&(start + 1).to_string())?;
  sim.settle(Instant::now())?;
  let hz = |rate: u64| Duration::from_micros(1_000_000 / rate);
  Ok(vec![
    motion(sim, "rows3_60hz", Action::Rows(3), hz(60), 180)?,
    motion(sim, "rows1_120hz", Action::Rows(1), hz(120), 240)?,
    motion(sim, "half_10hz", Action::Scroll(0.5), hz(10), 30)?,
    motion(sim, "half_up_10hz", Action::Scroll(-0.5), hz(10), 30)?,
    motion(sim, "page_4hz", Action::Scroll(1.0), hz(4), 12)?,
  ])
}

fn document(path: &Path, screen: Screen) -> Result<Report> {
  let started = Instant::now();
  let app = App::with_screen(path, "", false, screen)?;
  let open_ms = ms(started.elapsed());
  let mut sim = Sim { app, frame: Vec::new(), uploaded: 0 };
  let (_, open_complete_ms) = sim.settle(started)?;
  let jumps = jumps(&mut sim, 20)?;
  let zoom = zooms(&mut sim)?;
  let motions = motions(&mut sim)?;
  Ok(Report {
    document: path.file_name().unwrap_or_default().to_string_lossy().into(),
    pages: sim.app.doc.pages.len(),
    screen: (screen.columns, screen.rows),
    open_ms,
    open_complete_ms,
    jumps,
    zoom,
    motions,
  })
}

fn main() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let files: Vec<PathBuf> = match std::env::var_os("VPDF_EXTRA") {
    Some(extra) => std::env::split_paths(&extra).collect(),
    None => ["book", "paper", "form", "ocr"]
      .iter()
      .map(|name| root.join(format!("{name}.pdf")))
      .collect(),
  };
  let columns = std::env::var("VPDF_COLUMNS")
    .ok()
    .and_then(|value| value.parse().ok())
    .unwrap_or(179);
  let screen = Screen {
    columns,
    rows: 103,
    cell_width: 11,
    cell_height: 21,
    origin: (0, 0),
  };
  let mut reports = Vec::new();
  for file in &files {
    reports.push(document(file, screen)?);
  }
  let output = serde_json::to_string_pretty(&reports)?;
  if let Ok(path) = std::env::var("VPDF_BENCH_OUTPUT") {
    std::fs::write(path, &output)?;
  }
  println!("{output}");
  Ok(())
}
