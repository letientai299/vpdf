use crate::Result;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use vpdf_ui::perf::{Overlay, Snapshot};

pub const INTERVAL: Duration = Duration::from_millis(100);
const SECOND: Duration = Duration::from_secs(1);

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
  pub(crate) view: Overlay,
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
      view: Overlay::default(),
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
    self.view.update(Snapshot {
      fps: fps as f64 / span.max(0.001),
      cpu: resources.and_then(|r| r.cpu),
      rss: resources.map(|r| r.rss),
      bytes_per_second: output as f64 / span.max(0.001),
    });
  }
}

pub fn switch(argument: &str) -> Result<Option<bool>> {
  match argument {
    "" => Ok(None),
    "on" => Ok(Some(true)),
    "off" => Ok(Some(false)),
    _ => anyhow::bail!("Usage: :perf [on|off]"),
  }
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
  use vpdf_ui::layout::Screen;

  fn screen(columns: u16, rows: u16) -> Screen {
    Screen { columns, rows, cell_width: 8, cell_height: 16, origin: (0, 0) }
  }

  #[test]
  fn idle_expires_frames_and_output() -> Result<()> {
    let mut monitor = Monitor::default();
    let start = monitor.started;
    for i in 1..=120 {
      monitor.frames.push_back(start + Duration::from_millis(i * 8));
    }
    monitor.output.push_back((start + SECOND, 1024));
    monitor.sample(start + SECOND);
    let mut out = Vec::new();
    monitor.view.draw(&mut out, screen(80, 30), false)?;
    let text = String::from_utf8(out)?;
    assert!(text.contains("FPS 120"));
    assert!(text.contains("Out 1.0K/s"));
    monitor.sample(start + SECOND * 2);
    let mut out = Vec::new();
    monitor.view.draw(&mut out, screen(80, 30), false)?;
    let text = String::from_utf8(out)?;
    assert!(text.contains("FPS 0"));
    assert!(text.contains("Out 0.0K/s"));
    Ok(())
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
    assert!(monitor.view.draw(&mut out, screen(80, 30), false)?);
    assert!(monitor.frames.is_empty());
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
