use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;

/// Waits for the outline the app loads off the input thread.
pub fn outline_loaded(app: &mut App) -> Result<()> {
  let started = Instant::now();
  while app.outline_job.is_some() {
    app.outline_updates()?;
    anyhow::ensure!(started.elapsed() < Duration::from_secs(10), "Outline");
    std::thread::sleep(Duration::from_millis(5));
  }
  Ok(())
}
