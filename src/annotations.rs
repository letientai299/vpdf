use crate::Result;
use crate::app::App;
use crate::document::Annotation;
use crate::panel::{Destination, Row, Style};
use crate::pointer::Hit;
use crate::text::{Bounds, PageId, Position, Text};

/// Markup kinds whose marked text matters more than their contents.
const MARKUP: &[&str] = &["Highlight", "Underline", "Squiggly", "StrikeOut"];
/// Kinds drawn as fixed-size icons.
const ICONS: &[&str] = &["Note", "FileAttachment", "Sound"];

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
      for annotation in annotations.iter().cloned() {
        let (position, marked) = marked(&text, page, &annotation);
        let body = match (marked.is_empty(), annotation.contents.is_empty()) {
          (false, false) => format!("{marked} — {}", annotation.contents),
          (false, true) => marked,
          _ => annotation.contents.clone(),
        };
        rows.push(Row {
          style: Style::Key,
          ..Row::new(
            format!("{}: {body}", annotation.kind),
            self.doc.label(page).to_owned(),
            Destination::Annotation { position, annotation },
          )
        });
      }
    }
    Ok(rows)
  }

  pub(crate) fn annotation_at(
    &mut self,
    position: Position,
  ) -> Result<Option<Annotation>> {
    let Some((_, x, y)) = self.glyph_point(position)? else { return Ok(None) };
    let text = self.doc.text(position.page)?;
    let annotations = self.doc.annotations(position.page)?;
    let mut nearest = None;
    for annotation in annotations.iter() {
      if annotation.contents.is_empty() {
        continue;
      }
      if noted(annotation, x, y) {
        return Ok(Some(annotation.clone()));
      }
      if nearest.is_none()
        && !markup(annotation)
        && marked(&text, position.page, annotation).0 == position
      {
        nearest = Some(annotation.clone());
      }
    }
    Ok(nearest)
  }

  /// Opens a note's popup (U11).
  pub(crate) fn show_note(&mut self, page: PageId, index: usize) -> Result<()> {
    let annotations = self.doc.annotations(page)?;
    let Some(note) = annotations.get(index).cloned() else { return Ok(()) };
    let text = self.doc.text(page)?;
    let position = marked(&text, page, &note).0;
    self.preview_annotation(position, &note);
    if let Some(area) = note.areas.first().filter(|_| icon(&note)) {
      self.anchor_preview(page, index, *area);
    }
    Ok(())
  }

  /// Whether a note draws as an icon.
  pub(crate) fn note_icon(&mut self, page: PageId, index: usize) -> bool {
    let Ok(notes) = self.doc.annotations(page) else { return false };
    let icon = notes.get(index).is_some_and(icon);
    !self.text_mode && icon
  }

  /// The hovered note icon's areas.
  pub fn hover_icon(&mut self) -> Option<(PageId, Vec<Bounds>)> {
    let Hit::Note { page, index, text: false } = self.hover else {
      return None;
    };
    let annotations = self.doc.annotations(page).ok()?;
    Some((page, annotations.get(index)?.areas.clone()))
  }

  /// The commented annotation covering a page point.
  pub(crate) fn note_at(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<usize>> {
    let annotations = self.doc.annotations(page)?;
    Ok(annotations.iter().position(|annotation| noted(annotation, x, y)))
  }

  /// A hovered note: kind, author, comment.
  pub(crate) fn note_line(
    &mut self,
    page: PageId,
    index: usize,
  ) -> Result<Option<String>> {
    let annotations = self.doc.annotations(page)?;
    Ok(annotations.get(index).map(|note| match note.author.as_str() {
      "" => format!("{}: {}", note.kind, note.contents),
      author => format!("{} · {author}: {}", note.kind, note.contents),
    }))
  }
}

/// Whether a note marks text.
fn markup(note: &Annotation) -> bool {
  MARKUP.contains(&note.kind.as_str())
}

/// Whether a note draws as an icon.
fn icon(note: &Annotation) -> bool {
  ICONS.contains(&note.kind.as_str())
}

/// A commented annotation covers the point.
fn noted(annotation: &Annotation, x: f32, y: f32) -> bool {
  !annotation.contents.is_empty()
    && annotation.areas.iter().any(|area| area.contains(x, y))
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
  if !markup(annotation) {
    return (start, String::new());
  }
  let (first, marked) = text.in_areas(page, &annotation.areas);
  (first.unwrap_or(start), marked)
}
