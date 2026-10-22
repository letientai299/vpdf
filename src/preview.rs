use crate::Result;
use crate::app::App;
use vpdf_model::document::{Annotation, Target};
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_ui::keys::Input;
use vpdf_ui::preview::{Event, Preview};
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
    let mut reading = self.reading_parts();
    reading.view.show_preview(reading.overlays, target, source)
  }

  pub(crate) fn preview_annotation(
    &mut self,
    position: Position,
    annotation: &Annotation,
  ) {
    let mut reading = self.reading_parts();
    reading.view.preview_annotation(reading.overlays, position, annotation);
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
    self.overlays.preview().map(Preview::top)
  }

  /// Identifies the view: scroll and text top.
  pub(crate) fn view_key(&self) -> (u32, Position) {
    (self.reader.scroll.to_bits(), self.text_view.top())
  }

  pub(crate) fn follow_preview(&mut self) -> Result<()> {
    let Some(target) =
      self.overlays.close_preview().and_then(Preview::into_target)
    else {
      return Ok(());
    };
    self.follow(&target)
  }

  pub fn scroll_preview(&mut self, rows: f32) -> Result<()> {
    if self.overlays.preview().is_none() {
      return Ok(());
    }
    let mut reading = self.reading_parts();
    reading.view.scroll_preview(reading.overlays, rows)
  }

  pub fn preview_cells(&self) -> Option<Rect> {
    self.overlays.preview().and_then(Preview::cells)
  }

  /// The previewed link's destination for status.
  pub(crate) fn preview_label(&mut self) -> Option<String> {
    let peek = self.overlays.preview().filter(|peek| peek.follows())?;
    let (target, hovered) = (peek.target().clone(), peek.hovered());
    // As on its link: citations show the reference.
    let link = hovered
      .and_then(|(page, index)| Some((page, self.link(page, index).ok()??)));
    if let Some((page, link)) = link {
      return Some(self.link_label(page, &link));
    }
    Some(crate::links::destination(&target, &self.doc, ""))
  }

  pub(crate) fn anchor_preview(
    &mut self,
    page: PageId,
    index: usize,
    icon: Bounds,
  ) {
    if let Some(peek) = self.overlays.preview_mut() {
      peek.anchor(page, index, icon);
    }
    self.drop_stray_preview();
  }

  /// Whether a press lands on the popup's icon.
  pub(crate) fn on_anchor(
    &mut self,
    mouse: vpdf_model::input::MouseEvent,
    pixel: Option<vpdf_ui::pointer::Pixel>,
  ) -> bool {
    let anchor = self
      .overlays
      .preview()
      .and_then(|peek| peek.shown_anchor(self.text_mode));
    let frame = self.frame(self.screen);
    vpdf_ui::preview::on_anchor(anchor, frame, mouse, |column, row| {
      self.pressed_hit(column, row, pixel).unwrap_or_default()
    })
  }

  pub fn drop_stray_preview(&mut self) {
    if self.overlays.preview().is_none() {
      return;
    }
    let mut reading = self.reading_parts();
    reading.view.drop_stray_preview(reading.overlays);
  }

  pub fn scene_preview<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
  ) -> Result<()>
  where
    S::Error: Into<anyhow::Error>,
  {
    if self.overlays.preview().is_none() {
      return Ok(());
    }
    let mut reading = self.reading_parts();
    reading.view.scene_preview(reading.overlays, scene)
  }
}
