use crate::Result;
use crate::app::App;
use std::io::Write;
use vpdf_model::document::{Annotation, Target};
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::keys::Input;
use vpdf_ui::pointer::Hit;
use vpdf_ui::preview::{Event, Frame, Preview};
use vpdf_ui::split::Rect;

impl App {
  pub fn preview_link(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    if let Some(annotation) = self.annotation_at(position)? {
      self.preview_annotation(position, &annotation);
      return Ok(());
    }
    let target = self
      .link_target(position)?
      .ok_or_else(|| anyhow::anyhow!("No link or note under cursor"))?;
    self.show_preview(target, position)
  }

  pub(crate) fn show_preview(
    &mut self,
    target: Target,
    source: Position,
  ) -> Result<()> {
    if let Target::Uri(uri) = target {
      self.reader.notice(uri);
      return Ok(());
    }
    let mut preview = Preview::open(
      target,
      source,
      false,
      self.text_mode,
      &self.layout,
      &mut crate::pdf_text::PdfText(&mut self.doc),
    )?;
    preview.keep(self.visible_position(source).unwrap_or(false));
    self.peek = Some(preview);
    self.scroll_preview(0.0)
  }

  pub(crate) fn preview_annotation(
    &mut self,
    position: Position,
    annotation: &Annotation,
  ) {
    if annotation.contents.is_empty() {
      return;
    }
    let seen = self.visible_position(position).unwrap_or(false);
    self.peek = Preview::note(position, annotation, seen, &self.layout);
  }

  pub(crate) fn preview_input(&mut self, input: Input) -> Result<()> {
    match Preview::input(input, &self.layout) {
      Event::Scroll(rows) => self.scroll_preview(rows),
      Event::Follow => self.follow_preview(),
      Event::Dismiss => {
        self.dismiss_overlay();
        Ok(())
      }
      Event::Action(input) => self.action(input),
      Event::Transfer(input) => {
        self.dismiss_overlay();
        self.action(input)
      }
      Event::Ignored => Ok(()),
    }
  }

  pub fn preview_top(&self) -> Option<i32> {
    self.peek.as_ref().map(Preview::top)
  }

  /// Identifies the view: scroll and text top.
  pub(crate) fn view_key(&self) -> (u32, Position) {
    (self.reader.scroll.to_bits(), self.text_view.top())
  }

  pub(crate) fn follow_preview(&mut self) -> Result<()> {
    let Some(target) = self.peek.take().and_then(Preview::into_target) else {
      return Ok(());
    };
    self.follow(&target)
  }

  pub fn scroll_preview(&mut self, rows: f32) -> Result<()> {
    let Some(preview) = &mut self.peek else { return Ok(()) };
    if !preview.scroll(
      rows,
      &self.layout,
      &mut crate::pdf_text::PdfText(&mut self.doc),
    )? {
      self.peek = None;
    }
    Ok(())
  }

  pub fn preview_cells(&self) -> Option<Rect> {
    self.peek.as_ref().and_then(Preview::cells)
  }

  /// The previewed link's destination for status.
  pub(crate) fn preview_label(&mut self) -> Option<String> {
    let peek = self.peek.as_ref().filter(|peek| peek.follows())?;
    let (target, hovered) = (peek.target().clone(), peek.hovered());
    // As on its link: citations show the reference.
    let link = hovered
      .and_then(|(page, index)| Some((page, self.link(page, index).ok()??)));
    if let Some((page, link)) = link {
      return Some(self.link_label(page, &link));
    }
    Some(crate::links::destination(&target, &self.doc, ""))
  }

  /// The hovered link's box; PDF view only.
  fn hovered_box(&mut self) -> Option<Bounds> {
    let (page, index) = self.peek.as_ref()?.hovered()?;
    let link = self.link(page, index).ok().flatten()?;
    let pdf = !self.text_mode;
    pdf.then(|| self.layout.project(page, link.first(), self.reader.scroll))
  }

  pub(crate) fn anchor_preview(
    &mut self,
    page: PageId,
    index: usize,
    icon: Bounds,
  ) {
    if let Some(peek) = &mut self.peek {
      peek.anchor(page, index, icon);
    }
    self.drop_stray_preview();
  }

  /// Whether a press lands on the popup's icon.
  pub(crate) fn on_anchor(
    &mut self,
    mouse: crossterm::event::MouseEvent,
    pixel: Option<vpdf_ui::pointer::Pixel>,
  ) -> bool {
    let Some((page, at, _)) = self.shown_anchor() else { return false };
    let (column, row) = self.pane_cell(mouse);
    // Page cells only: not the panel or status.
    if column < self.left || row + 1 >= self.screen.rows {
      return false;
    }
    let hit = self.pressed_hit(column, row, pixel).unwrap_or_default();
    hit == Hit::Note { page, index: at, text: false }
  }

  fn shown_anchor(&self) -> Option<(PageId, usize, Bounds)> {
    self.peek.as_ref()?.shown_anchor(self.text_mode)
  }

  /// Page-area pixels of a preview's icon.
  fn anchor_rect(&self) -> Option<Bounds> {
    let (page, _, icon) = self.shown_anchor()?;
    Some(self.layout.project(page, icon, self.reader.scroll))
  }

  pub fn drop_stray_preview(&mut self) {
    let Some(source) = self.peek.as_ref().map(Preview::source) else { return };
    let shown = match self.shown_anchor() {
      Some((page, _, icon)) => self.page_view().shows(page, icon),
      None => self.visible_position(source).unwrap_or(false),
    };
    if self.peek.as_mut().is_some_and(|peek| !peek.keep(shown)) {
      self.peek = None;
    }
  }

  pub fn draw_preview(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    if self.peek.is_none() {
      return Ok(());
    }
    self.scroll_preview(0.0)?;
    let Some(source) = self.peek.as_ref().map(Preview::source) else {
      return Ok(());
    };
    let cursor = match self.anchor_rect() {
      Some(rect) => Some(rect),
      None => self.position_rect(source)?,
    };
    let link = self.hovered_box();
    let identity = self.view_key();
    let Some(preview) = &mut self.peek else { return Ok(()) };
    preview.draw(
      graphics,
      out,
      Frame {
        layout: &self.layout,
        pane: vpdf_ui::paint::Frame {
          at: self.at,
          screen: self.screen,
          left: self.left,
          text: self.text_mode,
        },
        identity,
        cursor,
        link,
      },
      &crate::pdf_text::PdfText(&mut self.doc),
    )
  }
}
