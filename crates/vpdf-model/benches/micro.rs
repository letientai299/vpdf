#[path = "../../vpdf-dev/support/directory.rs"]
mod directory;
use anyhow::{Context, Result};
use std::hint::black_box;
#[path = "../../vpdf-dev/support/fixtures.rs"]
mod fixtures;
#[path = "../../vpdf-dev/support/hash.rs"]
mod hash;
#[path = "../../vpdf-dev/support/mod.rs"]
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
  support::run(&[("region text", region), ("geometry", geometry)])
}

fn geometry() -> Result<()> {
  use vpdf_model::geometry::{page_matrix, transform};
  let prepared = fixtures::text()?;
  support::measure("page_matrix and transform", || {
    for (_, bounds) in &prepared.metadata {
      for angle in [0, 90, 180, 270] {
        let matrix = page_matrix(black_box(*bounds), 2048, black_box(angle));
        black_box(transform(*bounds, &matrix));
      }
    }
  })
}
