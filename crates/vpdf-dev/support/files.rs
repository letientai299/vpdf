#[path = "corpus.rs"]
mod root;
use std::path::PathBuf;

/// `VPDF_EXTRA` paths, else `names` in the corpus.
pub fn files(names: &[&str]) -> anyhow::Result<Vec<PathBuf>> {
  if let Some(extra) = std::env::var_os("VPDF_EXTRA") {
    return Ok(std::env::split_paths(&extra).collect());
  }
  names.iter().map(|name| root::corpus(name)).collect()
}

/// Eight page indexes spread over `pages` pages.
pub fn spread(pages: usize) -> impl Iterator<Item = usize> {
  (0..8).map(move |i| (i * 7919 + pages / 3) % pages)
}
