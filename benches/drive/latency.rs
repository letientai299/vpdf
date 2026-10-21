use crate::runner::Driver;
use crate::stats::{Timing, timing};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use vpdf::Result;
use vpdf::scenario::{Step, Wait};
use vpdf::trace::Log;
use vpdf_model::geometry::Screen;

#[derive(Serialize)]
struct Jumps {
  enqueue: Timing,
  first_tile: Timing,
  complete: Timing,
}

#[derive(Serialize)]
struct Find {
  query: &'static str,
  hits: usize,
  first_hit_ms: Option<f64>,
  first_view_ms: Option<f64>,
  done_ms: f64,
  next: Jumps,
}

#[derive(Serialize)]
struct Motion {
  name: &'static str,
  frames: usize,
  incomplete_frames: usize,
  inputs: u32,
  input_events: usize,
  build: Timing,
  delivery: Timing,
  settle_ms: f64,
}

#[derive(Serialize)]
struct Report {
  backend: &'static str,
  revision: &'static str,
  document: String,
  sha256: String,
  pages: usize,
  screen: (u16, u16, u16, u16),
  trace: PathBuf,
  open_complete_ms: f64,
  jumps: Jumps,
  idle_jumps: Jumps,
  scan_jumps: Jumps,
  scanning: bool,
  zoom: Jumps,
  ends: Jumps,
  history: Jumps,
  sections: Jumps,
  finds: Vec<Find>,
  motions: Vec<Motion>,
}

fn keys(driver: &mut Driver, text: &str, id: usize) -> Result<()> {
  let Some(Step::Keys(keys)) = Step::parse(&format!("key {text}"))? else {
    anyhow::bail!("Invalid latency keys");
  };
  driver.keys(&keys, id)
}

fn page(driver: &mut Driver, page: usize) -> Result<()> {
  let digits = page
    .to_string()
    .chars()
    .map(|ch| ch.to_string())
    .collect::<Vec<_>>()
    .join(" ");
  keys(driver, &format!(": {digits} enter"), 0)
}

fn settled(driver: &mut Driver, actions: &[String]) -> Result<Jumps> {
  let (mut enqueue, mut first, mut complete) = (vec![], vec![], vec![]);
  for (index, action) in actions.iter().enumerate() {
    let started = Instant::now();
    keys(driver, action, index + 1)?;
    enqueue.push(started.elapsed().as_secs_f64() * 1000.0);
    let mut one = None;
    let all = driver.until(|frame| {
      if !frame.images.is_empty() {
        one.get_or_insert_with(|| started.elapsed().as_secs_f64() * 1000.0);
      }
      frame.complete && !frame.metrics.animating
    })?;
    first.push(one.unwrap_or(all));
    complete.push(all);
  }
  Ok(Jumps {
    enqueue: timing(enqueue),
    first_tile: timing(first),
    complete: timing(complete),
  })
}

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

fn jumps(driver: &mut Driver, count: usize, seed: u64) -> Result<Jumps> {
  let total = driver.last.as_ref().map_or(0, |frame| frame.metrics.pages);
  let actions = pages(count, total, seed)
    .into_iter()
    .map(|page| {
      let digits = (page + 1)
        .to_string()
        .chars()
        .map(|ch| ch.to_string())
        .collect::<Vec<_>>()
        .join(" ");
      format!(": {digits} enter")
    })
    .collect::<Vec<_>>();
  settled(driver, &actions)
}

fn fresh_jumps(
  path: &Path,
  screen: Screen,
  log: &Arc<Log>,
  scan: bool,
) -> Result<(Jumps, bool)> {
  let mut driver = Driver::open(
    vec![path.into()],
    screen,
    Arc::clone(log),
    Duration::from_secs(120),
  )?;
  driver.wait(&Wait::Idle)?;
  if scan {
    keys(&mut driver, "/ z k q x j v enter", 0)?;
  }
  let jumps = jumps(&mut driver, 10, 0xc2b2_ae35)?;
  let scanning =
    driver.last.as_ref().is_some_and(|frame| frame.metrics.searching);
  driver.finish()?;
  Ok((jumps, scanning))
}

fn sections(driver: &mut Driver) -> Result<Jumps> {
  keys(driver, "g o", 0)?;
  driver.wait(&Wait::Idle)?;
  let count = driver
    .last
    .as_ref()
    .map_or(0, |frame| frame.metrics.sections.saturating_sub(1).min(15));
  keys(driver, "esc", 0)?;
  page(driver, 1)?;
  driver.wait(&Wait::Idle)?;
  settled(driver, &vec!["] ]".into(); count])
}

fn find(driver: &mut Driver, query: &'static str) -> Result<Find> {
  page(driver, 1)?;
  driver.wait(&Wait::Idle)?;
  driver.mark(&format!("find-{query}"))?;
  let text =
    query.chars().map(|ch| ch.to_string()).collect::<Vec<_>>().join(" ");
  keys(driver, &format!("/ C-u {text} enter"), 0)?;
  let started = Instant::now();
  let (mut hit, mut view) = (None, None);
  let done_ms = driver.until(|frame| {
    let now = started.elapsed().as_secs_f64() * 1000.0;
    if frame.metrics.hits > 0 {
      hit.get_or_insert(now);
      if frame.complete {
        view.get_or_insert(now);
      }
    }
    !frame.metrics.searching && frame.complete
  })?;
  let hits = driver.last.as_ref().map_or(0, |frame| frame.metrics.hits);
  let next =
    settled(driver, &vec!["n".into(); if hits == 0 { 0 } else { 10 }])?;
  Ok(Find {
    query,
    hits,
    first_hit_ms: hit,
    first_view_ms: view,
    done_ms,
    next,
  })
}

fn motion(
  driver: &mut Driver,
  name: &'static str,
  keys: &str,
  hz: u32,
  inputs: u32,
) -> Result<Motion> {
  driver.mark(name)?;
  let checkpoint = driver.checkpoint();
  let text = format!(
    "repeat {}ms @{hz} key {keys}",
    u64::from(inputs) * 1000 / u64::from(hz)
  );
  let step = Step::parse(&text)?
    .ok_or_else(|| anyhow::anyhow!("Missing latency motion"))?;
  anyhow::ensure!(driver.step(&step, 0)?.is_continue(), "Latency motion quit");
  let started = Instant::now();
  driver.until(|frame| frame.complete && !frame.metrics.animating)?;
  let phase = driver.phase(&checkpoint);
  Ok(Motion {
    name,
    frames: phase.frames,
    incomplete_frames: phase.incomplete_frames,
    inputs,
    input_events: phase.inputs,
    build: phase.build,
    delivery: phase.delivery,
    settle_ms: started.elapsed().as_secs_f64() * 1000.0,
  })
}

fn motions(driver: &mut Driver) -> Result<Vec<Motion>> {
  let start = driver.last.as_ref().map_or(0, |frame| frame.metrics.pages / 3);
  page(driver, start + 1)?;
  driver.wait(&Wait::Idle)?;
  [
    ("rows3_60hz", "3 j", 60, 180),
    ("rows1_120hz", "j", 120, 240),
    ("half_10hz", "C-d", 10, 30),
    ("half_up_10hz", "C-u", 10, 30),
    ("page_4hz", "C-f", 4, 12),
    ("page_30hz", "pagedown", 30, 60),
  ]
  .into_iter()
  .map(|(name, keys, hz, inputs)| motion(driver, name, keys, hz, inputs))
  .collect()
}

fn document(path: &Path, screen: Screen, trace: PathBuf) -> Result<Report> {
  let sha256 = crate::hash::digest(&std::fs::read(path)?);
  let log = Arc::new(Log::open(&trace)?);
  log.mark(Instant::now(), "startup")?;
  let mut driver = Driver::open(
    vec![path.into()],
    screen,
    Arc::clone(&log),
    Duration::from_secs(300),
  )?;
  let open_complete_ms = driver.wait(&Wait::Idle)?;
  let pages = driver.last.as_ref().map_or(0, |frame| frame.metrics.pages);
  anyhow::ensure!(pages > 0, "Latency document did not open");
  driver.mark("jumps")?;
  let random = jumps(&mut driver, 20, 0x9e37_79b9)?;
  fresh_jumps(path, screen, &log, false)?;
  driver.mark("idle-jumps")?;
  let (idle_jumps, _) = fresh_jumps(path, screen, &log, false)?;
  driver.mark("scan-jumps")?;
  let (scan_jumps, scanning) = fresh_jumps(path, screen, &log, true)?;
  driver.mark("history")?;
  let history = settled(&mut driver, &vec!["C-o".into(); 10])?;
  driver.mark("zoom")?;
  let zoom =
    settled(&mut driver, &["+", "+", "+", "-", "-", "-"].map(String::from))?;
  keys(&mut driver, "=", 0)?;
  driver.wait(&Wait::Idle)?;
  driver.mark("ends")?;
  let ends = settled(
    &mut driver,
    &["G", "g g"].repeat(3).into_iter().map(String::from).collect::<Vec<_>>(),
  )?;
  driver.mark("sections")?;
  let sections = sections(&mut driver)?;
  let finds = ["qzxjvw", "the", "vwjxzq"]
    .into_iter()
    .map(|query| find(&mut driver, query))
    .collect::<Result<_>>()?;
  let motions = motions(&mut driver)?;
  driver.mark("done")?;
  driver.finish()?;
  drop(log);
  Ok(Report {
    backend: "native-bridge",
    revision: env!("VPDF_VERSION"),
    document: path.file_name().unwrap_or_default().to_string_lossy().into(),
    sha256,
    pages,
    screen: (
      screen.columns,
      screen.rows,
      screen.cell_width,
      screen.cell_height,
    ),
    trace,
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

pub fn run(files: &[PathBuf], screen: Screen) -> Result<String> {
  let home =
    std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is unset"))?;
  let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
  let cache =
    if cfg!(target_os = "macos") { "Library/Caches" } else { ".cache" };
  let root = PathBuf::from(home)
    .join(cache)
    .join("vpdf-bench")
    .join(format!("latency-{stamp}-{}", std::process::id()));
  std::fs::create_dir_all(&root)?;
  let reports = files
    .iter()
    .enumerate()
    .map(|(index, path)| {
      document(path, screen, root.join(format!("{index}.jsonl")))
    })
    .collect::<Result<Vec<_>>>()?;
  Ok(serde_json::to_string_pretty(&reports)?)
}
