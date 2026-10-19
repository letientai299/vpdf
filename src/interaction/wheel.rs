use super::{App, Result, Route, WheelView};
use vpdf_ui::input::wheel::Turn;

impl App {
  /// A turn over a menu or the status row moves the surface beneath it.
  pub(super) fn menu_wheel(&mut self, route: Route) -> Result<()> {
    if self.covered() || route.mouse.column < self.left {
      return self.panel_mouse(route.mouse, route.clicks);
    }
    if self.overview.is_some() {
      return self.overview_mouse(route.mouse);
    }
    self.page_wheel(route)
  }

  /// The wheel over the page scrolls; Ctrl zooms at the pointer; Shift
  /// or a sideways turn pans; over the `K` preview it scrolls that.
  pub(super) fn page_wheel(&mut self, route: Route) -> Result<()> {
    let frame = vpdf_ui::paint::Frame {
      screen: self.screen,
      at: self.at,
      left: self.left,
      text: self.text_mode,
    };
    match self.wheel.turn(route, frame, self.peek.is_some()) {
      Turn::None => Ok(()),
      Turn::Zoom { steps, at } => self.zoom_at(steps, at),
      Turn::Pan(delta) => self.pan(delta),
      Turn::Scroll(delta) => self.scroll(delta),
      Turn::Preview(rows) => self.scroll_preview(rows),
    }
  }

  /// What a wheel turn can move, to tell whether one moved anything.
  pub(crate) fn wheel_view(&self) -> WheelView {
    WheelView {
      scroll: self.reader.animation().target().unwrap_or(self.reader.scroll),
      pan: self.layout.pan(),
      zoom: self.layout.zoom(),
      text: self.text_view.top(),
      panel: self.panel.as_ref().map(|panel| {
        (
          panel.view.top(),
          panel.view.shelf().map_or(0, vpdf_ui::panel::recent::Shelf::top),
        )
      }),
      overview: self.overview.as_ref().map(vpdf_ui::overview::Overview::top),
      preview: self.preview_top(),
      clues: self.clues.shown().map(vpdf_ui::clues::List::top),
    }
  }
}
