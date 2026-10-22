use crate::Result;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use vpdf_ui::layout::Screen;
use vpdf_ui::perf::{Overlay, Snapshot};

pub const INTERVAL: Duration = Duration::from_millis(100);
const SECOND: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
pub(crate) struct Resources {
  pub at: Instant,
  pub cpu: Option<f64>,
  pub rss: u64,
}

enum Signal {
  /// Back to busy sampling after an idle stretch.
  Wake,
  /// Idle: sample once a second.
  Rest,
  Stop,
}

pub(crate) struct Sampler {
  latest: Arc<Mutex<Option<Resources>>>,
  signal: mpsc::Sender<Signal>,
  thread: Option<JoinHandle<()>>,
}

impl Sampler {
  fn new() -> Self {
    Self::observe(|_| {})
  }

  pub(crate) fn observe(observe: impl Fn(Resources) + Send + 'static) -> Self {
    let latest = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&latest);
    let (signal, receiver) = mpsc::channel();
    let thread = thread::spawn(move || {
      let mut system = System::new();
      let pid = Pid::from_u32(std::process::id());
      let kind =
        ProcessRefreshKind::nothing().with_cpu().with_memory().without_tasks();
      // Short windows round a few ms of CPU to visible jitter.
      let mut history: VecDeque<(Instant, u64)> = VecDeque::new();
      let mut interval = INTERVAL;
      loop {
        system.refresh_processes_specifics(
          ProcessesToUpdate::Some(&[pid]),
          true,
          kind,
        );
        let at = Instant::now();
        let sample = system.process(pid).map(|process| {
          let total = process.accumulated_cpu_time();
          let cpu = windowed(&mut history, at, total);
          Resources { at, cpu, rss: process.memory() }
        });
        if let Ok(mut value) = shared.lock() {
          *value = sample;
        }
        if let Some(sample) = sample {
          observe(sample);
        }
        match receiver.recv_timeout(interval) {
          Ok(Signal::Wake) => interval = INTERVAL,
          Ok(Signal::Rest) => interval = SECOND,
          Err(mpsc::RecvTimeoutError::Timeout) => {}
          Ok(Signal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
            break;
          }
        }
      }
    });
    Self { latest, signal, thread: Some(thread) }
  }
}

impl Drop for Sampler {
  fn drop(&mut self) {
    let _ = self.signal.send(Signal::Stop);
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
  /// Overlay bytes written since the last count; Out leaves them out.
  own: usize,
  idle: bool,
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
      own: 0,
      idle: false,
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
    self.wake(at);
  }

  pub fn bytes(&mut self, at: Instant, bytes: usize) {
    if self.sampler.is_none() {
      return;
    }
    let bytes = bytes.saturating_sub(std::mem::take(&mut self.own));
    if bytes == 0 {
      return;
    }
    self.output.push_back((at, bytes));
    while self.output.len() > 2048 {
      self.output.pop_front();
    }
    self.wake(at);
  }

  /// Paints the overlay into a frame, uncounted by Out.
  pub fn draw(&mut self, out: &mut impl Write, screen: Screen) -> Result<bool> {
    let mut buffer = Vec::new();
    let drawn = crate::presentation::draw(&mut buffer, None, |scene| {
      self.view.scene(scene, screen, false)
    })?;
    self.exclude(buffer.len());
    out.write_all(&buffer)?;
    Ok(drawn)
  }

  pub fn exclude(&mut self, bytes: usize) {
    if self.sampler.is_some() {
      self.own += bytes;
    }
  }

  fn wake(&mut self, at: Instant) {
    if !self.idle {
      return;
    }
    self.idle = false;
    self.next = self.next.min(at + INTERVAL);
    self.signal(Signal::Wake);
  }

  fn signal(&self, signal: Signal) {
    if let Some(sampler) = &self.sampler {
      let _ = sampler.signal.send(signal);
    }
  }

  pub fn sample(&mut self, now: Instant) {
    if now < self.next {
      return;
    }
    self.frames.retain(|at| now.duration_since(*at) < SECOND);
    self.output.retain(|(at, _)| now.duration_since(*at) < SECOND);
    // Frames and output wake an idle monitor themselves.
    if !self.idle && self.frames.is_empty() && self.output.is_empty() {
      self.idle = true;
      self.signal(Signal::Rest);
    }
    self.next = now + if self.idle { SECOND } else { INTERVAL };
    let span = now.duration_since(self.started).as_secs_f64().min(1.0);
    let fps = self.frames.len();
    let resources = self
      .sampler
      .as_ref()
      .and_then(|s| s.latest.lock().ok().and_then(|value| *value))
      // Idle sampling runs once a second.
      .filter(|sample| now.saturating_duration_since(sample.at) < SECOND * 2);
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

/// CPU over the last second of samples.
fn windowed(
  history: &mut VecDeque<(Instant, u64)>,
  at: Instant,
  total: u64,
) -> Option<f64> {
  history.push_back((at, total));
  while history.len() > 2 && at.duration_since(history[1].0) >= SECOND {
    history.pop_front();
  }
  let first = history.front().copied().filter(|(time, _)| *time < at);
  cpu_percent(first, at, total)
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

  use crate::screen::cells as screen;

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
    crate::presentation::draw(&mut out, None, |scene| {
      monitor.view.scene(scene, screen(80, 30), false)
    })?;
    let text = String::from_utf8(out)?;
    assert!(text.contains("FPS 120"));
    assert!(text.contains("Out 1.0K/s"));
    monitor.sample(start + SECOND * 2);
    let mut out = Vec::new();
    crate::presentation::draw(&mut out, None, |scene| {
      monitor.view.scene(scene, screen(80, 30), false)
    })?;
    let text = String::from_utf8(out)?;
    assert!(text.contains("FPS 0"));
    assert!(text.contains("Out 0.0K/s"));
    Ok(())
  }

  #[test]
  fn out_leaves_out_overlay_repaints() -> Result<()> {
    let mut monitor = Monitor::default();
    monitor.running(true);
    let start = monitor.started;
    let mut frame = Vec::new();
    assert!(monitor.draw(&mut frame, screen(80, 30))?);
    monitor.exclude(16);
    monitor.bytes(start, frame.len() + 16);
    assert!(monitor.output.is_empty());
    monitor.bytes(start, 100);
    assert_eq!(monitor.output.iter().map(|(_, n)| n).sum::<usize>(), 100);
    Ok(())
  }

  #[test]
  fn idle_samples_once_a_second_until_a_frame() {
    let mut monitor = Monitor::default();
    monitor.running(true);
    let start = monitor.started;
    monitor.sample(start);
    assert!(monitor.idle);
    assert_eq!(monitor.wait(start), SECOND);
    let at = start + INTERVAL;
    monitor.frame(at);
    assert!(!monitor.idle);
    assert_eq!(monitor.wait(at), INTERVAL);
    monitor.sample(at + INTERVAL);
    assert_eq!(monitor.wait(at + INTERVAL), INTERVAL);
  }

  #[test]
  fn cpu_spans_the_last_second() {
    let start = Instant::now();
    let mut history = VecDeque::new();
    assert_eq!(windowed(&mut history, start, 0), None);
    for tick in 1..=20 {
      let at = start + INTERVAL * tick;
      // 5 ms of CPU in the first second only.
      let total = if tick <= 10 { u64::from(tick) / 2 } else { 5 };
      let cpu = windowed(&mut history, at, total).unwrap();
      if tick == 10 {
        assert!((cpu - 0.5).abs() < 1e-9, "{cpu}");
      }
    }
    assert_eq!(windowed(&mut history, start + INTERVAL * 21, 5), Some(0.0));
    assert!(history.len() <= 12);
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
    assert!(crate::presentation::draw(&mut out, None, |scene| monitor
      .view
      .scene(scene, screen(80, 30), false))?);
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
