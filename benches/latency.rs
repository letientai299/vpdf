//! Cold-path latency: open, jumps, zoom, and paced scrolling through the
//! reader with a real render worker. Frames go to memory, not a terminal.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_reader::motion::Motion as Move;
use vpdf_ui::find::Phase;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Screen;

const FRAME: Duration = Duration::from_micros(8333);
const SETTLE: Duration = Duration::from_secs(10);
/// A full scan of a 5,000-page manual stays well inside this.
const SCAN: Duration = Duration::from_secs(120);

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
fn missing(app: &mut Solo) -> usize {
  let keys = vpdf_ui::layout::visible(&app.layout, app.reader.scroll);
  keys.iter().filter(|&&key| !app.graphics.contains(&(app.id, key))).count()
}

/// One loop iteration of the session: results, animation, frame.
struct Frame {
  build: f64,
  bytes: usize,
  missing: usize,
}

struct Sim {
  app: Solo,
  frame: Vec<u8>,
  uploaded: usize,
}

impl Sim {
  fn tick(&mut self) -> Result<Frame> {
    self.app.search_updates()?;
    self.app.outline_updates()?;
    self.app.crop_updates()?;
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
      if frame.missing == 0
        && !self.app.reader.animation().active()
        && !self.cropping()
      {
        let done = ms(started.elapsed());
        return Ok((first.unwrap_or(done), done));
      }
      anyhow::ensure!(started.elapsed() < SETTLE, "Viewport never settled");
      std::thread::sleep(FRAME);
    }
  }

  fn act(&mut self, action: Action) -> Result<f64> {
    let started = Instant::now();
    self.app.action(Input::plain(action))?;
    Ok(ms(started.elapsed()))
  }

  fn cropping(&self) -> bool {
    self.app.crop_scan.active()
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
  ready_ms: f64,
  complete_ms: f64,
}

#[derive(Serialize)]
struct Find {
  query: &'static str,
  hits: usize,
  first_hit_ms: f64,
  first_view_ms: f64,
  done_ms: f64,
  next: Jumps,
}

#[derive(Serialize)]
struct Report {
  document: String,
  pages: usize,
  screen: (u16, u16),
  open_ms: f64,
  open_complete_ms: f64,
  jumps: Jumps,
  idle_jumps: Jumps,
  scan_jumps: Jumps,
  /// The scan outlasted `scan_jumps`.
  scanning: bool,
  zoom: Jumps,
  ends: Jumps,
  history: Jumps,
  sections: Jumps,
  finds: Vec<Find>,
  motions: Vec<Motion>,
}

/// Distinct pseudo-random pages, reproducible across runs.
fn pages(count: usize, total: usize, mut seed: u64) -> Vec<usize> {
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

fn jumps(sim: &mut Sim, count: usize, seed: u64) -> Result<Jumps> {
  let (mut input, mut first, mut complete) = (vec![], vec![], vec![]);
  for page in pages(count, sim.app.doc.pages.len(), seed) {
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

/// The same pages in a fresh reader, with or without a full search scan
/// running, which shares `MuPDF`'s locks; whether the scan outlasted them.
fn fresh_jumps(
  path: &Path,
  screen: Screen,
  scan: bool,
) -> Result<(Jumps, bool)> {
  let app = Solo::open(path, "", false, screen)?;
  let mut sim = Sim { app, frame: Vec::new(), uploaded: 0 };
  sim.settle(Instant::now())?;
  if scan {
    sim.app.start_search("zkqxjv", false, false)?;
  }
  let jumps = jumps(&mut sim, 10, 0xc2b2_ae35)?;
  Ok((jumps, matches!(sim.app.search.phase(), Phase::Running { .. })))
}

/// Times each action from input to a complete view.
fn settled(
  sim: &mut Sim,
  actions: impl IntoIterator<Item = Action>,
) -> Result<Jumps> {
  let (mut input, mut first, mut complete) = (vec![], vec![], vec![]);
  for action in actions {
    let started = Instant::now();
    input.push(sim.act(action)?);
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

/// Sections forward once the outline loads; none without one.
fn sections(sim: &mut Sim, count: usize) -> Result<Jumps> {
  let started = Instant::now();
  sim.app.want_outline();
  while sim.app.outline_pending() {
    sim.tick()?;
    anyhow::ensure!(started.elapsed() < SCAN, "Outline never loaded");
    std::thread::sleep(FRAME);
  }
  if sim.app.doc.outline.is_empty() {
    return settled(sim, []);
  }
  sim.app.command("1")?;
  sim.settle(Instant::now())?;
  // Steps past the last heading do nothing and would skew the timing.
  let count = count.min(sim.app.doc.outline.len().saturating_sub(1));
  settled(sim, std::iter::repeat_n(Action::Section(true), count))
}

/// Searches from page 1 to the scan's end, then steps through matches.
fn find(sim: &mut Sim, query: &'static str) -> Result<Find> {
  sim.app.command("1")?;
  sim.settle(Instant::now())?;
  let started = Instant::now();
  sim.app.start_search(query, false, false)?;
  let (mut hit, mut view) = (None, None);
  loop {
    let frame = sim.tick()?;
    let now = ms(started.elapsed());
    if hit.is_none() && !sim.app.search.hits().is_empty() {
      hit = Some(now);
    }
    if hit.is_some() && view.is_none() && frame.missing == 0 {
      view = Some(now);
    }
    if matches!(sim.app.search.phase(), Phase::Complete) && frame.missing == 0 {
      break;
    }
    anyhow::ensure!(started.elapsed() < SCAN, "Search never finished");
    std::thread::sleep(FRAME);
  }
  let done_ms = ms(started.elapsed());
  let hits = sim.app.search.hits().len();
  let steps = if hits == 0 { 0 } else { 10 };
  let next = settled(sim, std::iter::repeat_n(Action::Result(false), steps))?;
  Ok(Find {
    query,
    hits,
    first_hit_ms: hit.unwrap_or(done_ms),
    first_view_ms: view.unwrap_or(done_ms),
    done_ms,
    next,
  })
}

fn zooms(sim: &mut Sim) -> Result<Jumps> {
  let zoom = settled(sim, [1, 1, 1, -1, -1, -1].map(Action::Zoom))?;
  sim.act(Action::Fit)?;
  sim.settle(Instant::now())?;
  Ok(zoom)
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
  paced(sim, name, period, inputs, |sim, _| sim.act(action))
}

fn paced(
  sim: &mut Sim,
  name: &'static str,
  period: Duration,
  inputs: usize,
  mut act: impl FnMut(&mut Sim, usize) -> Result<f64>,
) -> Result<Motion> {
  let (mut input, mut build, mut bytes) = (vec![], vec![], vec![]);
  let (mut frames, mut incomplete, mut gap) = (0, 0, 0.0_f64);
  let mut blank_since: Option<Instant> = None;
  let uploaded = sim.uploaded;
  let started = Instant::now();
  let mut sent = 0;
  let (mut ready, mut complete) = (None, None);
  while sent < inputs {
    if started.elapsed() >= period * sent as u32 {
      input.push(act(sim, sent)?);
      sent += 1;
    }
    let frame = sim.tick()?;
    if !sim.cropping() {
      ready.get_or_insert_with(|| ms(started.elapsed()));
      if frame.missing == 0 {
        complete.get_or_insert_with(|| ms(started.elapsed()));
      }
    }
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
    ready_ms: ready.unwrap_or_else(|| ms(started.elapsed())),
    complete_ms: complete.unwrap_or_else(|| ms(started.elapsed())),
  })
}

fn motions(sim: &mut Sim) -> Result<Vec<Motion>> {
  let start = sim.app.doc.pages.len() / 3;
  sim.app.command(&(start + 1).to_string())?;
  sim.settle(Instant::now())?;
  let hz = |rate: u64| Duration::from_micros(1_000_000 / rate);
  let mut motions = vec![
    motion(sim, "rows3_60hz", Action::Rows(3), hz(60), 180)?,
    motion(sim, "rows1_120hz", Action::Rows(1), hz(120), 240)?,
    motion(sim, "half_10hz", Action::Scroll(0.5), hz(10), 30)?,
    motion(sim, "half_up_10hz", Action::Scroll(-0.5), hz(10), 30)?,
    motion(sim, "page_4hz", Action::Scroll(1.0), hz(4), 12)?,
    // A held Page Down at key-repeat rate.
    motion(sim, "page_30hz", Action::Scroll(1.0), hz(30), 60)?,
  ];
  sim.app.command(&(start + 1).to_string())?;
  sim.settle(Instant::now())?;
  motions.push(paced(sim, "direct_reverse_60hz", hz(60), 96, |sim, index| {
    let started = Instant::now();
    let step = if (index / 8) % 2 == 0 { 96.0 } else { -96.0 };
    let scroll = sim.app.layout.clamp_scroll(sim.app.reader.scroll + step);
    sim.app.reading_view().scrub_scroll(scroll);
    Ok(ms(started.elapsed()))
  })?);
  Ok(motions)
}

fn document(path: &Path, screen: Screen) -> Result<Report> {
  let started = Instant::now();
  let app = Solo::open(path, "", false, screen)?;
  let open_ms = ms(started.elapsed());
  let mut sim = Sim { app, frame: Vec::new(), uploaded: 0 };
  let (_, open_complete_ms) = sim.settle(started)?;
  let random = jumps(&mut sim, 20, 0x9e37_79b9)?;
  // An untimed pass reads the pages into the OS file cache for both.
  fresh_jumps(path, screen, false)?;
  let (idle_jumps, _) = fresh_jumps(path, screen, false)?;
  let (scan_jumps, scanning) = fresh_jumps(path, screen, true)?;
  let history =
    settled(&mut sim, std::iter::repeat_n(Action::History(false), 10))?;
  let zoom = zooms(&mut sim)?;
  let ends = settled(
    &mut sim,
    [Move::Last, Move::First].repeat(3).into_iter().map(Action::Motion),
  )?;
  let sections = sections(&mut sim, 15)?;
  // The last query repeats a full scan over cached indexes.
  let finds = ["qzxjvw", "the", "vwjxzq"]
    .into_iter()
    .map(|query| find(&mut sim, query))
    .collect::<Result<_>>()?;
  let motions = motions(&mut sim)?;
  Ok(Report {
    document: path.display().to_string(),
    pages: sim.app.doc.pages.len(),
    screen: (screen.columns, screen.rows),
    open_ms,
    open_complete_ms,
    jumps: random,
    idle_jumps,
    scan_jumps,
    scanning,
    zoom,
    ends,
    history,
    sections,
    finds,
    motions,
  })
}

#[derive(Serialize)]
struct CropReport {
  document: String,
  pages: usize,
  screen: ScreenSize,
  open_ms: f64,
  open_complete_ms: f64,
  crop_input_ms: f64,
  crop_ready_ms: f64,
  crop_view_ms: f64,
  plain_scroll: Motion,
  detect_scroll: Motion,
  cropped_scroll: Motion,
  cached_toggle: Jumps,
  restored_open_ms: f64,
  restored_first_ms: f64,
  restored_complete_ms: f64,
}

type ScreenSize = (u16, u16, u16, u16);

fn crop_document(path: &Path, screen: Screen) -> Result<CropReport> {
  let started = Instant::now();
  let app = Solo::open(path, "", false, screen)?;
  let open_ms = ms(started.elapsed());
  let mut sim = Sim { app, frame: Vec::new(), uploaded: 0 };
  let (_, open_complete_ms) = sim.settle(started)?;
  let page = (sim.app.doc.pages.len() / 3 + 1).min(100).to_string();
  sim.app.command(&page)?;
  sim.settle(Instant::now())?;
  let space = std::env::var_os("VPDF_CROP_SPACE").is_some();
  let (action, period, inputs) = if space {
    (Action::Screen(1), Duration::from_millis(33), 30)
  } else {
    (Action::Rows(1), Duration::from_micros(8333), 120)
  };
  let plain_scroll = motion(&mut sim, "plain", action, period, inputs)?;
  sim.app.command(&page)?;
  sim.settle(Instant::now())?;
  let crop_input_ms = sim.act(Action::Crop)?;
  let detect_scroll = motion(&mut sim, "detecting", action, period, inputs)?;
  let crop_ready_ms = crop_input_ms + detect_scroll.ready_ms;
  let crop_view_ms = crop_input_ms + detect_scroll.complete_ms;
  sim.app.command(&page)?;
  sim.settle(Instant::now())?;
  let cropped_scroll = motion(&mut sim, "cropped", action, period, inputs)?;
  let cached_toggle = settled(&mut sim, std::iter::repeat_n(Action::Crop, 10))?;
  sim.app.command(&page)?;
  sim.settle(Instant::now())?;
  let mut state = vpdf::state::State::default();
  state.remember(sim.app.saved_entry().expect("an uncovered pane"));
  let entry = state.recent[0].clone();
  let pages = sim.app.doc.pages.len();
  drop(sim);
  let started = Instant::now();
  let mut app = Solo::open(path, "", false, screen)?;
  app.restore(&entry)?;
  let restored_open_ms = ms(started.elapsed());
  let mut sim = Sim { app, frame: Vec::new(), uploaded: 0 };
  let (restored_first_ms, restored_complete_ms) = sim.settle(started)?;
  Ok(CropReport {
    document: path.display().to_string(),
    pages,
    screen: (
      screen.columns,
      screen.rows,
      screen.cell_width,
      screen.cell_height,
    ),
    open_ms,
    open_complete_ms,
    crop_input_ms,
    crop_ready_ms,
    crop_view_ms,
    plain_scroll,
    detect_scroll,
    cropped_scroll,
    cached_toggle,
    restored_open_ms,
    restored_first_ms,
    restored_complete_ms,
  })
}

fn main() -> Result<()> {
  let files: Vec<PathBuf> = match std::env::var_os("VPDF_EXTRA") {
    Some(extra) => std::env::split_paths(&extra).collect(),
    None => {
      let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
      ["book", "paper", "form", "ocr"]
        .iter()
        .map(|name| root.join(format!("{name}.pdf")))
        .collect()
    }
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
  let output = if std::env::var_os("VPDF_CROP_BENCH").is_some() {
    let screen = Screen {
      columns: 120,
      rows: 50,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let reports = files
      .iter()
      .map(|file| crop_document(file, screen))
      .collect::<Result<Vec<_>>>()?;
    serde_json::to_string_pretty(&reports)?
  } else {
    let reports = files
      .iter()
      .map(|file| document(file, screen))
      .collect::<Result<Vec<_>>>()?;
    serde_json::to_string_pretty(&reports)?
  };
  if let Ok(path) = std::env::var("VPDF_BENCH_OUTPUT") {
    std::fs::write(path, &output)?;
  }
  println!("{output}");
  Ok(())
}
