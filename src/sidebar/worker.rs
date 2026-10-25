use super::{Request, Result};
use crate::pdf_text::PdfText;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};
use vpdf_model::{
  document::{Annotation, Target},
  text::{Bounds, PageId, Position, Text},
};
use vpdf_pdf::{content_cache::Record, document::Document};
use vpdf_ui::panel::{Destination, Row, Style};

#[derive(Deserialize, Serialize)]
enum Content {
  Link(Target),
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
      Destination::Link(target) => Content::Link(target.clone()),
      Destination::Annotation { position, annotation } => {
        Content::Annotation(*position, annotation.clone())
      }
      _ => return None,
    };
    Some(Self { label: row.label.clone(), detail: row.detail.clone(), content })
  }
  fn row(self, request: Request, pages: usize) -> Option<Row> {
    let (style, destination) = match (request, self.content) {
      (Request::Links(_), Content::Link(target)) => {
        let valid = match &target {
          Target::Page(page) => page.0 < pages,
          Target::Point { page, x, y } => {
            page.0 < pages && x.is_finite() && y.is_finite()
          }
          Target::Uri(_) => true,
        };
        if !valid {
          return None;
        }
        (Style::Plain, Destination::Link(target))
      }
      (Request::Annotations, Content::Annotation(position, annotation)) => {
        if position.page.0 >= pages
          || annotation.areas.iter().any(|b| {
            ![b.x0, b.y0, b.x1, b.y1].iter().all(|n| n.is_finite())
              || b.x0 > b.x1
              || b.y0 > b.y1
          })
        {
          return None;
        }
        (Style::Key, Destination::Annotation { position, annotation })
      }
      _ => return None,
    };
    Some(Row { label: self.label, detail: self.detail, style, destination })
  }
}

fn recall(cache: &Record, request: Request, pages: usize) -> Option<Vec<Row>> {
  let (count, entries): (usize, Vec<Entry>) = cache.load()?;
  if count != pages || entries.len() > 100_000 {
    return None;
  }
  entries.into_iter().map(|entry| entry.row(request, pages)).collect()
}

pub(super) fn scan(
  path: &Path,
  password: &str,
  bounds: Vec<Bounds>,
  request: Request,
  seeds: Vec<(PageId, Arc<Text>)>,
  stop: &AtomicBool,
  root: Option<PathBuf>,
) -> Result<Vec<Row>> {
  let pages = bounds.len();
  let ready = || vpdf_pdf::ready(stop);
  let cache = root.filter(|_| seeds.is_empty()).and_then(|root| {
    Record::open(root, path, password, request.kind(), &request.code(), ready)
  });
  if let Some(rows) =
    cache.as_ref().and_then(|cache| recall(cache, request, pages))
  {
    return Ok(rows);
  }
  anyhow::ensure!(ready(), "Sidebar load stopped");
  let mut doc = Document::open_known(path, password, bounds)?;
  // Open ignores `stop`; recheck before extraction.
  anyhow::ensure!(ready(), "Sidebar load stopped");
  for (page, text) in seeds {
    doc.seed(page, text);
  }
  let rows = match request {
    Request::Links(page) => link_rows(&mut doc, page, ready)?,
    Request::Annotations => annotation_rows(&mut doc, ready)?,
  };
  anyhow::ensure!(ready(), "Sidebar load stopped");
  if let Some(cache) = cache {
    let entries: Option<Vec<_>> = rows.iter().map(Entry::from_row).collect();
    if let Some(entries) = entries {
      cache.store(&(pages, entries));
    }
  }
  Ok(rows)
}

fn link_rows(
  doc: &mut Document,
  page: PageId,
  ready: impl Fn() -> bool,
) -> Result<Vec<Row>> {
  let links = doc.links(page)?;
  if links.is_empty() {
    return Ok(Vec::new());
  }
  let text = doc.text(page).ok();
  links
    .iter()
    .cloned()
    .map(|link| {
      anyhow::ensure!(ready(), "Sidebar load stopped");
      let (anchor, context) = vpdf_ui::panel::links::context(
        page,
        &link,
        text.as_deref(),
        &mut PdfText(doc),
      );
      Ok(vpdf_ui::panel::links::link_row(link, anchor, context, |page| {
        doc.get_label(page)
      }))
    })
    .collect()
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
