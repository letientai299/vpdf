use super::View;
use crate::Result;
use crate::overlays::Overlays;
use crate::preview::{Frame, Preview};
use crate::source::{Markup, Pages, Raster};
use crate::text::{Bounds, PageId, Position};
use std::io::Write;
use vpdf_model::document::{Annotation, Target};
use vpdf_reader::reader::TextSource;
use vpdf_terminal::graphics::Graphics;

impl<S: TextSource + Pages> View<'_, S> {
  pub fn preview_annotation(
    &mut self,
    overlays: &mut Overlays,
    position: Position,
    annotation: &Annotation,
  ) {
    if annotation.contents.is_empty() {
      return;
    }
    let seen = self.visible(position).unwrap_or(false);
    overlays.preview = Preview::note(position, annotation, seen, self.layout);
  }

  pub fn drop_stray_preview(&mut self, overlays: &mut Overlays) {
    let preview = &mut overlays.preview;
    let Some(peek) = preview else { return };
    let shown = match peek.shown_anchor(self.frame.pane.text) {
      Some((page, _, icon)) => self.page().shows(page, icon),
      None => self.visible(peek.source()).unwrap_or(false),
    };
    if !peek.keep(shown) {
      *preview = None;
    }
  }
}

impl<S: TextSource + Pages + Raster> View<'_, S> {
  pub fn show_preview(
    &mut self,
    overlays: &mut Overlays,
    target: Target,
    position: Position,
  ) -> Result<()> {
    if let Target::Uri(uri) = target {
      self.reader.message = uri;
      return Ok(());
    }
    let mut peek = Preview::open(
      target,
      position,
      false,
      self.frame.pane.text,
      self.layout,
      &mut self.source,
    )?;
    peek.keep(self.visible(position).unwrap_or(false));
    overlays.preview = Some(peek);
    self.scroll_preview(overlays, 0.0)
  }

  pub fn scroll_preview(
    &mut self,
    overlays: &mut Overlays,
    rows: f32,
  ) -> Result<()> {
    let Some(peek) = &mut overlays.preview else { return Ok(()) };
    if !peek.scroll(rows, self.layout, &mut self.source)? {
      overlays.preview = None;
    }
    Ok(())
  }
}

impl<S: TextSource + Pages + Markup> View<'_, S> {
  fn hovered_box(&mut self, preview: &Preview) -> Option<Bounds> {
    let (page, index) = preview.hovered()?;
    let links = self.source.links(page).ok()?;
    let link = links.get(index)?;
    (!self.frame.pane.text)
      .then(|| self.layout.project(page, link.first(), self.reader.scroll))
  }

  pub fn hover_target(
    &mut self,
    page: PageId,
    index: usize,
  ) -> Result<Option<(Target, Position)>> {
    let links = self.source.links(page)?;
    let Some(link) = links.get(index) else { return Ok(None) };
    let text = self.source.page_text(page)?;
    let boxes: Vec<_> = link.boxes().collect();
    Ok(
      text.in_areas(page, &boxes).0.map(|source| (link.target.clone(), source)),
    )
  }
}

impl<S: TextSource + Pages + Markup + Raster> View<'_, S> {
  pub fn draw_preview(
    &mut self,
    overlays: &mut Overlays,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let mut scene =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    let result = self.scene_preview(overlays, &mut scene);
    scene.finish()?;
    result
  }

  pub fn scene_preview<'a, D: vpdf_model::scene::Surface<'a>>(
    &mut self,
    overlays: &mut Overlays,
    scene: &mut D,
  ) -> Result<()>
  where
    D::Error: Into<anyhow::Error>,
  {
    self.scroll_preview(overlays, 0.0)?;
    let Some(preview) = &mut overlays.preview else { return Ok(()) };
    let cursor = match preview.shown_anchor(self.frame.pane.text) {
      Some((page, _, icon)) => {
        Some(self.layout.project(page, icon, self.reader.scroll))
      }
      None => self.position_rect(preview.source())?,
    };
    let link = self.hovered_box(preview);
    let identity = (self.reader.scroll.to_bits(), self.text.top());
    preview
      .scene(
        scene,
        Frame {
          layout: self.layout,
          pane: self.frame.pane,
          identity,
          cursor,
          link,
        },
        &self.source,
      )
      .map_err(Into::into)
  }
}
