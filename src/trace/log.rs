use crate::Result;
use crate::perf::Sampler;
use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Log {
  _sampler: Sampler,
  output: Arc<Mutex<BufWriter<File>>>,
  started: Instant,
}

#[derive(Serialize)]
struct Row<'a, T> {
  kind: &'a str,
  t_ms: f64,
  elapsed_ms: f64,
  #[serde(flatten)]
  fields: T,
}

#[derive(Serialize)]
struct Clock {
  unix_ns: u128,
}

#[derive(Serialize)]
struct Sample {
  cpu: Option<f64>,
  rss: u64,
}

#[derive(Serialize)]
struct Input {
  step: u64,
}

#[derive(Serialize)]
struct Mark<'a> {
  name: &'a str,
}

pub struct Paint {
  pub outcome: serde_json::Value,
  pub started: Instant,
  pub built: Instant,
  pub build: std::time::Duration,
  pub busy: bool,
  pub fresh: bool,
  pub upload_bytes: usize,
  pub stand_ins: usize,
  pub missing: usize,
  pub missing_pixels: f32,
  pub stand_in_pixels: f32,
  pub complete: bool,
}

#[derive(Serialize)]
struct NativeFrame {
  #[serde(flatten)]
  outcome: serde_json::Value,
  paint_ms: f64,
  build_ms: f64,
  built_ms: f64,
  fresh: bool,
  upload_bytes: usize,
  stand_ins: usize,
  missing: usize,
  missing_pixels: f32,
  stand_in_pixels: f32,
  #[serde(flatten)]
  readiness: Readiness,
}

#[derive(Serialize)]
struct Readiness {
  busy: bool,
  blank: bool,
  complete: bool,
}

impl NativeFrame {
  fn of(paint: &Paint, started: Instant) -> Self {
    Self {
      outcome: paint.outcome.clone(),
      paint_ms: paint.started.elapsed().as_secs_f64() * 1000.0,
      build_ms: paint.build.as_secs_f64() * 1000.0,
      built_ms: paint.built.saturating_duration_since(started).as_secs_f64()
        * 1000.0,
      fresh: paint.fresh,
      upload_bytes: paint.upload_bytes,
      stand_ins: paint.stand_ins,
      missing: paint.missing,
      missing_pixels: paint.missing_pixels,
      stand_in_pixels: paint.stand_in_pixels,
      readiness: Readiness {
        busy: paint.busy,
        blank: paint.missing != 0,
        complete: paint.complete,
      },
    }
  }
}

impl Log {
  pub fn open(path: &Path) -> Result<Self> {
    let output = Arc::new(Mutex::new(BufWriter::new(File::create(path)?)));
    let shared = Arc::clone(&output);
    let started = Instant::now();
    let clock = Clock {
      unix_ns: SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
    };
    write(
      &mut *output
        .lock()
        .map_err(|_| std::io::Error::other("Trace writer lock poisoned"))?,
      &Row::new("clock", started, started, clock),
    )?;
    let sampler = Sampler::observe(move |sample| {
      let row = Row::new(
        "sample",
        started,
        sample.at,
        Sample { cpu: sample.cpu, rss: sample.rss },
      );
      if let Ok(mut output) = shared.lock()
        && let Err(error) = write(&mut *output, &row)
      {
        eprintln!("Trace sample failed: {error}");
      }
    });
    Ok(Self { _sampler: sampler, output, started })
  }

  pub fn elapsed_ms(&self, at: Instant) -> f64 {
    at.saturating_duration_since(self.started).as_secs_f64() * 1000.0
  }

  pub fn row(
    &self,
    kind: &str,
    at: Instant,
    fields: impl Serialize,
  ) -> Result<()> {
    let mut output = self
      .output
      .lock()
      .map_err(|_| std::io::Error::other("Trace writer lock poisoned"))?;
    write(&mut *output, &Row::new(kind, self.started, at, fields))
  }

  pub fn input(&self, at: Instant, step: u64) -> Result<()> {
    self.row("input", at, Input { step })
  }

  pub fn mark(&self, at: Instant, name: &str) -> Result<()> {
    self.row("mark", at, Mark { name })
  }

  pub fn paint(&self, paint: &Paint) -> Result<()> {
    self.row("frame", paint.started, NativeFrame::of(paint, self.started))
  }
}

impl<'a, T> Row<'a, T> {
  fn new(kind: &'a str, started: Instant, at: Instant, fields: T) -> Self {
    let t_ms = at.saturating_duration_since(started).as_secs_f64() * 1000.0;
    Self { kind, t_ms, elapsed_ms: t_ms, fields }
  }
}

fn write(output: &mut impl Write, row: &impl Serialize) -> Result<()> {
  serde_json::to_writer(&mut *output, row)?;
  writeln!(output)?;
  output.flush()?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::{Value, json};
  use std::time::Duration;

  #[test]
  fn rows_share_clock() -> Result<()> {
    let started = Instant::now();
    let at = started + Duration::from_millis(25);
    let mut bytes = Vec::new();
    write(&mut bytes, &Row::new("input", started, at, Input { step: 3 }))?;
    write(
      &mut bytes,
      &Row::new("mark", started, at, Mark { name: "a\"b\nc" }),
    )?;
    write(
      &mut bytes,
      &Row::new("sample", started, at, Sample { cpu: None, rss: 42 }),
    )?;
    let rows: Vec<Value> = String::from_utf8(bytes)?
      .lines()
      .map(serde_json::from_str)
      .collect::<std::result::Result<_, _>>()?;
    for row in &rows {
      assert_eq!(row["t_ms"], 25.0);
      assert_eq!(row["elapsed_ms"], row["t_ms"]);
    }
    assert_eq!(rows[0]["step"], 3);
    assert_eq!(rows[1]["name"], "a\"b\nc");
    assert_eq!(rows[2]["cpu"], Value::Null);
    assert_eq!(rows[2]["rss"], 42);
    Ok(())
  }

  #[test]
  fn native_readiness_is_explicit() -> Result<()> {
    let at = Instant::now();
    let paint = Paint {
      outcome: serde_json::json!({}),
      started: at,
      built: at,
      build: Duration::from_millis(2),
      busy: true,
      fresh: true,
      upload_bytes: 128,
      stand_ins: 2,
      missing: 1,
      missing_pixels: 12.0,
      stand_in_pixels: 8.0,
      complete: false,
    };
    let mut bytes = Vec::new();
    let mut frame = NativeFrame::of(&paint, at);
    frame.paint_ms = 1.0;
    write(&mut bytes, &Row::new("frame", at, at, &frame))?;
    let row: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(
      row,
      json!({"kind":"frame", "t_ms":0.0, "elapsed_ms":0.0,
      "paint_ms":1.0, "build_ms":2.0, "built_ms":0.0, "busy":true,
      "fresh":true, "upload_bytes":128,
      "stand_ins":2, "missing":1, "missing_pixels":12.0,
      "stand_in_pixels":8.0, "blank":true, "complete":false})
    );
    Ok(())
  }
}
