use anyhow::{Context, Result};
use std::path::Path;
use vpdf_model::geometry::Screen;
use vpdf_model::text::PageId;

pub fn prepare(source: &Path, screen: Screen) -> Result<Vec<u8>> {
  let mut app = vpdf::solo::Solo::open(source, "", false, screen)?;
  let page = PageId(2);
  let figures = app.doc.figures(page)?;
  let figure = figures
    .iter()
    .find(|figure| figure.caption.starts_with("Figure 1"))
    .context("Figure 1 missing")?;
  app.select_region(page, figure.bounds)?;
  let scroll = app.layout.project(page, figure.bounds, 0.0).y0;
  app.reading_view().scrub_scroll(scroll);
  let mut hashes = Vec::new();
  for _ in 0..2 {
    let mut output = Vec::new();
    app.graphics.begin(&mut output)?;
    {
      let (app, graphics) = app.parts();
      app.draw_region(graphics, &mut output)?;
    }
    app.graphics.end(&mut output)?;
    hashes.push(super::hash::digest(&output));
  }
  let text = app.doc.text(page)?;
  let text = text.in_region(figure.bounds);
  let bounds = figure.bounds;
  let screen = app.layout.screen();
  serde_json::to_vec(&serde_json::json!({
    "text_sha256": super::hash::digest(text.as_bytes()),
    "page": page.0,
    "bounds": [bounds.x0, bounds.y0, bounds.x1, bounds.y1],
    "screen": [screen.columns, screen.rows, screen.cell_width, screen.cell_height],
    "origin": screen.origin,
    "at": [app.at.column, app.at.row],
    "left": app.left,
    "scroll": app.reader.scroll,
    "output_sha256": hashes,
  })).map_err(Into::into)
}
