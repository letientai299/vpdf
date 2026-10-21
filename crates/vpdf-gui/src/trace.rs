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
    let result = output.paint(&Paint {
      started,
      built: frame.metrics.built,
      build: frame.metrics.build,
      busy: frame.metrics.busy,
      fresh,
      upload_bytes,
      stand_ins: renderer.stand_ins,
      missing: renderer.missing,
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
