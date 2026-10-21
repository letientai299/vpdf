use serde::Serialize;
#[path = "percentile.rs"]
mod percentile;

#[derive(Default, Serialize)]
pub struct Timing {
  pub samples: usize,
  pub p50_ms: f64,
  pub p95_ms: Option<f64>,
  pub p99_ms: Option<f64>,
  pub max_ms: f64,
}

pub fn timing(mut times: Vec<f64>) -> Timing {
  if times.is_empty() {
    return Timing::default();
  }
  times.sort_by(f64::total_cmp);
  let count = times.len();
  Timing {
    samples: count,
    p50_ms: times[count / 2],
    p95_ms: percentile::tail(&times, 95),
    p99_ms: percentile::tail(&times, 99),
    max_ms: times[count - 1],
  }
}
