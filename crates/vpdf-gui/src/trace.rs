//! `VPDF_TRACE` frame log: one JSON object per painted frame.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

pub struct Trace {
  output: Option<BufWriter<File>>,
  started: Instant,
}

/// What one paint did.
pub struct Paint {
  pub started: Instant,
  /// A new session frame arrived.
  pub fresh: bool,
  pub upload_bytes: usize,
  /// Visible tiles drawn from another scale.
  pub stand_ins: usize,
  /// Visible tiles drawn blank.
  pub missing: usize,
}

impl Trace {
  pub fn from_env() -> Self {
    let output = std::env::var_os("VPDF_TRACE")
      .filter(|path| !path.is_empty())
      .and_then(|path| match File::create(&path) {
        Ok(file) => Some(BufWriter::new(file)),
        Err(error) => {
          eprintln!("Trace unavailable: {error}");
          None
        }
      });
    Self { output, started: Instant::now() }
  }

  pub fn paint(&mut self, paint: &Paint) {
    let Some(output) = &mut self.output else { return };
    let now = Instant::now();
    let ms = |from: Instant, to: Instant| {
      to.saturating_duration_since(from).as_secs_f64() * 1000.0
    };
    let written = writeln!(
      output,
      concat!(
        r#"{{"elapsed_ms":{:.3},"paint_ms":{:.3},"fresh":{},"#,
        r#""upload_bytes":{},"stand_ins":{},"missing":{}}}"#,
      ),
      ms(self.started, paint.started),
      ms(paint.started, now),
      paint.fresh,
      paint.upload_bytes,
      paint.stand_ins,
      paint.missing,
    )
    .and_then(|()| output.flush());
    if let Err(error) = written {
      eprintln!("Trace failed: {error}");
      self.output = None;
    }
  }
}
