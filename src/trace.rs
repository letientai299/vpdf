use crate::Result;
use crate::app::App;
use serde::Serialize;
use std::io::Write;

mod log;
pub use log::{Coverage, Log, Paint};
use std::time::Instant;
use vpdf_terminal::graphics::Graphics;

pub struct Trace {
  output: Option<Log>,
  step: u64,
  input: Option<Instant>,
  uploaded: u64,
}

#[derive(Serialize)]
struct Frame {
  build_ms: f64,
  built_ms: f64,
  #[serde(flatten)]
  outcome: crate::session::native::Observation,
  flush_ms: f64,
  input_to_flush_ms: Option<f64>,
  bytes: usize,
  base_upload_bytes: u64,
  page: usize,
  scroll: f32,
  /// Every visible tile of the focused pane was placed.
  complete: bool,
  busy: bool,
  missing: usize,
  blank: bool,
  stand_ins: usize,
}

/// What a frame showed of the focused pane.
#[derive(Default)]
pub struct Observation {
  /// Image bytes sent so far.
  pub uploaded: u64,
  pub outcome: crate::session::native::Observation,
  pub page: usize,
  pub scroll: f32,
  /// Every visible tile of the focused pane was placed.
  pub complete: bool,
  pub busy: bool,
  pub missing: usize,
  pub stand_ins: usize,
}

impl Observation {
  pub(crate) fn pane(pane: &crate::session::Pane, graphics: &Graphics) -> Self {
    pane.document().map_or_else(
      || Self {
        uploaded: graphics.uploaded,
        outcome: crate::session::native::Observation::pane(pane),
        complete: true,
        missing: graphics.missing_tiles,
        stand_ins: graphics.stand_ins,
        ..Self::default()
      },
      |app| Self::of(app, graphics),
    )
  }

  pub fn of(app: &App, graphics: &Graphics) -> Self {
    Self {
      uploaded: graphics.uploaded,
      outcome: crate::session::native::Observation::of(app),
      page: app.navigation_position().page.0 + 1,
      scroll: app.reader.scroll,
      complete: app.complete,
      busy: app.busy(),
      missing: graphics.missing_tiles,
      stand_ins: graphics.stand_ins,
    }
  }
}

/// Something that draws whole frames around a focused pane.
pub trait Drawable {
  fn draw_frame(&mut self, out: &mut Vec<u8>) -> Result<()>;
  fn observe(&self) -> Observation;
  fn flushed(&mut self, _at: Instant) {}
}

impl Trace {
  pub fn new() -> Result<Self> {
    let output = std::env::var_os("VPDF_TRACE")
      .filter(|path| !path.is_empty())
      .map(|path| Log::open(path.as_ref()))
      .transpose()?;
    Ok(Self { output, step: 0, input: None, uploaded: 0 })
  }

  pub fn input(&mut self, at: Instant) -> Result<()> {
    self.input.get_or_insert(at);
    self.step += 1;
    if let Some(output) = &self.output {
      output.input(at, self.step)?;
    }
    Ok(())
  }

  #[cfg(feature = "bench-events")]
  pub fn benchmark(
    &self,
    event: &vpdf_terminal::decode::Event,
  ) -> Result<bool> {
    use base64::Engine;
    use vpdf_terminal::decode::{Event, Reply};
    let Event::Reply(Reply::Osc { code: 777, body }) = event else {
      return Ok(false);
    };
    let Some(encoded) = body.strip_prefix("vpdf-bench-mark=") else {
      return Ok(false);
    };
    let Some(output) = &self.output else { return Ok(false) };
    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let name = String::from_utf8(bytes)?;
    output.mark(Instant::now(), &name)?;
    Ok(true)
  }

  pub fn draw(
    &mut self,
    target: &mut impl Drawable,
    output: &mut impl Write,
    buffer: &mut Vec<u8>,
  ) -> Result<()> {
    buffer.clear();
    let started = Instant::now();
    target.draw_frame(buffer)?;
    let built = Instant::now();
    output.write_all(buffer)?;
    output.flush()?;
    let flushed = Instant::now();
    target.flushed(flushed);
    let input = self.input.take();
    let Some(trace) = &self.output else { return Ok(()) };
    let seen = target.observe();
    let frame = Frame {
      build_ms: built.duration_since(started).as_secs_f64() * 1000.0,
      built_ms: trace.elapsed_ms(started),
      outcome: seen.outcome,
      flush_ms: flushed.duration_since(built).as_secs_f64() * 1000.0,
      input_to_flush_ms: input
        .map(|time| flushed.duration_since(time).as_secs_f64() * 1000.0),
      bytes: buffer.len(),
      base_upload_bytes: seen.uploaded.saturating_sub(self.uploaded),
      page: seen.page,
      scroll: seen.scroll,
      complete: seen.complete,
      busy: seen.busy,
      missing: seen.missing,
      blank: seen.missing != 0,
      stand_ins: seen.stand_ins,
    };
    self.uploaded = seen.uploaded;
    trace.row("frame", flushed, &frame)
  }
}

#[cfg(all(test, feature = "bench-events"))]
mod tests {
  use super::*;
  use vpdf_terminal::decode::{Event, Reply};

  #[test]
  fn benchmark_marks_use_reader_time() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("trace.jsonl");
    let trace = Trace {
      output: Some(Log::open(&path)?),
      step: 0,
      input: None,
      uploaded: 0,
    };
    let mark =
      |code, body: &str| Event::Reply(Reply::Osc { code, body: body.into() });
    assert!(!trace.benchmark(&mark(777, "notification"))?);
    assert!(!trace.benchmark(&mark(778, "vpdf-bench-mark=cGhhc2U="))?);
    assert!(trace.benchmark(&mark(777, "vpdf-bench-mark=cGhhc2U="))?);
    drop(trace);
    let rows = std::fs::read_to_string(path)?;
    let rows: Vec<serde_json::Value> = rows
      .lines()
      .map(serde_json::from_str)
      .collect::<std::result::Result<_, _>>(
    )?;
    assert!(rows.iter().any(|row| {
      row["kind"] == "mark"
        && row["name"] == "phase"
        && row["t_ms"].as_f64().is_some_and(|time| time >= 0.0)
    }));
    Ok(())
  }
}

#[cfg(test)]
mod keys {
  use super::*;

  fn keys(value: &serde_json::Value) -> Vec<&str> {
    value.as_object().expect("object").keys().map(String::as_str).collect()
  }

  #[test]
  fn frame_keys_stay_stable() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("doc.pdf");
    crate::fixtures::pdf(&path, &["first"], "")?;
    let mut session = crate::session::Session::new(None);
    session.open(&path, false, crate::screen::cells(80, 24))?;
    let seen = session.observe();
    let frame = Frame {
      build_ms: 0.0,
      built_ms: 0.0,
      outcome: seen.outcome,
      flush_ms: 0.0,
      input_to_flush_ms: None,
      bytes: 0,
      base_upload_bytes: 0,
      page: seen.page,
      scroll: seen.scroll,
      complete: seen.complete,
      busy: seen.busy,
      missing: seen.missing,
      blank: false,
      stand_ins: 0,
    };
    let row = serde_json::to_value(&frame)?;
    assert_eq!(
      keys(&row),
      [
        "animating",
        "base_upload_bytes",
        "blank",
        "build_ms",
        "built_ms",
        "busy",
        "bytes",
        "complete",
        "document",
        "document_generation",
        "error",
        "flush_ms",
        "hits",
        "input_to_flush_ms",
        "missing",
        "overview",
        "page",
        "pages",
        "pane",
        "pane_count",
        "panel",
        "query",
        "query_generation",
        "resources",
        "scroll",
        "search_outcome",
        "selected_hit",
        "stand_ins",
        "tab",
      ]
    );
    assert_eq!(
      keys(&row["resources"][0]),
      [
        "active_workers",
        "owner",
        "search_workers",
        "text_bytes",
        "tile_bytes",
        "visible",
      ]
    );
    Ok(())
  }
}
