use anyhow::Result;
use std::io::{self, Write};
use std::time::{Duration, Instant};

const SAMPLES: usize = 31;

/// Median, p10, and p90 nanoseconds per call.
pub fn measure(name: &str, mut call: impl FnMut()) -> Result<()> {
  // Batches of about a millisecond hide the clock's resolution.
  let mut batch = 1_u32;
  loop {
    let started = Instant::now();
    for _ in 0..batch {
      call();
    }
    if started.elapsed() >= Duration::from_millis(1) {
      break;
    }
    batch *= 2;
  }
  let mut samples: Vec<f64> = (0..SAMPLES)
    .map(|_| {
      let started = Instant::now();
      for _ in 0..batch {
        call();
      }
      started.elapsed().as_nanos() as f64 / f64::from(batch)
    })
    .collect();
  samples.sort_by(f64::total_cmp);
  let at = |share: usize| samples[(SAMPLES - 1) * share / 100];
  writeln!(
    io::stdout(),
    "{name:<28} {:>12.1} ns  p10 {:.1}  p90 {:.1}",
    at(50),
    at(10),
    at(90)
  )?;
  Ok(())
}

fn selected(name: &str) -> bool {
  let filters: Vec<_> =
    std::env::args().skip(1).filter(|arg| !arg.starts_with("--")).collect();
  filters.is_empty()
    || filters.iter().any(|filter| name.contains(filter.as_str()))
}

pub type Case = fn() -> Result<()>;

pub fn run(cases: &[(&str, Case)]) -> Result<()> {
  let mut failed = 0;
  for (name, case) in cases {
    if selected(name)
      && let Err(error) = case()
    {
      writeln!(io::stderr(), "{name}: {error:#}")?;
      failed += 1;
    }
  }
  anyhow::ensure!(failed == 0, "{failed} cases failed");
  Ok(())
}
