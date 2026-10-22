//! Full-session readiness; crop retains component measurements.
use std::path::PathBuf;
use vpdf::Result;

#[path = "latency/crop.rs"]
mod crop;
#[path = "support/hash.rs"]
mod hash;
#[path = "drive/latency.rs"]
mod latency;
#[path = "drive/runner.rs"]
mod runner;
#[path = "../tests/support/screen.rs"]
mod screen;
#[path = "support/timing.rs"]
mod stats;

fn main() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let files: Vec<PathBuf> = match std::env::var_os("VPDF_EXTRA") {
    Some(extra) => std::env::split_paths(&extra).collect(),
    None => vec![root.join("../huge/sdm.pdf")],
  };
  let columns = std::env::var("VPDF_COLUMNS")
    .ok()
    .and_then(|value| value.parse().ok())
    .unwrap_or(179);
  let screen = screen::pixels(columns, 103, 11, 21);
  let output = if std::env::var_os("VPDF_CROP_BENCH").is_some() {
    crop::run(&files)?
  } else {
    latency::run(&files, screen)?
  };
  if let Ok(path) = std::env::var("VPDF_BENCH_OUTPUT") {
    std::fs::write(path, &output)?;
  }
  println!("{output}");
  Ok(())
}
