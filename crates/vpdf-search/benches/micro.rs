#[path = "../../../benches/support/directory.rs"]
mod directory;
use anyhow::Result;
use std::hint::black_box;
use vpdf_model::text::PageId;
use vpdf_search::search::{Index, compile};
#[path = "../../../benches/support/fixtures.rs"]
mod fixtures;
#[path = "../../../benches/support/hash.rs"]
mod hash;
#[path = "../../../benches/support/mod.rs"]
mod support;
use support::measure;

fn index() -> Result<()> {
  let prepared = fixtures::text()?;
  let page = PageId(1);
  let text = &prepared.text[page.0];
  measure("Index::new paper p2", || {
    black_box(Index::new(page, black_box(text)));
  })?;
  let index = Index::new(page, text);
  let query = compile("the", false)?;
  let hits = index.matches(&query, 1000)?;
  anyhow::ensure!(!hits.is_empty(), "Missing query matches");
  measure("Index::matches paper p2", || {
    black_box(index.matches(black_box(&query), 1000).expect("paper query"));
  })?;
  let mut bytes = Vec::new();
  index.encode(&mut bytes);
  let decoded = Index::decode(page, &bytes)?;
  let restored = decoded.matches(&query, 1000)?;
  anyhow::ensure!(restored.len() == hits.len(), "Decoded hit count differs");
  for (before, after) in hits.iter().zip(&restored) {
    anyhow::ensure!(
      (before.first, before.last, &before.excerpt)
        == (after.first, after.last, &after.excerpt),
      "Decoded hits differ"
    );
  }
  let mut encoded = Vec::with_capacity(bytes.len());
  decoded.encode(&mut encoded);
  anyhow::ensure!(encoded == bytes, "Encoded index differs");
  println!("index bytes {} sha256 {}", bytes.len(), hash::digest(&bytes));
  measure("Index::encode paper p2", || {
    encoded.clear();
    index.encode(&mut encoded);
    black_box(&encoded);
  })?;
  measure("Index::decode paper p2", || {
    black_box(Index::decode(page, black_box(&bytes)).expect("paper index"));
  })
}

fn main() -> Result<()> {
  support::run(&[("index", index)])
}
