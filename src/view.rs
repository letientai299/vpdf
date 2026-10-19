//! Binds the pane to UI reading views, and lays it out.

use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Location;
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::Panel;

pub(crate) struct Reading<'a> {
  pub view: vpdf_ui::reading::View<'a, PdfText<'a>>,
  pub search: &'a mut vpdf_ui::find::Search,
  pub bar: &'a mut vpdf_ui::scrollbar::Bar,
  pub gesture: &'a mut vpdf_ui::input::gesture::Gesture,
  pub grab: &'a mut vpdf_ui::input::grab::Grab,
  pub outline: &'a mut vpdf_ui::reading::Outline,
  pub panel: Option<&'a mut Panel>,
  pub overlays: &'a mut vpdf_ui::overlays::Overlays,
}

impl App {
  pub fn reading_view(&mut self) -> vpdf_ui::reading::View<'_, PdfText<'_>> {
    self.reading_parts().view
  }

  pub(crate) fn reading_parts(&mut self) -> Reading<'_> {
    let frame = vpdf_ui::reading::Frame {
      pane: self.frame(self.screen),
      visible: self.uncovered(),
    };
    let view = self.reader.bind(
      &mut self.layout,
      &mut self.text_view,
      frame,
      PdfText(&mut self.doc),
    );
    Reading {
      view,
      search: &mut self.search,
      bar: &mut self.bar,
      gesture: &mut self.gesture,
      grab: &mut self.grab,
      outline: &mut self.outline,
      panel: self.panel.as_mut().map(|panel| &mut panel.view),
      overlays: &mut self.overlays,
    }
  }

  /// This pane's drawing frame over `screen`: the whole pane, the page
  /// area beside the panel, or that area above bottom lists.
  pub(crate) fn frame(&self, screen: Screen) -> vpdf_ui::paint::Frame {
    vpdf_ui::paint::Frame {
      at: self.at,
      screen,
      left: self.left,
      text: self.text_mode,
    }
  }

  pub(crate) fn bottom_overlay(&self) -> vpdf_ui::input::Overlay<'_> {
    self.overlays.bottom(self.screen)
  }

  /// Rows covered above the status row.
  pub fn menu_rows(&self) -> u16 {
    self.bottom_overlay().rows()
  }

  pub(crate) fn menu_top(&self) -> u16 {
    self.bottom_overlay().top()
  }

  pub fn uncovered(&self) -> Screen {
    self.bottom_overlay().uncovered(self.layout.screen())
  }

  pub fn relayout(&mut self) -> Result<()> {
    self.blank = false;
    let (left, screen) = vpdf_ui::panel::page_space(
      self.screen,
      self.panel.as_ref().map(|panel| &panel.view),
      self.panel_width,
    );
    self.left = left;
    let mut reading = self.reading_parts();
    reading.view.resize(screen, reading.bar)
  }

  pub fn navigation_position(&self) -> Position {
    vpdf_ui::reading::navigation(
      &self.reader,
      &self.layout,
      self.text_view.top(),
      self.text_mode,
    )
  }

  pub(crate) fn location(&mut self) -> Result<Location> {
    self.reading_view().location()
  }

  /// Page point at a page-area pixel, for a view to return to.
  pub(super) fn anchor_at(&self, at: (f32, f32)) -> (PageId, (f32, f32)) {
    self.layout.anchor_at(at, self.reader.scroll)
  }

  /// Scrolls and pans so `anchor` shows at a page-area pixel.
  pub(super) fn restore_at(
    &mut self,
    anchor: (PageId, (f32, f32)),
    at: (f32, f32),
  ) {
    self.reading_view().restore_at(anchor, at);
  }

  pub(crate) fn explicit_target(&mut self) -> Result<Option<Position>> {
    self.reading_view().explicit_target()
  }

  pub fn reading_position(&mut self) -> Result<Position> {
    self.reading_view().position()
  }

  pub(crate) fn first_visible(&mut self) -> Result<Option<Position>> {
    self.reading_view().first()
  }

  pub fn ensure_cursor(&mut self) -> Result<()> {
    self.reading_view().reveal()
  }
}
