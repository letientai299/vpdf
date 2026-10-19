//! Cold per-page stage costs: page load, display list, text, raster, and
//! encoding, so changes to the render path can be attributed.
use mupdf::IRect;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use vpdf::Result;
use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH};
use vpdf_pdf::document::{Document, TEXT_FLAGS, extract};
use vpdf_pdf::raster::draw;
use vpdf_terminal::payload::encode;
use vpdf_ui::layout::{Crop, page_matrix};

#[derive(Default, Serialize)]
struct Stages {
  load: f64,
  list: f64,
  text_page: f64,
  text_list: f64,
  raster_tiles: f64,
  encode: f64,
  tiles: usize,
  encoded_kib: f64,
}

fn ms(started: Instant) -> f64 {
  started.elapsed().as_secs_f64() * 1000.0
}

/// Text extraction and rendering each start from a fresh document, as
/// the reader's foreground and render worker do.
fn page(path: &Path, index: usize, width: f32) -> Result<Stages> {
  let mut stages = Stages::default();
  let doc = Document::open(path, "")?;
  let started = Instant::now();
  let page = doc.native.load_page(index as i32)?;
  stages.load = ms(started);
  let started = Instant::now();
  extract(&page)?;
  stages.text_page = ms(started);
  let doc = Document::open(path, "")?;
  let bounds = doc.pages[index].bounds;
  let scale = ((width / bounds.width()) * 1024.0).round() as u32;
  let matrix = page_matrix(Crop::default().apply(bounds), scale, 0);
  let started = Instant::now();
  let list = doc.native.load_page(index as i32)?.to_display_list(true)?;
  stages.list = ms(started);
  let (w, h) = (
    (bounds.width() * scale as f32 / 1024.0) as i32,
    ((bounds.height() * scale as f32 / 1024.0) as i32).min(2142),
  );
  let mut tiles = Vec::new();
  let started = Instant::now();
  for y in (0..h).step_by(TILE_HEIGHT as usize) {
    for x in (0..w).step_by(TILE_WIDTH as usize) {
      let area =
        IRect::new(x, y, (x + TILE_WIDTH).min(w), (y + TILE_HEIGHT).min(h));
      tiles.push(draw(&list, &matrix, area)?);
    }
  }
  stages.raster_tiles = ms(started);
  stages.tiles = tiles.len();
  let started = Instant::now();
  for tile in &tiles {
    stages.encoded_kib += encode(tile.samples())?.len() as f64 / 1024.0;
  }
  stages.encode = ms(started);
  let started = Instant::now();
  drop(list.to_text_page(TEXT_FLAGS)?);
  stages.text_list = ms(started);
  Ok(stages)
}

fn main() -> Result<()> {
  let files = std::env::var_os("VPDF_EXTRA").map_or_else(
    || {
      let root =
        PathBuf::from(std::env::var("VPDF_CORPUS").unwrap_or_default());
      ["book", "paper", "form", "ocr"]
        .map(|name| root.join(format!("{name}.pdf")))
        .to_vec()
    },
    |extra| std::env::split_paths(&extra).collect(),
  );
  let mut report = Vec::new();
  for file in files {
    let pages = Document::open(&file, "")?.pages.len();
    let picks: Vec<usize> =
      (0..8).map(|i| (i * 7919 + pages / 3) % pages).collect();
    for index in picks {
      let stages = page(&file, index, 1969.0)?;
      let name = file.file_name().unwrap_or_default().to_string_lossy();
      report.push(serde_json::json!({
        "document": name, "page": index + 1, "stages": stages,
      }));
    }
  }
  println!("{}", serde_json::to_string(&report)?);
  Ok(())
}
