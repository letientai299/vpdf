//! Single hot functions, timed in isolation, so a change is measured
//! without the noise of whole-reader runs. Arguments filter cases by name.
use std::hint::black_box;
use std::io::{self, Write};
use std::time::{Duration, Instant};
use vpdf::Result;

const SAMPLES: usize = 31;

/// Median, p10, and p90 nanoseconds per call.
fn measure(name: &str, mut call: impl FnMut()) -> Result<()> {
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

/// A status line row as `App::status` cleans it each frame.
fn clean() -> Result<()> {
  let status =
    " NORMAL  1:paper.pdf  2:book.pdf  3.2 Results  12/40 [xii] 31% ";
  measure("terminal::clean", || {
    black_box(vpdf::terminal::clean(black_box(status), 80));
  })
}

/// A hostile title: wide emoji, then zero-width marks.
fn clean_marks() -> Result<()> {
  let title = "❤\u{FE0F}".repeat(100) + &"\u{301}".repeat(50_000);
  measure("terminal::clean marks", || {
    black_box(vpdf::terminal::clean(black_box(&title), 160));
  })
}

type Case = fn() -> Result<()>;

const CASES: &[(&str, Case)] =
  &[("clean", clean), ("clean marks", clean_marks)];

fn main() -> Result<()> {
  // Cargo passes `--bench`; other arguments name cases.
  let filters: Vec<String> =
    std::env::args().skip(1).filter(|arg| !arg.starts_with("--")).collect();
  for (name, case) in CASES {
    if filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())) {
      case()?;
    }
  }
  Ok(())
}
