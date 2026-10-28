use std::path::PathBuf;

pub fn fixtures() -> PathBuf {
  std::env::var_os("VPDF_BENCH_FIXTURES").map_or_else(
    || {
      PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/bench-fixtures")
    },
    PathBuf::from,
  )
}
