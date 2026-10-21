//! Crop-specific component measurements.
use crate::stats::{Timing, timing};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Screen;

const FRAME: Duration = Duration::from_micros(8333);
const SETTLE: Duration = Duration::from_secs(10);

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
  let (mut ready, mut complete) = (None, None);
  while sent < inputs {
    if started.elapsed() >= period * sent as u32 {
      input.push(sim.act(action)?);
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

pub fn run(files: &[PathBuf]) -> Result<String> {
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
  Ok(serde_json::to_string_pretty(&reports)?)
}
