#[path = "../../crates/vpdf-dev/support/percentile.rs"]
mod stats;

#[test]
fn ranks_include_the_extreme_tail() {
  let values: Vec<_> = (0..1001).map(f64::from).collect();
  assert_eq!(stats::percentile(&values, 50), Some(500.0));
  assert_eq!(stats::tail(&values, 95), Some(950.0));
  assert_eq!(stats::tail(&values, 99), Some(990.0));
  assert_eq!(stats::percentile(&values, 100), Some(1000.0));
  assert_eq!(stats::percentile(&[], 99), None);
  assert_eq!(stats::percentile(&values, 101), None);
}

#[test]
fn sparse_tails_are_unavailable() {
  let values = vec![1.0; 1000];
  assert_eq!(stats::tail(&values[..199], 95), None);
  assert_eq!(stats::tail(&values[..200], 95), Some(1.0));
  assert_eq!(stats::tail(&values[..999], 99), None);
  assert_eq!(stats::tail(&values, 99), Some(1.0));
  assert_eq!(stats::tail(&values, 100), None);
}
