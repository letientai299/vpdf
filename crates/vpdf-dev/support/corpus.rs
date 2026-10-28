use anyhow::Result;
use std::path::PathBuf;

pub fn corpus(name: &str) -> Result<PathBuf> {
  let root = std::env::var_os("VPDF_CORPUS").ok_or_else(|| {
    anyhow::anyhow!("VPDF_CORPUS unset; run mise bench:micro")
  })?;
  let path = PathBuf::from(root).join(name);
  anyhow::ensure!(path.is_file(), "{name} missing; run mise corpus");
  Ok(path)
}
