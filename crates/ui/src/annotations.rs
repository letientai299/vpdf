use crate::panel::{Destination, Row, RowKind, Style};
use crate::text::{PageId, Position, Text};
use vpdf_model::document::Annotation;

/// Markup kinds whose marked text matters more than their contents.
const MARKUP: &[&str] = &["Highlight", "Underline", "Squiggly", "StrikeOut"];
/// Kinds drawn as fixed-size icons.
const ICONS: &[&str] = &["Note", "FileAttachment", "Sound"];

/// A row quoting the marked text, else the note, else the kind.
pub fn row(text: &Text, page: PageId, annotation: Annotation) -> Row {
  let (position, marked) = marked(text, page, &annotation);
  let label = [marked, annotation.contents.clone(), annotation.kind.clone()]
    .into_iter()
    .find(|label| !label.is_empty())
    .unwrap_or_default();
  entry(label, position, annotation)
}

/// The row for an annotation and its label, also as cached.
pub fn entry(label: String, position: Position, annotation: Annotation) -> Row {
  let kind = match annotation.kind.as_str() {
    "Highlight" => RowKind::Highlight,
    "Underline" | "Squiggly" | "StrikeOut" => RowKind::Underline,
    _ => RowKind::Note,
  };
  // A bare kind name says nothing the glyph does not.
  let style =
    if label == annotation.kind { Style::Muted } else { Style::Plain };
  let destination = Destination::Annotation { position, annotation };
  Row { kind, style, ..Row::new(label, String::new(), destination) }
}

pub fn at(
  annotations: &[Annotation],
  text: &Text,
  position: Position,
  x: f32,
  y: f32,
) -> Option<Annotation> {
  let mut nearest = None;
  for annotation in annotations {
    if annotation.contents.is_empty() {
      continue;
    }
    if noted(annotation, x, y) {
      return Some(annotation.clone());
    }
    if nearest.is_none()
      && !markup(annotation)
      && marked(text, position.page, annotation).0 == position
    {
      nearest = Some(annotation.clone());
    }
  }
  nearest
}

pub fn hit(annotations: &[Annotation], x: f32, y: f32) -> Option<usize> {
  annotations.iter().position(|annotation| noted(annotation, x, y))
}

pub fn label(note: &Annotation) -> String {
  match note.author.as_str() {
    "" => format!("{}: {}", note.kind, note.contents),
    author => format!("{} · {author}: {}", note.kind, note.contents),
  }
}

/// Whether a note marks text.
fn markup(note: &Annotation) -> bool {
  MARKUP.contains(&note.kind.as_str())
}

/// Whether a note draws as an icon.
pub fn icon(note: &Annotation) -> bool {
  ICONS.contains(&note.kind.as_str())
}

/// A commented annotation covers the point.
fn noted(annotation: &Annotation, x: f32, y: f32) -> bool {
  !annotation.contents.is_empty()
    && annotation.areas.iter().any(|area| area.contains(x, y))
}

/// First glyph and text inside a markup annotation's areas.
pub fn marked(
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::Bounds;

  #[test]
  fn direct_hit_precedes_nearest_note() {
    let notes: Vec<_> = [0.0, 4.0]
      .into_iter()
      .map(|from| Annotation {
        kind: "Note".into(),
        author: String::new(),
        color: None,
        contents: from.to_string(),
        areas: vec![Bounds {
          x0: from,
          y0: from,
          x1: from + 2.0,
          y1: from + 2.0,
        }],
      })
      .collect();
    let text = Text::default();
    let position = Position::default();
    assert_eq!(at(&notes, &text, position, 5.0, 5.0), Some(notes[1].clone()));
    assert_eq!(hit(&notes, 5.0, 5.0), Some(1));
    assert_eq!(at(&notes, &text, position, 9.0, 9.0), Some(notes[0].clone()));
    assert_eq!(hit(&notes, 9.0, 9.0), None);
  }
}
