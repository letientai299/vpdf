use super::View;
use crate::Result;
use crate::input::{gesture::Gesture, grab::Grab};
use crate::pointer::Route;
use crate::source::Pages;
use crate::text::{Bounds, PageId, Position};
use vpdf_model::input::MouseEventKind;
use vpdf_reader::reader::TextSource;

impl<S: TextSource + Pages> View<'_, S> {
  pub fn press_position(&mut self, route: &Route) -> Result<Option<Position>> {
    let mouse = route.mouse;
    if self.frame.pane.text {
      // Presses on rules miss; drags extend.
      let down = matches!(mouse.kind, MouseEventKind::Down(_));
      if down && self.text().is_break(mouse.row)? {
        return Ok(None);
      }
      return self.text().position(mouse.column, mouse.row).map(Some);
    }
    let Some((page, x, y)) = self.page_point(route) else {
      return Ok(None);
    };
    self.page().nearest_at(page, (x, y)).map(Some)
  }

  pub fn alt_press(
    &mut self,
    gesture: &mut Gesture,
    route: &Route,
  ) -> Result<()> {
    gesture.reset();
    if !self.frame.pane.text {
      let Some((page, x, y)) = self.page_point(route) else {
        return Ok(());
      };
      if self.shown_page(page).contains(x, y) {
        gesture.begin_sweep(page, (x, y));
        return Ok(());
      }
    }
    if let Some(position) = self.press_position(route)? {
      self.extend_select(gesture, position)?;
    }
    Ok(())
  }

  pub fn sweep_to(
    &mut self,
    gesture: &mut Gesture,
    page: PageId,
    route: &Route,
  ) {
    let at =
      self.frame.pane.point(route.mouse.column, route.mouse.row, route.pixel);
    let Some(point) = self.layout.unproject(page, at, self.reader.scroll)
    else {
      return;
    };
    gesture.sweep_to(point, self.shown_page(page));
  }

  /// Whether release should follow a link.
  pub fn middle(&mut self, grab: &mut Grab, route: &Route) -> Result<bool> {
    let frame = self.frame.pane;
    let at = frame.point(route.mouse.column, route.mouse.row, route.pixel);
    match route.mouse.kind {
      MouseEventKind::Down(_) => {
        self.animation.cancel();
        grab.begin(at);
      }
      MouseEventKind::Drag(_) => {
        if frame.text {
          let cell = (frame.screen.cell_width, frame.screen.cell_height);
          if let Some((columns, rows)) = grab.cells(at, cell) {
            self.text().scroll(rows)?;
            self.text().pan(columns)?;
          }
        } else if let Some((pan, delta)) = grab.pixels(at) {
          self.grab_page(delta, pan)?;
        }
      }
      _ => return Ok(grab.release()),
    }
    Ok(false)
  }

  fn page_point(&self, route: &Route) -> Option<(PageId, f32, f32)> {
    let at =
      self.frame.pane.point(route.mouse.column, route.mouse.row, route.pixel);
    self.layout.point_at(at, self.reader.scroll)
  }

  fn shown_page(&self, page: PageId) -> Bounds {
    self.layout.crop_for(page).apply(self.source.bounds(page))
  }
}
