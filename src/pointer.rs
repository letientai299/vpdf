//! Mouse input on a pane: hits, presses, and the wheel.

use crate::Result;
use crate::app::App;
use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};
use vpdf_model::text::{PageId, Position};
use vpdf_ui::input::InputOwner;
use vpdf_ui::input::links::Step;
use vpdf_ui::input::mouse::{Content, Dispatch, Surface};
use vpdf_ui::input::wheel::{self, Background, Precise, Turn};
use vpdf_ui::layout::Zoom;
use vpdf_ui::pointer::{Hit, Pixel, Region, Route, WHEEL_ROWS};
use vpdf_ui::reading::{HitFrame, ScrollInput};

/// What a wheel turn moves; an equal view needs no frame.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct WheelView {
  scroll: f32,
  pan: f32,
  zoom: f32,
  text: Position,
  /// List and card tops.
  panel: Option<(usize, usize)>,
  overview: Option<usize>,
  clues: Option<usize>,
  preview: Option<i32>,
}

impl App {
  pub fn pointer_pane(&self) -> vpdf_ui::pointer::Pane {
    let rect = vpdf_ui::split::Rect {
      column: self.at.column,
      row: self.at.row,
      columns: self.screen.columns,
      rows: self.screen.rows,
    };
    vpdf_ui::pointer::Pane {
      id: self.id,
      rect,
      panel: self.left,
      overlay: self.menu_rows(),
      overview: self.overlays.overview().is_some(),
      preview: self.preview_cells(),
      menu: self.overlays.menu().map(vpdf_ui::context::ContextMenu::area),
      actions: self.overlays.region().and_then(vpdf_ui::region::Region::cells),
    }
  }
}

impl App {
  /// Shown glyph nearest a press.
  pub(crate) fn press_position(
    &mut self,
    route: &Route,
  ) -> Result<Option<Position>> {
    self.reading_view().press_position(route)
  }

  /// Link at a pressed, shown cell.
  pub(crate) fn link_pressed(
    &mut self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Option<vpdf_model::document::Target>> {
    let hit = self.pressed_hit(column, row, pixel)?;
    self.link_of(hit)
  }

  /// The target of a link hit.
  pub(crate) fn link_of(
    &mut self,
    hit: Hit,
  ) -> Result<Option<vpdf_model::document::Target>> {
    let Hit::Link(page, index) = hit else { return Ok(None) };
    Ok(self.link(page, index)?.map(|link| link.target))
  }

  /// What a pressed, shown page cell hits.
  pub(crate) fn pressed_hit(
    &mut self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Hit> {
    self.hit_at(Region::Page, column, row, pixel, HitFrame::Drawn)
  }

  /// Page-area pixels at a cell: reported, else centre.
  pub(crate) fn cell_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> (f32, f32) {
    self.frame(self.screen).point(column, row, pixel)
  }

  /// Page and point under the pointer in a pane cell.
  pub fn page_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Option<(PageId, f32, f32)> {
    self
      .layout
      .point_at(self.cell_point(column, row, pixel), self.reader.scroll)
  }

  /// What the pointer covers next frame.
  pub fn hovered(
    &mut self,
    region: Region,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Hit> {
    self.hit_at(region, column, row, pixel, HitFrame::Pending)
  }

  /// Hit geometry follows the requested frame.
  fn hit_at(
    &mut self,
    region: Region,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
    frame: HitFrame,
  ) -> Result<Hit> {
    if let Some(hit) = self.pointer_targets().hit(
      region,
      (column, row),
      || self.on_bar(column),
      || self.thumbnail_at(column, row),
      || self.chrome_at(column, row),
    ) {
      return Ok(hit);
    }
    self.reading_view().hit(frame, column, row, pixel)
  }
}

impl App {
  /// A screen report's cell in this pane.
  pub(crate) fn pane_cell(&self, mouse: MouseEvent) -> (u16, u16) {
    let column = mouse.column.wrapping_sub(self.at.column);
    (column, mouse.row.wrapping_sub(self.at.row))
  }

  /// Cancels pending keys, as `Esc` does.
  pub fn disarm(&mut self) {
    self.search.forget_yank();
    self.keys.clear();
    self.reader.disarm();
    self.after_key();
  }

  /// Closes key-driven state as focus leaves.
  pub fn blur(&mut self) -> Result<()> {
    if let Some(panel) = &mut self.panel {
      panel.view.leave_removal();
    }
    self.cancel_definition();
    // Focus loss closes this pane's overlays.
    self.dismiss_overlay();
    self.overlays.close_hints();
    self.drop_scrub();
    self.cancel_prompt()?;
    self.disarm();
    Ok(())
  }

  /// Acts on a routed pointer event.
  pub fn mouse(&mut self, route: Route) -> Result<()> {
    let Route { mouse, region, .. } = route;
    if mouse.kind != MouseEventKind::Moved {
      self.cancel_definition();
    }
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
      self.autoscroll.cancel();
    }
    if matches!(mouse.kind, MouseEventKind::Down(_)) && self.dismiss_click()? {
      return Ok(());
    }
    // A new press means a lost release.
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      self.drop_scrub();
    }
    // An overlay opened mid-press keeps the release.
    if mouse.kind == MouseEventKind::Up(MouseButton::Middle)
      && self.input_owner() != InputOwner::Content
    {
      self.grab.cancel();
    }
    let content = match self.pointer_targets().surface(mouse.kind, region) {
      Surface::Ignore => return Ok(()),
      Surface::MenuDrag => return self.menu_drag(route),
      Surface::MenuWheel => return self.menu_wheel(route),
      Surface::Panel => return self.panel_mouse(mouse, route.clicks),
      Surface::Clues(turn) => {
        let step = if turn.forward() { 1 } else { -1 };
        self.scroll_clues(step * WHEEL_ROWS as isize);
        return Ok(());
      }
      Surface::PageWheel => return self.page_wheel(route),
      Surface::Content(content) => content,
    };
    if matches!(mouse.kind, MouseEventKind::Down(_)) && self.panel_focused() {
      self.blur()?;
      if let Some(panel) = &mut self.panel {
        panel.view.focus(false);
      }
      self.relayout()?;
    }
    match content.dispatch(
      mouse,
      self.overlays.overview().is_some(),
      &self.bar,
      &self.gesture,
      || self.on_bar(mouse.column),
    ) {
      Dispatch::Ignore => Ok(()),
      Dispatch::Overview => self.overview_mouse(mouse),
      Dispatch::Wheel => self.page_wheel(route),
      Dispatch::Bar => self.bar_mouse(&route),
      Dispatch::Alt => self.page_alt(route),
      Dispatch::Left => self.page_left(route),
      Dispatch::Context => {
        self.page_context(route);
        Ok(())
      }
      Dispatch::Middle => self.page_middle(&route),
    }
  }

  /// Left button: select, or follow links.
  fn page_left(&mut self, route: Route) -> Result<()> {
    let mouse = route.mouse;
    match self.link_press.step(mouse.kind, route.clicks) {
      Step::Stop => return Ok(()),
      Step::Follow(target) => return self.follow(&target),
      Step::Note(page, index) => return self.show_note(page, index),
      Step::Drag(from) => {
        if let Some(from) = from {
          self.click_select(from, 1)?;
        }
        self.link_press.cancel();
      }
      Step::Begin { inspect } => {
        self.gesture.reset();
        // A broken link table still selects.
        let hit = inspect
          .then(|| self.pressed_hit(mouse.column, mouse.row, route.pixel));
        let hit = hit.and_then(Result::ok).unwrap_or_default();
        if let Hit::Note { page, index, text: false } = hit {
          // The cursor stays until a drag.
          self.reader.cancel_scroll();
          let from = self.press_position(&route)?;
          self.link_press.note(page, index, from);
          return Ok(());
        }
        let link = self.link_of(hit).ok().flatten();
        self.link_press.link(link);
      }
    }
    self.reader.cancel_scroll();
    if matches!(mouse.kind, MouseEventKind::Drag(_)) {
      self.edge_follow(&route)?;
      return self.drag_to(&route);
    }
    let Some(pos) = self.press_position(&route)? else { return Ok(()) };
    self.click_select(pos, route.clicks)
  }

  /// Alt-left on the page.
  fn page_alt(&mut self, route: Route) -> Result<()> {
    self.link_press.cancel();
    self.reader.cancel_scroll();
    match route.mouse.kind {
      MouseEventKind::Down(_) => self.alt_press(&route),
      MouseEventKind::Drag(_) => self.alt_drag(&route),
      _ => self.alt_release(),
    }
  }
}

impl App {
  /// What a turn over a menu or the status row moves.
  fn wheel_background(&self, route: Route) -> Background {
    wheel::background(
      route.mouse.column,
      self.left,
      self.covered(),
      self.overlays.overview().is_some(),
    )
  }

  /// Whether a turn reaches `page_wheel`, which takes fractions.
  fn page_turn(&self, route: Route) -> bool {
    match self.pointer_targets().surface(route.mouse.kind, route.region) {
      Surface::PageWheel => true,
      Surface::MenuWheel => self.wheel_background(route) == Background::Page,
      Surface::Content(Content::Page) => self.overlays.overview().is_none(),
      _ => false,
    }
  }

  /// A trackpad turn: the page follows its fraction; rows take whole
  /// turns.
  pub fn precise_wheel(
    &mut self,
    route: Route,
    precise: Precise,
  ) -> Result<()> {
    if self.page_turn(route) {
      self.wheel.precise(precise);
      return self.mouse(route);
    }
    for _ in 0..precise.whole {
      self.mouse(route)?;
    }
    Ok(())
  }

  /// A pinch zooms the page under the pointer by `factor`.
  pub fn pinch(&mut self, route: Route, factor: f32) -> Result<()> {
    let frame = self.frame(self.screen);
    if frame.text || !self.page_turn(route) {
      return Ok(());
    }
    self.cancel_definition();
    let mouse = route.mouse;
    let at = frame.point(mouse.column, mouse.row, route.pixel);
    self.zoom_at(Zoom::Scale(factor), at)
  }

  /// A turn over a menu or the status row moves the surface beneath it.
  pub(super) fn menu_wheel(&mut self, route: Route) -> Result<()> {
    match self.wheel_background(route) {
      Background::Panel => self.panel_mouse(route.mouse, route.clicks),
      Background::Overview => self.overview_mouse(route.mouse),
      Background::Page => self.page_wheel(route),
    }
  }

  /// The wheel over the page scrolls; Ctrl zooms at the pointer; Shift
  /// or a sideways turn pans; over the `K` preview it scrolls that.
  pub(super) fn page_wheel(&mut self, route: Route) -> Result<()> {
    let frame = self.frame(self.screen);
    match self.wheel.turn(route, frame, self.overlays.preview().is_some()) {
      Turn::None => Ok(()),
      Turn::Zoom { steps, at } => self.zoom_at(Zoom::Steps(steps), at),
      Turn::Pan(delta) => self.pan(delta),
      Turn::Scroll(delta) => self.scroll(delta),
      Turn::Exact(delta) => self.scroll_as(delta, ScrollInput::Precise),
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
      overview: self.overlays.overview().map(vpdf_ui::overview::Overview::top),
      preview: self.preview_top(),
      clues: self.overlays.clues().shown().map(vpdf_ui::clues::List::top),
    }
  }
}
