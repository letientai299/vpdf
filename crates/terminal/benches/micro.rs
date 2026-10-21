#[path = "../../../benches/support/directory.rs"]
mod directory;
use anyhow::Result;
#[path = "../../../benches/support/hash.rs"]
mod hash;
#[path = "../../../benches/support/raster.rs"]
mod raster;
use raster::fixture;
use std::hint::black_box;
use std::io::{self, Write};
use std::sync::Arc;
use vpdf_model::geometry::Screen;
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

fn inline() -> Result<()> {
  let (manifest, pixels) = fixture()?;
  let bounds = vpdf_model::text::Bounds {
    x0: 0.0,
    y0: 0.0,
    x1: manifest.width as f32,
    y1: manifest.height as f32,
  };
  let fit = vpdf_terminal::inline::Fit::new(bounds, (40, 12), 2.0)?;
  for tmux in [false, true] {
    let mut out = Vec::new();
    let transport = vpdf_terminal::terminal::Transport { tmux };
    measure(&format!("inline::print tmux {tmux}"), || {
      out.clear();
      fit
        .print(
          transport,
          123,
          (manifest.width as u32, manifest.height as u32),
          &pixels,
          &mut out,
        )
        .expect("inline output");
      black_box(&out);
    })?;
    println!("inline output {}", hash::digest(&out));
  }
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
  let key = manifest.key();
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
  support::run(&[
    ("clean", clean),
    ("clean marks", clean_marks),
    ("tile encode", tile_encode),
    ("inline", inline),
    ("spool", spool),
    ("placement", placement),
    ("residency", residency),
  ])
}
