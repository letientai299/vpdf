use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use vpdf_model::geometry::Crop;
use vpdf_model::text::Position;
use vpdf_pdf::jobs::{Wake, recent::Worker};
use vpdf_ui::layout::Screen;

pub fn prepare(source: &Path, screen: Screen) -> Result<Vec<u8>> {
  let path = source.canonicalize()?;
  let saved = vpdf::state::Entry {
    path: path.clone(),
    cursor: Position::default(),
    zoom: 1.0,
    rotation: 0,
    used: 0,
    marks: BTreeMap::default(),
    crop: Crop::default(),
  };
  let rows = vpdf::panel::recent_rows(std::slice::from_ref(&saved));
  let row = rows.first().context("Missing recent row")?;
  let (label, detail) = (row.label.clone(), row.detail.clone());
  let entries = vpdf::recent::entries(&[saved]);
  let mut view = vpdf_ui::home::Home::new(rows, &entries, screen);
  let mut pending = None;
  view.draw(
    &mut Vec::new(),
    &mut vpdf_terminal::graphics::Graphics::default(),
    0,
    |cards| pending = cards.take_request(),
  )?;
  let request = pending.context("Missing home thumbnail request")?;
  let fit = request.fit;
  let worker = Worker::new("", &Wake::none());
  worker.requests.send(vpdf_pdf::jobs::recent::Request {
    path: request.path,
    cursor: request.cursor,
    rotation: request.rotation,
    fit,
  })?;
  let (_, loaded) = worker.results.recv_timeout(Duration::from_secs(30))?;
  let facts = loaded.facts.map_err(anyhow::Error::msg)?;
  let thumb = facts.thumb.context("Missing home thumbnail")?;
  serde_json::to_vec(&serde_json::json!({
    "path": path, "label": label, "detail": detail, "fit": fit,
    "screen": [screen.columns, screen.rows, screen.cell_width, screen.cell_height],
    "pages": facts.pages, "bytes": facts.bytes, "metadata": facts.metadata,
    "section": facts.section,
    "thumb": {"width":thumb.width,"height":thumb.height,"rgb":thumb.rgb},
  })).map_err(Into::into)
}
