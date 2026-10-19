use crate::Result;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use vpdf_terminal::terminal::{self, At};
use vpdf_ui::layout::Screen;
use vpdf_ui::theme;

pub const INTERVAL: Duration = Duration::from_millis(100);
const SECOND: Duration = Duration::from_secs(1);
const CELLS: usize = 4;
const WIDTH: u16 = 16;

#[derive(Clone, Copy)]
struct Resources {
  at: Instant,
  cpu: Option<f64>,
  rss: u64,
}

struct Sampler {
  latest: Arc<Mutex<Option<Resources>>>,
  stop: mpsc::Sender<()>,
  thread: Option<JoinHandle<()>>,
}

impl Sampler {
  fn new() -> Self {
    let latest = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&latest);
    let (stop, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
      let mut system = System::new();
      let pid = Pid::from_u32(std::process::id());
      let kind =
        ProcessRefreshKind::nothing().with_cpu().with_memory().without_tasks();
      let mut previous: Option<(Instant, u64)> = None;
      loop {
        system.refresh_processes_specifics(
          ProcessesToUpdate::Some(&[pid]),
          true,
          kind,
        );
        let at = Instant::now();
        let sample = system.process(pid).map(|process| {
          let total = process.accumulated_cpu_time();
          let cpu = cpu_percent(previous, at, total);
          previous = Some((at, total));
          Resources { at, cpu, rss: process.memory() }
        });
        if let Ok(mut value) = shared.lock() {
          *value = sample;
        }
        if receiver.recv_timeout(INTERVAL)
          != Err(mpsc::RecvTimeoutError::Timeout)
        {
          break;
        }
      }
    });
    Self { latest, stop, thread: Some(thread) }
  }
}

impl Drop for Sampler {
  fn drop(&mut self) {
    let _ = self.stop.send(());
    if let Some(thread) = self.thread.take() {
      let _ = thread.join();
    }
  }
}

pub(crate) struct Monitor {
  sampler: Option<Sampler>,
  next: Instant,
  started: Instant,
  frames: VecDeque<Instant>,
  output: VecDeque<(Instant, usize)>,
  cells: Vec<String>,
  painted: Vec<String>,
  layout: Option<(u16, u16)>,
  changed: bool,
}

impl Default for Monitor {
  fn default() -> Self {
    let now = Instant::now();
    Self {
      sampler: None,
      next: now,
      started: now,
      frames: VecDeque::new(),
      output: VecDeque::new(),
      cells: Vec::new(),
      painted: Vec::new(),
      layout: None,
      changed: true,
    }
  }
}

impl Monitor {
  pub fn running(&mut self, on: bool) {
    if on && self.sampler.is_none() {
      *self = Self::default();
      self.sampler = Some(Sampler::new());
    } else if !on && self.sampler.is_some() {
      *self = Self::default();
    }
  }

  pub fn invalidate(&mut self) {
    self.layout = None;
  }

  pub fn wait(&self, now: Instant) -> Duration {
    self.next.saturating_duration_since(now)
  }

  pub fn frame(&mut self, at: Instant) {
    if self.sampler.is_none() {
      return;
    }
    self.frames.push_back(at);
    while self.frames.len() > 1024 {
      self.frames.pop_front();
    }
  }

  pub fn bytes(&mut self, at: Instant, bytes: usize) {
    if self.sampler.is_none() {
      return;
    }
    if bytes == 0 {
      return;
    }
    self.output.push_back((at, bytes));
    while self.output.len() > 2048 {
      self.output.pop_front();
    }
  }

  pub fn sample(&mut self, now: Instant) {
    if now < self.next {
      return;
    }
    self.next = now + INTERVAL;
    self.frames.retain(|at| now.duration_since(*at) < SECOND);
    self.output.retain(|(at, _)| now.duration_since(*at) < SECOND);
    let span = now.duration_since(self.started).as_secs_f64().min(1.0);
    let fps = self.frames.len();
    let resources = self
      .sampler
      .as_ref()
      .and_then(|s| s.latest.lock().ok().and_then(|value| *value))
      .filter(|sample| now.saturating_duration_since(sample.at) < SECOND);
    let output: usize = self.output.iter().map(|(_, bytes)| bytes).sum();
    self.changed = true;
    self.cells = vec![
      format!("FPS {:.0}", fps as f64 / span.max(0.001)),
      number("CPU", resources.and_then(|r| r.cpu), "%"),
      number("RSS", resources.map(|r| r.rss as f64 / 1_048_576.0), "M"),
      format!("Out {:.1}K/s", output as f64 / 1024.0 / span.max(0.001)),
    ];
  }

  pub fn draw(
    &mut self,
    out: &mut impl Write,
    screen: Screen,
    force: bool,
  ) -> Result<bool> {
    let layout = (screen.columns, screen.rows);
    if !force && !self.changed && self.layout == Some(layout) {
      return Ok(false);
    }
    let force = force || self.layout != Some(layout);
    self.layout = Some(layout);
    self.changed = false;
    let lines = self.lines(screen);
    if !force && lines == self.painted {
      return Ok(false);
    }
    write!(out, "\x1b7{}", theme::BAR)?;
    let top = 0;
    for (row, line) in lines.iter().enumerate() {
      terminal::line(
        out,
        At::default(),
        top + row as u16,
        line,
        screen.columns,
      )?;
    }
    write!(out, "{}\x1b8", theme::RESET)?;
    self.painted = lines;
    Ok(true)
  }

  fn lines(&self, screen: Screen) -> Vec<String> {
    let columns = usize::from((screen.columns / WIDTH).max(1));
    let width = usize::from(screen.columns.min(WIDTH));
    let mut cells = self.cells.clone();
    if cells.is_empty() {
      cells =
        ["FPS", "CPU", "RSS", "Out"].map(|label| format!("{label} —")).to_vec();
    }
    cells
      .chunks(columns)
      .take(usize::from(rows(screen)))
      .map(|chunk| {
        chunk.iter().fold(String::new(), |mut line, cell| {
          let cell: String = cell.chars().take(width).collect();
          let padding = width.saturating_sub(cell.chars().count());
          line.push_str(&cell);
          line.extend(std::iter::repeat_n(' ', padding));
          line
        })
      })
      .collect()
  }
}

pub fn rows(screen: Screen) -> u16 {
  let columns = (screen.columns / WIDTH).max(1);
  (CELLS as u16).div_ceil(columns).min(screen.rows.saturating_sub(3))
}

pub fn switch(argument: &str) -> Result<Option<bool>> {
  match argument {
    "" => Ok(None),
    "on" => Ok(Some(true)),
    "off" => Ok(Some(false)),
    _ => anyhow::bail!("Usage: :perf [on|off]"),
  }
}

fn number(label: &str, value: Option<f64>, unit: &str) -> String {
  value.map_or_else(
    || format!("{label} —"),
    |value| format!("{label} {value:.1}{unit}"),
  )
}

pub(crate) struct Output<W> {
  inner: W,
  bytes: usize,
}

impl<W> Output<W> {
  pub fn new(inner: W) -> Self {
    Self { inner, bytes: 0 }
  }
  pub fn take(&mut self) -> usize {
    std::mem::take(&mut self.bytes)
  }
}

impl<W: Write> Write for Output<W> {
  fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
    let written = self.inner.write(bytes)?;
    self.bytes += written;
    Ok(written)
  }
  fn flush(&mut self) -> std::io::Result<()> {
    self.inner.flush()
  }
}

fn cpu_percent(
  previous: Option<(Instant, u64)>,
  at: Instant,
  total: u64,
) -> Option<f64> {
  let (time, old) = previous?;
  let elapsed = at.duration_since(time).as_secs_f64();
  (elapsed > 0.0).then(|| total.saturating_sub(old) as f64 / elapsed / 10.0)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn screen(columns: u16, rows: u16) -> Screen {
    Screen { columns, rows, cell_width: 8, cell_height: 16, origin: (0, 0) }
  }

  #[test]
  fn wraps_and_preserves_content() {
    let mut monitor = Monitor::default();
    monitor.sample(Instant::now());
    for width in [22, 40, 80, 120, 286, 400] {
      let screen = screen(width, 30);
      let lines = monitor.lines(screen);
      assert_eq!(lines.len(), usize::from(rows(screen)));
      assert!(lines.iter().all(|line| line.chars().count() <= width as usize));
      let text = lines.join(" ");
      for label in ["FPS", "CPU", "RSS", "Out"] {
        assert_eq!(text.matches(label).count(), 1, "{width}: {label}");
      }
    }
    for height in 0..8 {
      for width in 0..22 {
        let screen = screen(width, height);
        assert!(rows(screen) <= height.saturating_sub(3));
        assert!(
          monitor
            .lines(screen)
            .iter()
            .all(|line| line.chars().count() <= width as usize)
        );
      }
    }
  }

  #[test]
  fn idle_expires_frames_and_output() {
    let mut monitor = Monitor::default();
    let start = monitor.started;
    for i in 1..=120 {
      monitor.frames.push_back(start + Duration::from_millis(i * 8));
    }
    monitor.output.push_back((start + SECOND, 1024));
    monitor.sample(start + SECOND);
    assert_eq!(monitor.cells[0], "FPS 120");
    assert_eq!(monitor.cells[3], "Out 1.0K/s");
    assert_eq!(monitor.cells.len(), 4);
    monitor.sample(start + SECOND * 2);
    assert_eq!(monitor.cells[0], "FPS 0");
    assert_eq!(monitor.cells[3], "Out 0.0K/s");
  }

  #[test]
  fn cpu_counts_cores_and_wall_time() {
    let now = Instant::now();
    let at = now + INTERVAL;
    assert_eq!(cpu_percent(None, at, 10), None);
    assert_eq!(cpu_percent(Some((now, 10)), at, 10), Some(0.0));
    assert_eq!(cpu_percent(Some((now, 10)), at, 110), Some(100.0));
    assert_eq!(cpu_percent(Some((now, 10)), at, 210), Some(200.0));
  }

  #[test]
  fn repaint_does_not_count_frames() -> Result<()> {
    let mut monitor = Monitor::default();
    let mut out = Vec::new();
    assert!(monitor.draw(&mut out, screen(80, 30), false)?);
    assert!(String::from_utf8_lossy(&out).contains("\x1b[1;1H"));
    out.clear();
    assert!(!monitor.draw(&mut out, screen(80, 30), false)?);
    assert_eq!(out, [] as [u8; 0]);
    assert!(monitor.frames.is_empty());
    assert!(monitor.draw(&mut out, screen(80, 31), false)?);
    assert!(String::from_utf8(out)?.contains("\x1b8"));
    Ok(())
  }

  #[test]
  fn disabled_has_no_sampling_or_frames() {
    let mut monitor = Monitor::default();
    monitor.frame(Instant::now());
    monitor.bytes(Instant::now(), 42);
    assert!(monitor.sampler.is_none());
    assert!(monitor.frames.is_empty());
    assert!(monitor.output.is_empty());
  }

  #[test]
  fn disabling_stops_sampling_and_clears_frames() {
    let mut monitor = Monitor::default();
    monitor.running(true);
    monitor.frame(Instant::now());
    assert_eq!(monitor.frames.len(), 1);
    monitor.running(false);
    assert!(monitor.sampler.is_none());
    assert!(monitor.frames.is_empty());
  }
}
