use std::path::PathBuf;

pub fn fixtures() -> PathBuf {
  std::env::var_os("VPDF_BENCH_FIXTURES").map_or_else(
    || {
      PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        if env!("CARGO_PKG_NAME") == "vpdf" {
          "target/bench-fixtures"
        } else {
          "../../target/bench-fixtures"
        },
      )
    },
    PathBuf::from,
  )
}
