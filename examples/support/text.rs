use anyhow::Result;
use vpdf_model::text::{Bounds, PageId};
use vpdf_pdf::document::Document;

pub fn coordinates(b: Bounds) -> [f32; 4] {
  [b.x0, b.y0, b.x1, b.y1]
}

pub fn prepare(doc: &mut Document) -> Result<Vec<u8>> {
  let mut pages = Vec::new();
  for index in 0..doc.pages.len() {
    let page = PageId(index);
    let text = doc.text(page)?;
    pages.push(serde_json::json!({
      "label": doc.label(page),
      "bounds": coordinates(doc.pages[index].bounds),
      "truncated": text.truncated,
      "lines": text.lines.iter().map(|line| (
        line.paragraph,
        line.glyphs.iter().map(|glyph| (glyph.ch, coordinates(glyph.bounds))).collect::<Vec<_>>()
      )).collect::<Vec<_>>(),
    }));
  }
  Ok(serde_json::to_vec(&pages)?)
}
