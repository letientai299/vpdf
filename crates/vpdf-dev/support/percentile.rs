/// Nearest-rank percentile of sorted samples.
pub fn percentile(sorted: &[f64], share: usize) -> Option<f64> {
  if sorted.is_empty() || share > 100 {
    return None;
  }
  let index = sorted.len().checked_mul(share)?.div_ceil(100).saturating_sub(1);
  sorted.get(index).copied()
}

/// Requires ten upper-tail sample positions.
pub fn tail(sorted: &[f64], share: usize) -> Option<f64> {
  let upper = 100_usize.checked_sub(share)?;
  if upper == 0 || sorted.len() < 1000_usize.div_ceil(upper) {
    return None;
  }
  percentile(sorted, share)
}
