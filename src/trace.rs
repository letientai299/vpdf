use crate::Result;
use crate::app::App;
use crate::graphics::Graphics;
use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

pub struct Trace {
  output: Option<BufWriter<File>>,
  started: Instant,
  input: Option<Instant>,
  uploaded: u64,
}

#[derive(Serialize)]
struct Frame {
  elapsed_ms: f64,
  build_ms: f64,
  flush_ms: f64,
  input_to_flush_ms: Option<f64>,
  bytes: usize,
  base_upload_bytes: u64,
  page: usize,
  scroll: f32,
  /// Every visible tile of the focused pane was placed.
  complete: bool,
}

/// What a frame showed of the focused pane.
pub struct Observation {
  /// Image bytes sent so far.
  pub uploaded: u64,
  pub page: usize,
  pub scroll: f32,
  /// Every visible tile of the focused pane was placed.
  pub complete: bool,
}

impl Observation {
  pub fn of(app: &App, graphics: &Graphics) -> Self {
    Self {
      uploaded: graphics.uploaded,
      page: app.navigation_position().page.0 + 1,
      scroll: app.reader.scroll,
      complete: app.complete,
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
      .map(File::create)
      .transpose()?
      .map(BufWriter::new);
    Ok(Self { output, started: Instant::now(), input: None, uploaded: 0 })
  }

  pub fn input(&mut self, at: Instant) {
    self.input.get_or_insert(at);
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
    let seen = target.observe();
    let input = self.input.take();
    let Some(trace) = &mut self.output else { return Ok(()) };
    let frame = Frame {
      elapsed_ms: flushed.duration_since(self.started).as_secs_f64() * 1000.0,
      build_ms: built.duration_since(started).as_secs_f64() * 1000.0,
      flush_ms: flushed.duration_since(built).as_secs_f64() * 1000.0,
      input_to_flush_ms: input
        .map(|time| flushed.duration_since(time).as_secs_f64() * 1000.0),
      bytes: buffer.len(),
      base_upload_bytes: seen.uploaded.saturating_sub(self.uploaded),
      page: seen.page,
      scroll: seen.scroll,
      complete: seen.complete,
    };
    self.uploaded = seen.uploaded;
    serde_json::to_writer(&mut *trace, &frame)?;
    writeln!(trace)?;
    Ok(())
  }
}
