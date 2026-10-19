use anyhow::{Context, Result};
#[path = "../../../benches/support/hash.rs"]
mod hash;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use vpdf_model::geometry::{Crop, Screen};
use vpdf_model::text::PageId;
use vpdf_model::tile::TileKey;
use vpdf_terminal::graphics::{Graphics, Presentation};
use vpdf_terminal::payload::{Encoder, Tile};
use vpdf_terminal::spool::Spool;

#[path = "../../../benches/support/mod.rs"]
mod support;
use support::measure;

/// A status line row as `App::status` cleans it each frame.
fn clean() -> Result<()> {
  let status =
    " NORMAL  1:paper.pdf  2:book.pdf  3.2 Results  12/40 [xii] 31% ";
  measure("terminal::clean", || {
    black_box(vpdf_terminal::terminal::clean(black_box(status), 80));
  })
}

/// A hostile title: wide emoji, then zero-width marks.
fn clean_marks() -> Result<()> {
  let title = "❤\u{FE0F}".repeat(100) + &"\u{301}".repeat(50_000);
  measure("terminal::clean marks", || {
    black_box(vpdf_terminal::terminal::clean(black_box(&title), 160));
  })
}

#[derive(serde::Deserialize)]
struct Manifest {
  width: i32,
  height: i32,
  rgb_sha256: String,
  source_sha256: String,
  revision: String,
}

fn fixture() -> Result<(Manifest, Vec<u8>)> {
  let directory = std::env::var_os("VPDF_BENCH_FIXTURES").map_or_else(
    || {
      PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/bench-fixtures")
    },
    PathBuf::from,
  );
  let metadata = std::fs::read(directory.join("manifest.json"))
    .context("Run mise run bench:fixtures first")?;
  let manifest: Manifest = serde_json::from_slice(&metadata)?;
  let pixels = std::fs::read(directory.join("tile.rgb"))?;
  anyhow::ensure!(
    manifest.width > 0
      && manifest.height > 0
      && pixels.len() as i64
        == i64::from(manifest.width) * i64::from(manifest.height) * 3,
    "Invalid raster fixture dimensions"
  );
  anyhow::ensure!(
    hash::digest(&pixels) == manifest.rgb_sha256,
    "Raster fixture hash mismatch"
  );
  println!(
    "fixture {} source {} revision {}",
    manifest.rgb_sha256, manifest.source_sha256, manifest.revision
  );
  Ok((manifest, pixels))
}

fn tile_encode() -> Result<()> {
  let (_, pixels) = fixture()?;
  let encoder = Encoder::default();
  let bytes = vpdf_terminal::payload::encode(&pixels)?.len();
  writeln!(io::stdout(), "tiles::encode bytes {bytes}")?;
  measure(&format!("tiles::encode {} KiB", pixels.len() / 1024), || {
    drop(black_box(encoder.encode(black_box(&pixels)).expect("encode")));
  })?;
  Ok(())
}

fn spool() -> Result<()> {
  let (_, pixels) = fixture()?;
  let directory = tempfile::tempdir()?;
  let spool = Arc::new(Spool::create(directory.path())?);
  let encoder = Encoder::new(Some(spool));
  measure("spool tile", || {
    drop(black_box(encoder.encode(black_box(&pixels)).expect("spool")));
  })?;
  Ok(())
}

fn resident() -> Result<(Graphics, TileKey, Screen)> {
  let (manifest, pixels) = fixture()?;
  let encoder = Encoder::default();
  let mut graphics = Graphics::new(Presentation::default(), &encoder);
  let key = TileKey {
    page: PageId(0),
    scale: 1024,
    rotation: 0,
    x: 0,
    y: 0,
    width: manifest.width,
    height: manifest.height,
    crop: Crop::default(),
  };
  let tile = Tile { key, payload: encoder.encode(&pixels)?, render_ms: 0.0 };
  graphics.upload(0, &tile, &mut Vec::new())?;
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  Ok((graphics, key, screen))
}

fn placement() -> Result<()> {
  let (mut graphics, key, screen) = resident()?;
  let mut output = Vec::new();
  measure("resident tile placement", || {
    output.clear();
    graphics.begin(&mut output).expect("begin");
    assert!(
      graphics
        .place_at((0, key), (0.0, 0.0), screen, 0, &mut output)
        .expect("place")
    );
    graphics.end(&mut output).expect("end");
    black_box(&output);
  })?;
  println!("placement bytes {}", output.len());
  Ok(())
}

fn residency() -> Result<()> {
  let (mut graphics, key, _) = resident()?;
  measure("resident tile lookup", || {
    black_box(graphics.has(black_box(&(0, key))));
  })?;
  Ok(())
}

fn main() -> Result<()> {
  type Case = fn() -> Result<()>;
  let cases: &[(&str, Case)] = &[
    ("clean", clean),
    ("clean marks", clean_marks),
    ("tile encode", tile_encode),
    ("spool", spool),
    ("placement", placement),
    ("residency", residency),
  ];
  for (name, call) in cases {
    if support::selected(name) {
      call()?;
    }
  }
  Ok(())
}
