use anyhow::{Context, Result};
#[path = "../benches/support/hash.rs"]
mod hash;
use std::path::PathBuf;
use vpdf_model::geometry::Screen;
use vpdf_pdf::document::Document;
use vpdf_ui::layout::Layout;

fn main() -> Result<()> {
  let mut args = std::env::args_os().skip(1);
  let source = PathBuf::from(args.next().context("Missing source PDF")?);
  let destination =
    PathBuf::from(args.next().context("Missing fixture directory")?);
  let mut doc = Document::open(&source, "")?;
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let layout = Layout::new(doc.pages.iter().map(|page| page.bounds), screen);
  let key = vpdf_ui::layout::visible(&layout, 0.0)[0];
  let pixels = vpdf_pdf::raster::raster(&mut doc, key)?;
  let coordinates = |b: vpdf_model::text::Bounds| [b.x0, b.y0, b.x1, b.y1];
  let geometry = serde_json::to_vec(&serde_json::json!({
    "pages": doc.pages.iter().map(|p| coordinates(p.bounds)).collect::<Vec<_>>(),
    "glyph": coordinates(doc.text(vpdf_model::text::PageId(0))?.lines[0].glyphs[0].bounds),
  }))?;
  let manifest = serde_json::json!({
    "source": source.canonicalize()?,
    "source_sha256": hash::digest(&std::fs::read(&source)?),
    "revision": env!("VPDF_VERSION"),
    "geometry_sha256": hash::digest(&geometry),
    "rgb_sha256": hash::digest(&pixels),
    "width": key.width, "height": key.height,
    "page": key.page.0, "scale": key.scale, "rotation": key.rotation,
    "x": key.x, "y": key.y, "crop": key.crop.0,
  });
  std::fs::create_dir_all(&destination)?;
  std::fs::write(destination.join("tile.rgb"), pixels)?;
  std::fs::write(destination.join("geometry.json"), geometry)?;
  std::fs::write(
    destination.join("manifest.json"),
    serde_json::to_vec_pretty(&manifest)?,
  )?;
  println!("Prepared {}", destination.display());
  Ok(())
}
