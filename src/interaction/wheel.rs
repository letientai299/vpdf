use super::{
  App, KeyModifiers, Region, Result, Route, Surface, WHEEL_ROWS, WHEEL_ZOOM,
  Wheel, WheelRun, WheelView,
};

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
    let mouse = route.mouse;
    let Some(turn) = Wheel::of(mouse.kind) else { return Ok(()) };
    let step = if turn.forward() { 1.0 } else { -1.0 };
    let rows = f32::from(WHEEL_ROWS);
    let sideways =
      turn.sideways() || mouse.modifiers.contains(KeyModifiers::SHIFT);
    let zoom = mouse.modifiers.contains(KeyModifiers::CONTROL);
    let Some(surface) = self.wheel_surface(route.region, sideways, zoom) else {
      return Ok(());
    };
    if zoom {
      // Sideways turns in a Ctrl gesture would pan off the anchor; text
      // shows no zoom.
      if sideways || self.text_mode {
        return Ok(());
      }
      let at = self.cell_point(mouse.column, mouse.row, route.pixel);
      return self.zoom_at(-step * WHEEL_ZOOM, at);
    }
    match (surface, sideways) {
      (Surface::Preview, false) => self.scroll_preview(step * rows),
      (Surface::Preview, true) => Ok(()),
      (Surface::Page, true) => {
        self.pan(step * rows * f32::from(self.screen.cell_width))
      }
      (Surface::Page, false) => {
        self.scroll(step * rows * f32::from(self.screen.cell_height))
      }
    }
  }

  /// What a turn moves: what its gesture's first turn hit, so a preview
  /// that slides under a resting pointer leaves the page scrolling; None
  /// for drift across the gesture's axis. Zooms keep the page.
  fn wheel_surface(
    &mut self,
    region: Region,
    sideways: bool,
    zoom: bool,
  ) -> Option<Surface> {
    if let Some(gesture) = self.wheel {
      let surface = match gesture.surface {
        Surface::Preview if self.peek.is_none() => Surface::Page,
        surface => surface,
      };
      return (gesture.sideways == sideways).then_some(surface);
    }
    let under = !zoom && region == Region::Preview;
    let surface = if under { Surface::Preview } else { Surface::Page };
    self.wheel = Some(WheelRun { surface, sideways });
    Some(surface)
  }

  /// What a wheel turn can move, to tell whether one moved anything.
  pub(crate) fn wheel_view(&self) -> WheelView {
    WheelView {
      scroll: self.animation.as_ref().map_or(self.reader.scroll, |a| a.target),
      pan: self.layout.pan,
      zoom: self.layout.zoom,
      text: self.text_top,
      panel: self.panel.as_ref().map(|panel| {
        (panel.top, panel.shelf.as_ref().map_or(0, crate::recent::Shelf::top))
      }),
      overview: self.overview.as_ref().map(crate::overview::Overview::top),
      preview: self.preview_top(),
      clues: self.clues.as_ref().map(|clues| clues.top),
    }
  }
}
