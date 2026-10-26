use super::{Result, code};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use vpdf_model::{
  document::Annotation,
  text::{Bounds, PageId, Position, Text},
};
use vpdf_pdf::{
  content_cache::{Kind, Record},
  document::Document,
};
use vpdf_ui::panel::{Destination, Row, Style};

#[derive(Deserialize, Serialize)]
enum Content {
  Annotation(Position, Annotation),
}

#[derive(Deserialize, Serialize)]
struct Entry {
  label: String,
  detail: String,
  content: Content,
}

impl Entry {
  fn from_row(row: &Row) -> Option<Self> {
    let content = match &row.destination {
      Destination::Annotation { position, annotation } => {
        Content::Annotation(*position, annotation.clone())
      }
      _ => return None,
    };
    Some(Self { label: row.label.clone(), detail: row.detail.clone(), content })
  }
  fn row(self, pages: usize) -> Option<Row> {
    let Content::Annotation(position, annotation) = self.content;
    if position.page.0 >= pages
      || annotation.areas.iter().any(|b| {
        ![b.x0, b.y0, b.x1, b.y1].iter().all(|n| n.is_finite())
          || b.x0 > b.x1
          || b.y0 > b.y1
      })
    {
      return None;
    }
    let destination = Destination::Annotation { position, annotation };
    Some(Row {
      label: self.label,
      detail: self.detail,
      style: Style::Key,
      destination,
    })
  }
}

fn recall(cache: &Record, pages: usize) -> Option<Vec<Row>> {
  let (count, entries): (usize, Vec<Entry>) = cache.load()?;
  if count != pages || entries.len() > 100_000 {
    return None;
  }
  entries.into_iter().map(|entry| entry.row(pages)).collect()
}

pub(super) fn scan(
  path: &Path,
  password: &str,
  bounds: Vec<Bounds>,
  seeds: Vec<(PageId, Arc<Text>)>,
  stop: &AtomicBool,
  root: Option<PathBuf>,
) -> Result<Vec<Row>> {
  let pages = bounds.len();
  let ready = || vpdf_pdf::ready(stop);
  let cache = root.filter(|_| seeds.is_empty()).and_then(|root| {
    Record::open(root, path, password, Kind::Annotations, &code(), ready)
  });
  if let Some(rows) = cache.as_ref().and_then(|cache| recall(cache, pages)) {
    return Ok(rows);
  }
  anyhow::ensure!(ready(), "Sidebar load stopped");
  let mut doc = Document::open_known(path, password, bounds)?;
  // Open ignores `stop`; recheck before extraction.
  anyhow::ensure!(ready(), "Sidebar load stopped");
  for (page, text) in seeds {
    doc.seed(page, text);
  }
  let rows = annotation_rows(&mut doc, ready)?;
  anyhow::ensure!(ready(), "Sidebar load stopped");
  if let Some(cache) = cache {
    let entries: Option<Vec<_>> = rows.iter().map(Entry::from_row).collect();
    if let Some(entries) = entries {
      cache.store(&(pages, entries));
    }
  }
  Ok(rows)
}

fn annotation_rows(
  doc: &mut Document,
  ready: impl Fn() -> bool,
) -> Result<Vec<Row>> {
  let mut rows = Vec::new();
  for page in (0..doc.pages.len()).map(PageId) {
    anyhow::ensure!(ready(), "Sidebar load stopped");
    let annotations = doc.annotations(page)?;
    if annotations.is_empty() {
      continue;
    }
    let text = doc.text(page)?;
    for annotation in annotations.iter().cloned() {
      rows.push(vpdf_ui::annotations::row(
        &text,
        page,
        annotation,
        doc.label(page),
      ));
    }
  }
  Ok(rows)
}
