//! Links and notes: hover, follow, and listing.

use crate::Result;
use crate::app::App;
use crossterm::event::MouseEvent;
use std::sync::Arc;
use std::time::Instant;
use vpdf_model::document::{Annotation, Link, Target};
use vpdf_model::text::{Bounds, PageId, Position, Text};
use vpdf_pdf::document::Document;
use vpdf_ui::annotations::{icon, marked};
use vpdf_ui::panel::Row;
use vpdf_ui::panel::links::{cited, link_row};
use vpdf_ui::pointer::{Hit, Hover};
use vpdf_ui::preview::{Press, Preview};

/// The hovered link's page, that page's links, and index.
type Hovered = (PageId, Arc<Vec<Link>>, usize);

impl App {
  /// Takes `hover`; new links beneath rest.
  pub fn point(&mut self, hover: Option<Hover>) {
    self.hover.point(
      hover.map_or_default(|hover| hover.hit),
      hover.and_then(|hover| hover.beneath),
      Instant::now,
    );
  }

  fn hovered_links(&mut self) -> Option<Hovered> {
    let Hit::Link(page, index) = self.hover.hit() else { return None };
    // Hover is advisory; failures show nothing.
    let links = self.doc.links(page).ok()?;
    Some((page, links, index))
  }

  /// The hovered link and its page.
  pub fn hover_link(&mut self) -> Option<(PageId, Link)> {
    let (page, links, index) = self.hovered_links()?;
    Some((page, links.get(index)?.clone()))
  }

  /// The hovered link's boxes, across pages.
  pub fn hover_segments(&mut self) -> Vec<(PageId, Bounds)> {
    let Some((page, links, index)) = self.hovered_links() else {
      return Vec::new();
    };
    let Some(link) = links.get(index) else { return Vec::new() };
    let mut boxes: Vec<_> = link.boxes().map(|b| (page, b)).collect();
    boxes.extend(vpdf_pdf::wraps::page_parts(&mut self.doc, page, link));
    boxes
  }

  /// The hovered link's destination; citations show the reference.
  pub fn hover_label(&mut self) -> Option<String> {
    let (page, link) = self.hover_link()?;
    Some(self.link_label(page, &link))
  }

  /// A link's destination; citations show the reference.
  pub(crate) fn link_label(&mut self, page: PageId, link: &Link) -> String {
    if matches!(link.target, Target::Uri(_)) {
      return destination(&link.target, &self.doc, "");
    }
    // Hover is advisory; unreadable text reads as a bare reference.
    let text = self.doc.text(page).ok();
    let (anchor, context) = self.link_context(page, link, text.as_deref());
    let cited = cited(&anchor, context);
    destination(&link.target, &self.doc, &cited)
  }

  /// When the rested link previews.
  pub fn hover_due(&mut self) -> Option<Instant> {
    let (page, index, due) =
      self.hover.due(self.input_owner(), self.overlays.preview().is_some())?;
    let link = self.link(page, index).ok()??;
    (!matches!(link.target, Target::Uri(_))).then_some(due)
  }

  /// Opens a due preview.
  pub fn hover_preview(&mut self, now: Instant) -> bool {
    if self.hover_due().is_none_or(|due| now < due) {
      return false;
    }
    self.calm_hover();
    let Some((page, index)) = self.hover.resting() else { return false };
    let Ok(Some((target, source))) =
      self.reading_view().hover_target(page, index)
    else {
      return false;
    };
    if self.show_preview(target, source).is_err() {
      return false;
    }
    if let Some(peek) = self.overlays.preview_mut() {
      peek.hover(page, index);
    }
    true
  }

  /// The open hover preview.
  pub(crate) fn hover_peek(&self) -> Option<&Preview> {
    self.overlays.preview().filter(|peek| peek.hovered().is_some())
  }

  /// Motion on the link restarts its rest.
  pub(crate) fn rest_again(&mut self) {
    self.hover.restart(Instant::now);
  }

  /// Ends the rest until another link.
  pub(crate) fn calm_hover(&mut self) {
    self.hover.calm();
  }

  /// Closes a hover preview the pointer left.
  pub fn hover_leave(&mut self, at: Option<(u16, u16)>) -> bool {
    let Some(peek) = self.hover_peek() else { return false };
    let view = self.view_key();
    let rested = self.hover.resting();
    if peek.retains_hover(at, view, rested) {
      return false;
    }
    self.overlays.close_preview();
    true
  }

  /// Closes a hover preview; whether it did.
  pub(crate) fn drop_hover_preview(&mut self) -> bool {
    let open = self.hover_peek().is_some();
    if open {
      self.overlays.close_preview();
    }
    self.calm_hover();
    open
  }

  /// Inside: left follows, middle splits.
  pub fn preview_press(&mut self, mouse: MouseEvent) -> Result<bool> {
    let Some(peek) = self.overlays.preview() else { return Ok(false) };
    match peek.press(mouse, self.in_preview(mouse), self.input_owner()) {
      Press::Ignored => Ok(false),
      Press::Follow => {
        self.disarm();
        self.follow_preview()?;
        Ok(true)
      }
      Press::Split => {
        self.disarm();
        if let Some(target) =
          self.overlays.close_preview().and_then(Preview::into_target)
        {
          self.open_split(&target)?;
        }
        Ok(true)
      }
      Press::Dismiss { inside } => {
        self.drop_hover_preview();
        if inside {
          self.disarm();
        }
        Ok(inside)
      }
    }
  }

  /// Whether a press lands on the drawn preview.
  pub(crate) fn in_preview(&self, mouse: MouseEvent) -> bool {
    let (column, row) = self.pane_cell(mouse);
    self.preview_cells().is_some_and(|at| at.contains(column, row))
  }

  pub(crate) fn link_rows(&mut self) -> Result<Vec<Row>> {
    let page = self.navigation_position().page;
    let links = self.doc.links(page)?;
    if links.is_empty() {
      return Ok(Vec::new());
    }
    // Unreadable text must not hide destinations.
    let text = self.doc.text(page).ok();
    let mut rows = Vec::with_capacity(links.len());
    for link in links.iter().cloned() {
      let (anchor, context) = self.link_context(page, &link, text.as_deref());
      rows
        .push(link_row(link, anchor, context, |page| self.doc.get_label(page)));
    }
    Ok(rows)
  }

  fn link_context(
    &mut self,
    page: PageId,
    link: &Link,
    text: Option<&Text>,
  ) -> (String, String) {
    vpdf_ui::panel::links::context(
      page,
      link,
      text,
      &mut crate::pdf_text::PdfText(&mut self.doc),
    )
  }
}

pub(crate) fn destination(
  target: &Target,
  doc: &Document,
  cited: &str,
) -> String {
  vpdf_ui::panel::links::destination(
    target,
    &doc.outline,
    doc.pages.len(),
    |page| doc.label(page),
    cited,
  )
}

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

  /// The hovered note icon's areas.
  pub fn hover_icon(&mut self) -> Option<(PageId, Vec<Bounds>)> {
    let Hit::Note { page, index, text: false } = self.hover.hit() else {
      return None;
    };
    let annotations = self.doc.annotations(page).ok()?;
    Some((page, annotations.get(index)?.areas.clone()))
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;
  use vpdf_ui::keys::{Action, Input, PanelKind};
  use vpdf_ui::layout::Screen;
  use vpdf_ui::panel::Destination;

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn real_links_resolve_text() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 50,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&root.join("paper.pdf"), "", true, screen)?;
    app.go_to_page(PageId(1))?;
    let rows = app.panel_rows(PanelKind::Links)?;
    assert!(rows[0].label.starts_with("[13] Sepp Hochreiter"));
    assert!(rows[0].label.contains("Long short-term memory"));
    assert_eq!(rows[0].detail, "p. 11");
    for row in rows.iter().filter(|row| !row.detail.is_empty()).take(3) {
      let Destination::Link(target) = &row.destination else {
        panic!("Reference destination");
      };
      app.follow(target)?;
    }
    app.go_to_page(PageId(3))?;
    let rows = app.panel_rows(PanelKind::Links)?;
    assert!(rows[3].label.starts_with("4 To illustrate"));
    assert_eq!(rows[3].detail, "p. 4");

    app.go_to_page(PageId(10))?;
    app.open_panel(PanelKind::Links)?;
    let row = &app.panel.as_ref().unwrap().view.rows()[0];
    assert_eq!(row.label, "arXiv:1610.02357");
    // U21: hovers quote what references cite, as the panel does.
    let text = app.doc.text(PageId(2))?;
    let first = app.doc.links(PageId(2))?.iter().position(|link| {
      let cites = matches!(link.target, Target::Point { page: PageId(9), .. });
      cites && link.anchor(&text, PageId(2)) == "1"
    });
    let cases = [
      (1, 0, "→ p. 11 · [13] Sepp Hochreiter and Jürgen Schmidhuber."),
      // The first entry sits right under the bibliography heading.
      (2, first.expect("[1] on p. 3"), "→ p. 10 · [1] Jimmy Lei Ba"),
      (3, 3, "→ p. 4 · 4 To illustrate"),
      (10, 0, "→ http://arxiv.org/abs/1610.02357"),
    ];
    for (page, index, label) in cases {
      app.hover.point(Hit::Link(PageId(page), index), None, Instant::now);
      let shown = app.hover_label().unwrap_or_default();
      assert!(shown.starts_with(label), "{shown:?}");
    }
    app.panel_input(Input { register: 'a', ..Input::plain(Action::Yank) })?;
    assert_eq!(
      app.reader.register('a').map(|text| &**text),
      Some("http://arxiv.org/abs/1610.02357\n")
    );
    Ok(())
  }
}
