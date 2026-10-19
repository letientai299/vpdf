use crate::Result;
use crate::app::App;
use vpdf_model::document::Annotation;
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_ui::annotations::{icon, marked};
use vpdf_ui::panel::Row;
use vpdf_ui::pointer::Hit;

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
        rows.push(vpdf_ui::annotations::row(
          &text,
          page,
          annotation,
          self.doc.label(page),
        ));
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
    Ok(vpdf_ui::annotations::at(&annotations, &text, position, x, y))
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
    let Hit::Note { page, index, text: false } = self.hover.hit() else {
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
    Ok(vpdf_ui::annotations::hit(&annotations, x, y))
  }

  /// A hovered note: kind, author, comment.
  pub(crate) fn note_line(
    &mut self,
    page: PageId,
    index: usize,
  ) -> Result<Option<String>> {
    let annotations = self.doc.annotations(page)?;
    Ok(annotations.get(index).map(vpdf_ui::annotations::label))
  }
}
