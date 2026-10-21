//! Native paint rows in the shared trace.

use std::time::Instant;
use vpdf::trace::Log;
pub use vpdf::trace::Paint;

pub struct Trace {
  output: Option<Log>,
  step: u64,
}

impl Trace {
  pub fn from_env() -> Self {
    let output = std::env::var_os("VPDF_TRACE")
      .filter(|path| !path.is_empty())
      .and_then(|path| match Log::open(path.as_ref()) {
        Ok(log) => Some(log),
        Err(error) => {
          eprintln!("Trace unavailable: {error}");
          None
        }
      });
    Self { output, step: 0 }
  }

  pub fn input(&mut self) {
    self.step += 1;
    if let Some(output) = &self.output {
      let result = output.input(Instant::now(), self.step);
      self.written(result);
    }
  }

  pub fn paint(&mut self, paint: &Paint) {
    let Some(output) = &self.output else { return };
    let result = output.paint(paint);
    self.written(result);
  }

  fn written(&mut self, result: vpdf::Result<()>) {
    if let Err(error) = result {
      eprintln!("Trace failed: {error}");
      self.output = None;
    }
  }
}
