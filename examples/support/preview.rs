use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use vpdf::app::App;
use vpdf_model::document::Target;
use vpdf_model::geometry::Screen;
use vpdf_model::text::{PageId, Text};
use vpdf_model::tile::TileKey;
use vpdf_pdf::document::Document;
use vpdf_reader::reader::TextSource;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::preview::Preview;
use vpdf_ui::source::Raster;

struct Capture<'a> {
  doc: &'a mut Document,
  rasters: Vec<Value>,
}

impl TextSource for Capture<'_> {
  fn page_count(&self) -> usize {
    self.doc.pages.len()
  }
  fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
    self.doc.text(page)
  }
}

impl Raster for Capture<'_> {
  fn raster(
    &mut self,
    key: TileKey,
    visit: impl FnOnce(&[u8], usize) -> Result<()>,
  ) -> Result<()> {
    vpdf_pdf::raster::visit_area(self.doc, key, |pixels, stride| {
      self.rasters.push(json!({
        "page":key.page, "scale":key.scale, "rotation":key.rotation,
        "rect":[key.x,key.y,key.width,key.height], "crop":key.crop,
        "pixels":pixels, "stride":stride,
      }));
      visit(pixels, stride)
    })
  }
}

pub fn prepare(path: &Path, screen: Screen) -> Result<Vec<u8>> {
  let mut cases = Vec::new();
  for text in [false, true] {
    let mut app = App::with_screen(path, "", text, screen)?;
    let link = app
      .doc
      .links(PageId(1))?
      .iter()
      .find(|link| !matches!(link.target, Target::Uri(_)))
      .cloned()
      .context("Missing preview citation")?;
    let (x, y) = link.first().centre();
    let position = app.doc.text(PageId(1))?.nearest(PageId(1), x, y);
    app.go_to(position, true)?;
    app.preview_link()?;
    let target =
      app.overlays.preview().context("Missing preview")?.target().clone();
    let cursor = app.position_rect(position)?;
    let mut output_sha256 = Vec::new();
    let mut graphics = Graphics::default();
    for _ in 0..2 {
      let mut out = Vec::new();
      graphics.begin(&mut out)?;
      vpdf::presentation::try_draw(&mut out, Some(&mut graphics), |s| {
        app.scene_preview(s)
      })?;
      graphics.end(&mut out)?;
      output_sha256.push(super::hash::digest(&out));
    }
    let mut capture = Capture { doc: &mut app.doc, rasters: Vec::new() };
    let mut preview = Preview::open(
      target.clone(),
      position,
      true,
      text,
      &app.layout,
      &mut capture,
    )?;
    anyhow::ensure!(
      preview.scroll(0.0, &app.layout, &mut capture)?,
      "Invalid preview fixture"
    );
    cases.push(json!({
      "text":text, "target":target, "source":position,
      "cursor":cursor.map(super::text::coordinates),
      "screen":
        [screen.columns, screen.rows, screen.cell_width, screen.cell_height],
      "scroll":app.reader.scroll, "rasters":capture.rasters,
      "output_sha256":output_sha256,
    }));
  }
  Ok(serde_json::to_vec(&cases)?)
}
