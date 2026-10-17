use crate::Result;
use crate::app::App;
use crate::document::Annotation;
use crate::panel::{Destination, Row, Style};
use crate::text::{PageId, Position, Text};

/// Markup kinds whose marked text matters more than their contents.
const MARKUP: &[&str] = &["Highlight", "Underline", "Squiggly", "StrikeOut"];

impl App {
  /// Every annotation in page order, with marked text or contents.
  pub fn annotation_rows(&mut self) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    for page in (0..self.doc.pages.len()).map(PageId) {
      let annotations = self.doc.annotations(page)?;
      if annotations.is_empty() {
        continue;
      }
      let text = self.doc.text(page)?;
      for annotation in annotations {
        let (position, marked) = marked(&text, page, &annotation);
        let body = match (marked.is_empty(), annotation.contents.is_empty()) {
          (false, false) => format!("{marked} — {}", annotation.contents),
          (false, true) => marked,
          _ => annotation.contents,
        };
        rows.push(Row {
          style: Style::Key,
          ..Row::new(
            format!("{}: {body}", annotation.kind),
            self.doc.pages[page.0].label.clone(),
            Destination::Position(position),
          )
        });
      }
    }
    Ok(rows)
  }
}

/// First glyph and text inside a markup annotation's areas.
fn marked(
  text: &Text,
  page: PageId,
  annotation: &Annotation,
) -> (Position, String) {
  let start = annotation
    .areas
    .first()
    .map_or(Position { page, ..Position::default() }, |area| {
      text.nearest(page, area.x0, f32::midpoint(area.y0, area.y1))
    });
  if !MARKUP.contains(&annotation.kind.as_str()) {
    return (start, String::new());
  }
  let mut out = String::new();
  let mut first = None;
  for (line, row) in text.lines.iter().enumerate() {
    let before = out.len();
    for (glyph, ch) in row.glyphs.iter().enumerate() {
      let x = f32::midpoint(ch.bounds.x0, ch.bounds.x1);
      let y = f32::midpoint(ch.bounds.y0, ch.bounds.y1);
      if annotation.areas.iter().any(|area| area.contains(x, y)) {
        first.get_or_insert(Position { page, line, glyph });
        out.push(ch.ch);
      }
    }
    if out.len() > before {
      out.push(' ');
    }
  }
  (first.unwrap_or(start), out.split_whitespace().collect::<Vec<_>>().join(" "))
}
