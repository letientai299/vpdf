use anyhow::{Context, Result};
#[path = "../../../support/hash.rs"]
mod hash;
#[path = "support/home.rs"]
mod home;
#[path = "support/menu.rs"]
mod menu;
#[path = "support/overlays.rs"]
mod overlays;
#[path = "support/preview.rs"]
mod preview;
#[path = "../../../../../tests/support/reader.rs"]
mod reader_fixture;
#[path = "support/region.rs"]
mod region;
#[path = "../../../../../tests/support/screen.rs"]
mod screen;
#[path = "support/scrollbar.rs"]
mod scrollbar;
#[path = "support/text.rs"]
mod text;
use std::path::PathBuf;
use vpdf_pdf::document::Document;
use vpdf_ui::layout::Layout;

fn main() -> Result<()> {
  let mut args = std::env::args_os().skip(1);
  let source = PathBuf::from(args.next().context("Missing source PDF")?);
  let destination =
    PathBuf::from(args.next().context("Missing fixture directory")?);
  let mut doc = Document::open(&source, "")?;
  let screen = screen::cells(120, 40);
  let layout = Layout::new(doc.pages.iter().map(|page| page.bounds), screen);
  let key = vpdf_ui::layout::visible(&layout, 0.0)[0];
  let pixels = vpdf_pdf::raster::raster(&mut doc, key)?;
  let coordinates = text::coordinates;
  let pages: Vec<_> = doc.pages.iter().map(|p| coordinates(p.bounds)).collect();
  let glyph = doc.text(vpdf_model::text::PageId(0))?.lines[0].glyphs[0].bounds;
  let geometry = serde_json::to_vec(&serde_json::json!({
    "pages": pages,
    "glyph": coordinates(glyph),
  }))?;
  let text = text::prepare(&mut doc)?;
  anyhow::ensure!(doc.pages.len() >= 2, "Citation fixture needs two pages");
  let page = vpdf_model::text::PageId(1);
  let scroll = layout.pages()[page.0].top;
  let mut found = Vec::new();
  for link in doc.links(page)?.iter() {
    let spot = vpdf_ui::hints::spot(link, |bounds| {
      vpdf_ui::hints::page_cell(&layout, scroll, 0, page, bounds)
    })?;
    if let Some(spot) = spot {
      found.push((spot, link.target.clone()));
    }
  }
  anyhow::ensure!(!found.is_empty(), "No visible citation links");
  let hints = serde_json::to_vec(&serde_json::json!({
    "screen":
      [screen.columns, screen.rows, screen.cell_width, screen.cell_height],
    "found": found,
    "page": page,
    "scroll": scroll,
  }))?;
  let home = home::prepare(&source, screen)?;
  let preview = preview::prepare(&source, screen)?;
  let context = menu::prepare(&source, screen)?;
  let region = region::prepare(&source, screen)?;
  let book_path = source.with_file_name("book.pdf");
  let book = text::prepare(&mut Document::open(&book_path, "")?)?;
  let overlays = overlays::prepare(&source, &book_path, screen)?;
  let scrollbar = scrollbar::prepare(&mut doc)?;
  let manifest = serde_json::json!({
    "source": source.canonicalize()?,
    "source_sha256": hash::digest(&std::fs::read(&source)?),
    "revision": vpdf::version::VERSION,
    "geometry_sha256": hash::digest(&geometry),
    "book_source": book_path.canonicalize()?,
    "book_source_sha256": hash::digest(&std::fs::read(&book_path)?),
    "book_sha256": hash::digest(&book),
    "overlays_sha256": hash::digest(&overlays),
    "scrollbar_sha256": hash::digest(&scrollbar),
    "text_sha256": hash::digest(&text),
    "hints_sha256": hash::digest(&hints),
    "region_sha256": hash::digest(&region),
    "context_sha256": hash::digest(&context),
    "home_sha256": hash::digest(&home),
    "preview_sha256": hash::digest(&preview),
    "rgb_sha256": hash::digest(&pixels),
    "width": key.width, "height": key.height,
    "page": key.page.0, "scale": key.scale, "rotation": key.rotation,
    "x": key.x, "y": key.y, "crop": key.crop.0,
  });
  std::fs::create_dir_all(&destination)?;
  std::fs::write(destination.join("tile.rgb"), pixels)?;
  std::fs::write(destination.join("geometry.json"), geometry)?;
  std::fs::write(destination.join("text.json"), text)?;
  std::fs::write(destination.join("book.json"), book)?;
  std::fs::write(destination.join("overlays.json"), overlays)?;
  std::fs::write(destination.join("scrollbar.json"), scrollbar)?;
  std::fs::write(destination.join("hints.json"), hints)?;
  std::fs::write(destination.join("context.json"), context)?;
  std::fs::write(destination.join("home.json"), home)?;
  std::fs::write(destination.join("preview.json"), preview)?;
  std::fs::write(destination.join("region.json"), region)?;
  std::fs::write(
    destination.join("manifest.json"),
    serde_json::to_vec_pretty(&manifest)?,
  )?;
  println!("Prepared {}", destination.display());
  Ok(())
}
