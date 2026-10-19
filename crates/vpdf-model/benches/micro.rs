use anyhow::{Context, Result};
use std::hint::black_box;
#[path = "../../../benches/support/fixtures.rs"]
mod fixtures;
#[path = "../../../benches/support/hash.rs"]
mod hash;
#[path = "../../../benches/support/mod.rs"]
mod support;

#[derive(serde::Deserialize)]
struct Region {
  page: usize,
  bounds: [f32; 4],
  text_sha256: String,
}

fn region() -> Result<()> {
  let prepared: Region = fixtures::read("region")?;
  let pages = fixtures::text()?;
  let text = pages.text.get(prepared.page).context("Missing region page")?;
  let bounds = fixtures::bounds(prepared.bounds);
  anyhow::ensure!(
    hash::digest(text.in_region(bounds).as_bytes()) == prepared.text_sha256,
    "Region fixture text differs"
  );
  support::measure("Text::in_region", || {
    black_box(black_box(text).in_region(bounds));
  })
}

fn main() -> Result<()> {
  if support::selected("region text") {
    region()?;
  }
  Ok(())
}
