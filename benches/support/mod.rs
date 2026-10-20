use anyhow::Result;
use std::io::{self, Write};
use std::time::{Duration, Instant};

pub mod percentile;

const BATCHES: usize = 31;
const SAMPLES: usize = 1001;
const SAMPLE_LIMIT: Duration = Duration::from_secs(10);

/// Individual latency and calibrated batch throughput.
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
  let mut samples: Vec<f64> = (0..BATCHES)
    .map(|_| {
      let started = Instant::now();
      for _ in 0..batch {
        call();
      }
      started.elapsed().as_nanos() as f64 / f64::from(batch)
    })
    .collect();
  samples.sort_by(f64::total_cmp);
  let throughput = samples[BATCHES / 2];
  let samples = calls(&mut call);
  report(name, &samples, Some((throughput, batch)))?;
  Ok(())
}

fn calls(mut call: impl FnMut()) -> Vec<f64> {
  let deadline = Instant::now() + SAMPLE_LIMIT;
  let mut samples = Vec::with_capacity(SAMPLES);
  for _ in 0..SAMPLES {
    let started = Instant::now();
    call();
    samples.push(started.elapsed().as_nanos() as f64);
    if Instant::now() >= deadline {
      break;
    }
  }
  samples.sort_by(f64::total_cmp);
  samples
}

fn report(
  name: &str,
  samples: &[f64],
  throughput: Option<(f64, u32)>,
) -> Result<()> {
  let display = |share| {
    percentile::tail(samples, share)
      .map_or_else(|| "n/a".to_owned(), |value| format!("{value:.1}"))
  };
  writeln!(
    io::stdout(),
    "{name:<28} p50 {:.1} ns  p95 {} ns  p99 {} ns  n={}",
    percentile::percentile(samples, 50).unwrap_or(0.0),
    display(95),
    display(99),
    samples.len()
  )?;
  if let Some((value, batch)) = throughput {
    writeln!(
      io::stdout(),
      "  batch median {value:.1} ns/op ({BATCHES} x {batch} calls)"
    )?;
  }
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
  writeln!(
    io::stdout(),
    "Call latency includes timer cost; batches measure throughput."
  )?;
  let floor = calls(|| {});
  report("timer floor", &floor, None)?;
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
