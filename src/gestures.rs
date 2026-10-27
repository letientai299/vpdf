//! Pointer gestures: click runs, drags, and buttons.

use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use std::time::Instant;
use vpdf_model::text::{PageId, Position};
use vpdf_ui::input::gesture::Swept;
use vpdf_ui::pointer::Route;

impl App {
  /// Alt-press: starts a capture; text extends.
  pub(crate) fn alt_press(&mut self, route: &Route) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.alt_press(reading.gesture, route)
  }

  /// Alt-drag: moves the capture's corner.
  pub(crate) fn alt_drag(&mut self, route: &Route) -> Result<()> {
    let Some(sweep) = self.gesture.sweep() else { return Ok(()) };
    let page = sweep.page();
    self.edge_follow(route)?;
    self.sweep_to(page, route);
    Ok(())
  }

  /// Moves the capture's corner to the pointer.
  fn sweep_to(&mut self, page: PageId, route: &Route) {
    let mut reading = self.reading_parts();
    reading.view.sweep_to(reading.gesture, page, route);
  }

  /// Alt-release: sweep, figure, else extend.
  pub(crate) fn alt_release(&mut self) -> Result<()> {
    let Some(sweep) = self.gesture.finish_sweep() else { return Ok(()) };
    match sweep.resolve(&mut PdfText(&mut self.doc))? {
      Swept::Region(page, bounds) => self.select_region(page, bounds),
      Swept::Extend(position) => self.extend_select(position),
    }
  }

  /// Whether an Alt-press sweeps a capture.
  pub(crate) fn sweeping(&self) -> bool {
    self.gesture.sweep().is_some()
  }

  /// The status hint while a sweep would capture.
  pub(crate) fn sweep_hint(&self) -> Option<String> {
    self.gesture.hint().map(str::to_owned)
  }

  /// Ends held gestures; whether a sweep was.
  pub(crate) fn end_gestures(&mut self) -> bool {
    self.autoscroll.cancel();
    self.drop_sweep()
  }

  /// Drops a held sweep; whether one was.
  pub(crate) fn drop_sweep(&mut self) -> bool {
    self.gesture.drop_sweep()
  }

  /// Middle clicks split links; drags pan.
  pub(crate) fn page_middle(&mut self, route: &Route) -> Result<()> {
    let mut reading = self.reading_parts();
    if reading.view.middle(reading.grab, route)? {
      let (column, row) = (route.mouse.column, route.mouse.row);
      if let Some(target) = self.link_pressed(column, row, route.pixel)? {
        self.open_split(&target)?;
      }
    }
    Ok(())
  }

  /// Whether a middle drag pans.
  pub fn grabbing(&self) -> bool {
    self.grab.moved()
  }

  /// Ends held drags, pans, and scrubs whose release may never come;
  /// whether any was held.
  pub(crate) fn drop_held(&mut self) -> bool {
    let panel = self
      .panel
      .as_mut()
      .and_then(|panel| panel.view.shelf_mut())
      .is_some_and(vpdf_ui::panel::recent::Shelf::release_pointer);
    panel | self.end_gestures() | self.drop_grab() | self.drop_scrub()
  }

  /// Ends a lost press; whether it panned.
  pub(crate) fn drop_grab(&mut self) -> bool {
    self.grab.cancel()
  }

  /// Click runs: target, word, line, paragraph.
  pub(crate) fn click_select(
    &mut self,
    pos: Position,
    clicks: u8,
  ) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.click_select(reading.gesture, pos, clicks)
  }

  pub(crate) fn drag_select(&mut self, pos: Position) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.drag_select(reading.gesture, pos)
  }

  pub(crate) fn extend_select(&mut self, pos: Position) -> Result<()> {
    let mut reading = self.reading_parts();
    reading.view.extend_select(reading.gesture, pos)
  }

  /// Rows an edge drag scrolls.
  pub(crate) fn edge_rows(&self, route: &Route) -> isize {
    let frame = self.frame(self.screen);
    vpdf_ui::input::edge::rows(frame, self.menu_rows(), route)
  }

  pub(crate) fn edge_follow(&mut self, route: &Route) -> Result<()> {
    let rows = self.edge_rows(route);
    if let Some(start) = self.autoscroll.follow(*route, rows) {
      self.edge_scroll(start.rows())?;
      start.begin(&mut self.autoscroll, Instant::now());
    }
    Ok(())
  }

  /// Steps a held edge drag; whether moved.
  pub(crate) fn autoscroll_step(&mut self, now: Instant) -> Result<bool> {
    let Some(held) = self.autoscroll.step(now) else { return Ok(false) };
    // A wheel's easing would hide the step.
    self.reader.cancel_scroll();
    let before = self.wheel_view();
    self.edge_scroll(held.rows)?;
    if self.wheel_view() == before {
      // At an end; the next report retries.
      self.autoscroll.cancel();
      return Ok(false);
    }
    if let Some(sweep) = self.gesture.sweep() {
      self.sweep_to(sweep.page(), &held.route);
    } else if !self.gesture.dropped() {
      self.drag_to(&held.route)?;
    }
    Ok(true)
  }

  /// Extends the drag's selection to the pointer.
  pub(crate) fn drag_to(&mut self, route: &Route) -> Result<()> {
    let Some(pos) = self.press_position(route)? else { return Ok(()) };
    self.drag_select(pos)
  }

  /// Scrolls `rows` at once, for drags.
  pub(crate) fn edge_scroll(&mut self, rows: isize) -> Result<()> {
    self.reading_view().edge_scroll(rows)
  }
}
