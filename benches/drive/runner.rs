use crate::stats::{Timing, timing};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::scenario::{Step, Wait};
use vpdf::session::native::{Bridge, Frame, Input, Ticket};
use vpdf::trace::Log;
use vpdf_model::geometry::Screen;
use vpdf_model::input::{KeyModifiers, MouseEvent, MouseEventKind};
use vpdf_model::scene::Op;

mod watchdog;
use watchdog::Watchdog;

pub struct Driver {
  pub bridge: Bridge,
  _watchdog: Watchdog,
  wakes: mpsc::Receiver<()>,
  trace: Arc<Log>,
  deadline: Instant,
  pub last: Option<Frame>,
  ticket: Ticket,
  submitted: Instant,
  screen: Screen,
  observing: Option<Instant>,
  checkpoints: HashMap<String, serde_json::Value>,
  frames: usize,
  incomplete: usize,
  inputs: usize,
  builds: Vec<f64>,
  deliveries: Vec<f64>,
  waits: Vec<WaitRecord>,
  _store: tempfile::TempDir,
}

#[derive(Serialize)]
pub struct WaitRecord {
  step: usize,
  mode: &'static str,
  complete_ms: f64,
}

#[derive(Default)]
pub struct Checkpoint {
  frames: usize,
  incomplete: usize,
  inputs: usize,
}

#[derive(Serialize)]
pub struct Phase {
  pub frames: usize,
  pub incomplete_frames: usize,
  pub inputs: usize,
  pub build: Timing,
  pub delivery: Timing,
}

#[derive(Serialize)]
pub struct Summary {
  #[serde(flatten)]
  phase: Phase,
  complete: Timing,
}

#[derive(Serialize)]
struct FrameRow {
  fresh: bool,
  build_ms: f64,
  built_ms: f64,
  observation_ms: Option<f64>,
  scroll: f32,
  delivery_ms: f64,
  #[serde(flatten)]
  readiness: Readiness,
  #[serde(flatten)]
  activity: Activity,
  page: Option<usize>,
  pages: usize,
  hits: usize,
  ticket: Ticket,
  #[serde(flatten)]
  observation: serde_json::Value,
}

#[derive(Serialize)]
struct Readiness {
  complete: bool,
  busy: bool,
  blank: bool,
  missing: usize,
  stand_ins: usize,
  missing_pixels: f32,
  stand_in_pixels: f32,
}

#[derive(Serialize)]
struct Activity {
  animating: bool,
  searching: bool,
}

impl Driver {
  pub fn open(
    paths: Vec<PathBuf>,
    screen: Screen,
    trace: Arc<Log>,
    timeout: Duration,
  ) -> Result<Self> {
    let started = Instant::now();
    let deadline = started + timeout;
    let watchdog = Watchdog::new(deadline);
    let store = tempfile::tempdir()?;
    let (sender, wakes) = mpsc::sync_channel(1);
    let bridge = Bridge::stored(
      paths,
      Some(store.path().join("state")),
      None,
      move || {
        let _ = sender.try_send(());
      },
    )?;
    bridge.send(Input::Observe(true));
    let ticket = bridge.send(Input::Resize(screen));
    Ok(Self {
      bridge,
      _watchdog: watchdog,
      wakes,
      trace,
      deadline,
      last: None,
      ticket,
      submitted: started,
      screen,
      observing: None,
      checkpoints: HashMap::new(),
      frames: 0,
      incomplete: 0,
      inputs: 0,
      builds: Vec::new(),
      deliveries: Vec::new(),
      waits: Vec::new(),
      _store: store,
    })
  }

  pub fn poll(&mut self) -> Result<()> {
    let Some(frame) = self.bridge.take() else { return Ok(()) };
    let at = Instant::now();
    let ready: HashSet<_> = frame.images.iter().map(|image| image.id).collect();
    let missing_areas: Vec<_> = frame
      .scene
      .iter()
      .filter_map(|op| {
        let Op::Tile(tile) = op else { return None };
        (!ready.contains(&tile.id())).then(|| tile.visible()).flatten()
      })
      .collect();
    let missing = missing_areas.len();
    let missing_pixels =
      missing_areas.iter().map(|r| r.width() * r.height()).sum();
    let build_ms = frame.metrics.build.as_secs_f64() * 1000.0;
    let delivery_ms =
      at.saturating_duration_since(frame.metrics.built).as_secs_f64() * 1000.0;
    self.trace.row(
      "frame",
      at,
      FrameRow {
        fresh: frame.content,
        observation_ms: frame
          .metrics
          .observation_time
          .map(|time| time.as_secs_f64() * 1000.0),
        scroll: frame.metrics.scroll,
        build_ms,
        built_ms: self.trace.elapsed_ms(frame.metrics.built),
        delivery_ms,
        readiness: Readiness {
          complete: frame.complete,
          busy: frame.metrics.busy,
          blank: missing != 0,
          missing,
          stand_ins: 0,
          missing_pixels,
          stand_in_pixels: 0.0,
        },
        activity: Activity {
          animating: frame.metrics.animating,
          searching: frame.metrics.searching,
        },
        page: frame.metrics.page.map(|page| page.0 + 1),
        pages: frame.metrics.pages,
        hits: frame.metrics.hits,
        ticket: frame.metrics.ticket,
        observation: serde_json::to_value(&frame.metrics.observation)?,
      },
    )?;
    self.frames += 1;
    self.incomplete += usize::from(!frame.complete);
    self.builds.push(build_ms);
    self.deliveries.push(delivery_ms);
    self.bridge.send(Input::Presented { content: frame.content, bytes: 0 });
    self.last = Some(frame);
    Ok(())
  }

  fn block(&mut self, until: Instant) -> Result<()> {
    anyhow::ensure!(
      Instant::now() < self.deadline,
      "Scenario deadline expired"
    );
    match self.wakes.recv_timeout(
      until.min(self.deadline).saturating_duration_since(Instant::now()),
    ) {
      Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => Ok(()),
      Err(mpsc::RecvTimeoutError::Disconnected) => {
        self.bridge.close()?;
        anyhow::bail!("Session exited before scenario completion")
      }
    }
  }

  pub fn pump(&mut self, duration: Duration) -> Result<()> {
    anyhow::ensure!(
      duration <= self.deadline.saturating_duration_since(Instant::now()),
      "Scenario wait exceeds deadline"
    );
    let until = Instant::now() + duration;
    loop {
      self.poll()?;
      if Instant::now() >= until {
        return Ok(());
      }
      self.block(until)?;
    }
  }

  pub fn submit(&mut self, input: Input, step: usize) -> Result<()> {
    self.submitted = Instant::now();
    self.ticket = self.bridge.send(input);
    self.inputs += 1;
    self.trace.row(
      "input",
      self.submitted,
      serde_json::json!({"step":step, "ticket":self.ticket}),
    )
  }

  pub fn keys(
    &mut self,
    keys: &[vpdf_model::input::KeyEvent],
    step: usize,
  ) -> Result<()> {
    for key in keys {
      self.submit(Input::Key(*key), step)?;
    }
    Ok(())
  }

  pub fn mark(&mut self, name: &str) -> Result<()> {
    let (sender, receiver) = mpsc::channel();
    self.ticket = self.bridge.send(Input::Mark {
      name: name.into(),
      trace: Arc::clone(&self.trace),
      ack: Some(sender),
    });
    receiver
      .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))?
      .map_err(anyhow::Error::msg)?;
    self.until(|_| true)?;
    if let Some(frame) = &self.last {
      self.checkpoints.insert(name.into(), observed(frame));
    }
    Ok(())
  }

  pub fn wait(&mut self, mode: &Wait) -> Result<f64> {
    if let Wait::Time(duration) = mode {
      self.pump(*duration)?;
      return Ok(self.submitted.elapsed().as_secs_f64() * 1000.0);
    }
    self.until(|frame| {
      frame.complete && (matches!(mode, Wait::Complete) || !frame.metrics.busy)
    })
  }

  pub fn until(
    &mut self,
    mut accept: impl FnMut(&Frame) -> bool,
  ) -> Result<f64> {
    loop {
      self.poll()?;
      if let Some(error) =
        self.last.as_ref().and_then(|f| f.metrics.observation.error.as_ref())
      {
        anyhow::bail!("Scenario job failed: {error}");
      }
      if self.last.as_ref().is_some_and(|frame| {
        frame.metrics.ticket >= self.ticket && accept(frame)
      }) {
        return Ok(self.submitted.elapsed().as_secs_f64() * 1000.0);
      }
      if let Some(mut due) = self.observing {
        if Instant::now() >= due {
          self.bridge.send(Input::Snapshot);
          due = Instant::now() + Duration::from_millis(100);
          self.observing = Some(due);
        }
        self.block(due.min(self.deadline))?;
      } else {
        self.block(self.deadline)?;
      }
    }
  }

  pub fn step(&mut self, step: &Step, id: usize) -> Result<ControlFlow<()>> {
    match step {
      Step::Expect(expectation) => {
        let baseline = if let Some(name) = expectation.checkpoint() {
          Some(
            self
              .checkpoints
              .get(name)
              .ok_or_else(|| {
                anyhow::anyhow!("Scenario checkpoint missing: {name}")
              })?
              .clone(),
          )
        } else {
          None
        };
        self.observing = Some(Instant::now());
        self
          .until(|frame| {
            expectation.matches_from(&observed(frame), baseline.as_ref())
          })
          .map_err(|error| {
            anyhow::anyhow!("Expectation {expectation:?} failed: {error}")
          })?;
        self.observing = None;
      }
      Step::Keys(keys) => self.keys(keys, id)?,
      Step::Wheel(x, y) => self.wheel(*x, *y, id)?,
      Step::Resize(width, height) => {
        self.screen.columns =
          (width / u32::from(self.screen.cell_width)) as u16;
        self.screen.rows = (height / u32::from(self.screen.cell_height)) as u16;
        anyhow::ensure!(
          self.screen.columns > 0 && self.screen.rows >= 2,
          "Scenario resize grid is too small"
        );
        self.submit(Input::Resize(self.screen), id)?;
      }
      Step::Place(_, _) => self.trace.row(
        "input",
        Instant::now(),
        serde_json::json!({"step":id, "skipped":"headless placement"}),
      )?,
      Step::Mark(name) => self.mark(name)?,
      Step::Wait(mode) => {
        let complete_ms = self.wait(mode)?;
        let mode = match mode {
          Wait::Complete => "complete",
          Wait::Idle => "idle",
          Wait::Time(_) => "time",
        };
        self.waits.push(WaitRecord { step: id, mode, complete_ms });
      }
      Step::Repeat { duration, hz, step } => {
        self.repeat(*duration, *hz, step, id)?;
      }
      Step::Mix { seed, keys } => {
        let mut seed = *seed;
        self.choice(keys, &mut seed, id)?;
      }
      Step::Quit => {
        self.bridge.close()?;
        return Ok(ControlFlow::Break(()));
      }
    }
    Ok(ControlFlow::Continue(()))
  }

  fn choice(
    &mut self,
    keys: &[Vec<vpdf_model::input::KeyEvent>],
    seed: &mut u64,
    id: usize,
  ) -> Result<()> {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    self.keys(&keys[(*seed >> 33) as usize % keys.len()], id)
  }

  fn wheel(&mut self, x: i16, y: i16, id: usize) -> Result<()> {
    let screen = self.screen;
    let (column, row) = (screen.columns / 2, screen.rows / 2);
    for (delta, positive, negative) in [
      (x, MouseEventKind::ScrollLeft, MouseEventKind::ScrollRight),
      (y, MouseEventKind::ScrollUp, MouseEventKind::ScrollDown),
    ] {
      for _ in 0..delta.unsigned_abs() {
        self.submit(
          Input::Mouse {
            event: MouseEvent {
              kind: if delta > 0 { positive } else { negative },
              column,
              row,
              modifiers: KeyModifiers::NONE,
            },
            pixels: (column * screen.cell_width, row * screen.cell_height),
            screen,
          },
          id,
        )?;
      }
    }
    Ok(())
  }

  fn repeat(
    &mut self,
    duration: Duration,
    hz: u32,
    step: &Step,
    id: usize,
  ) -> Result<()> {
    anyhow::ensure!(
      duration <= self.deadline.saturating_duration_since(Instant::now()),
      "Scenario repeat exceeds deadline"
    );
    let started = Instant::now();
    let mut index = 0_u32;
    let mut seed = if let Step::Mix { seed, .. } = step { *seed } else { 0 };
    loop {
      let due = Duration::from_secs_f64(f64::from(index) / f64::from(hz));
      if due >= duration {
        break;
      }
      self.pump(due.saturating_sub(started.elapsed()))?;
      match step {
        Step::Keys(keys) => self.keys(keys, id)?,
        Step::Wheel(x, y) => self.wheel(*x, *y, id)?,
        Step::Mix { keys, .. } => self.choice(keys, &mut seed, id)?,
        _ => anyhow::bail!("Invalid repeated scenario step"),
      }
      index += 1;
    }
    self.pump(duration.saturating_sub(started.elapsed()))
  }

  pub fn checkpoint(&self) -> Checkpoint {
    Checkpoint {
      frames: self.frames,
      incomplete: self.incomplete,
      inputs: self.inputs,
    }
  }

  pub fn phase(&self, checkpoint: &Checkpoint) -> Phase {
    let end = self.checkpoint();
    Phase {
      frames: end.frames - checkpoint.frames,
      incomplete_frames: end.incomplete - checkpoint.incomplete,
      inputs: end.inputs - checkpoint.inputs,
      build: timing(self.builds[checkpoint.frames..].to_vec()),
      delivery: timing(self.deliveries[checkpoint.frames..].to_vec()),
    }
  }

  pub fn summary(&self) -> Summary {
    Summary {
      phase: self.phase(&Checkpoint::default()),
      complete: timing(
        self
          .waits
          .iter()
          .filter(|wait| wait.mode != "time")
          .map(|wait| wait.complete_ms)
          .collect(),
      ),
    }
  }

  pub fn finish(mut self) -> Result<(Summary, Vec<WaitRecord>)> {
    self.bridge.close()?;
    let summary = self.summary();
    Ok((summary, std::mem::take(&mut self.waits)))
  }
}

fn observed(frame: &Frame) -> serde_json::Value {
  let mut row = serde_json::to_value(&frame.metrics.observation)
    .expect("serializable observation");
  row["scroll"] = frame.metrics.scroll.into();
  row["page"] = frame.metrics.page.map(|p| p.0 + 1).into();
  row["pages"] = frame.metrics.pages.into();
  row["hits"] = frame.metrics.hits.into();
  row["busy"] = frame.metrics.busy.into();
  row["animating"] = frame.metrics.animating.into();
  row["complete"] = frame.complete.into();
  let ready: HashSet<_> = frame.images.iter().map(|image| image.id).collect();
  row["missing"] = frame
    .scene
    .iter()
    .filter(|op| {
      matches!(op, Op::Tile(tile) if tile.visible().is_some()
              && !ready.contains(&tile.id()))
    })
    .count()
    .into();
  row["stand_ins"] = 0.into();
  row
}
