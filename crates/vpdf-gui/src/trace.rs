//! Native paint rows in the shared trace.

use crate::paint::Renderer;
use std::sync::Arc;
use std::time::Instant;
use vpdf::session::native::Frame;
use vpdf::trace::{Log, Paint};

pub struct Trace {
  output: Option<Arc<Log>>,
  step: u64,
}

impl Trace {
  pub fn from_env() -> Self {
    let output = std::env::var_os("VPDF_TRACE")
      .filter(|path| !path.is_empty())
      .and_then(|path| match Log::open(path.as_ref()) {
        Ok(log) => Some(Arc::new(log)),
        Err(error) => {
          eprintln!("Trace unavailable: {error}");
          None
        }
      });
    Self { output, step: 0 }
  }

  #[cfg(feature = "bench-events")]
  pub fn log(&self) -> Option<Arc<Log>> {
    self.output.clone()
  }

  pub fn enabled(&self) -> bool {
    self.output.is_some()
  }

  pub fn input(&mut self) {
    self.step += 1;
    if let Some(output) = &self.output {
      let result = output.input(Instant::now(), self.step);
      self.written(result);
    }
  }

  pub fn paint(
    &mut self,
    frame: &Frame,
    renderer: &Renderer,
    started: Instant,
    fresh: bool,
    upload_bytes: usize,
  ) {
    let Some(output) = &self.output else { return };
    if frame.metrics.observation_time.is_none() {
      return;
    }
    let mut outcome =
      serde_json::to_value(&frame.metrics.observation).unwrap_or_default();
    outcome["scroll"] = frame.metrics.scroll.into();
    let observation_ms =
      frame.metrics.observation_time.unwrap_or_default().as_secs_f64() * 1000.0;
    outcome["observation_ms"] = observation_ms.into();
    outcome["page"] = frame.metrics.page.map(|p| p.0 + 1).into();
    outcome["pages"] = frame.metrics.pages.into();
    outcome["hits"] = frame.metrics.hits.into();
    outcome["animating"] = frame.metrics.animating.into();
    let result = output.paint(&Paint {
      outcome,
      started,
      built: frame.metrics.built,
      build: frame.metrics.build,
      busy: frame.metrics.busy,
      fresh,
      upload_bytes,
      stand_ins: renderer.stand_ins,
      missing: renderer.missing,
      missing_pixels: renderer.missing_pixels,
      stand_in_pixels: renderer.stand_in_pixels,
      complete: frame.complete,
    });
    self.written(result);
  }

  fn written(&mut self, result: vpdf::Result<()>) {
    if let Err(error) = result {
      eprintln!("Trace failed: {error}");
      self.output = None;
    }
  }
}
