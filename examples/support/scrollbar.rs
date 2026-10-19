use anyhow::Result;
use serde_json::json;
use vpdf_model::document::section;
use vpdf_model::text::PageId;
use vpdf_pdf::document::Document;
use vpdf_search::search::{Index, compile};

pub fn prepare(doc: &mut Document) -> Result<Vec<u8>> {
  doc.load_outline()?;
  let pattern = compile("attention", false)?;
  let mut hits = Vec::new();
  let mut labels = Vec::new();
  for index in 0..doc.pages.len() {
    let page = PageId(index);
    let text = doc.text(page)?;
    hits.extend(Index::new(page, &text).matches(&pattern, 1000)?);
    let title = section(&doc.outline, page, 0.0)
      .map(|index| doc.outline[index].title.trim());
    let label = doc.label(page);
    labels.push(match title {
      Some(title) => format!("p. {label} · {title}"),
      None => format!("p. {label}"),
    });
  }
  Ok(serde_json::to_vec(&json!({
    "headings": doc.outline.iter().map(|h| (&h.title, h.depth, &h.target)).collect::<Vec<_>>(),
    "hits": hits.iter().map(|h| (&h.first, &h.last, &h.excerpt)).collect::<Vec<_>>(),
    "labels": labels,
  }))?)
}
