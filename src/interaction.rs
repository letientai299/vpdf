use crate::Result;
use crate::app::{App, InputOwner};
use crate::gestures::Gesture;
use crate::keys::{Action, Input};
use crate::layout::Screen;
use crate::links::LinkPress;
use crate::motion::Motion;
use crate::panel::Panel;
use crate::pointer::{
  Hit, Items, List, Pixel, Region, Route, WHEEL_ROWS, Wheel,
};
use crate::preview::Origin;
use crate::reader::Location;
use crate::region::BAR_HINT;
use crate::text::{Bounds, PageId, Position};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

/// Zoom steps per wheel turn: touchpads send a turn per cell of travel.
const WHEEL_ZOOM: f32 = 0.5;

/// What the turns of one wheel gesture move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WheelRun {
  pub surface: Surface,
  /// The first turn's axis; turns across it are drift.
  pub sideways: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Surface {
  Page,
  /// The `K` preview.
  Preview,
}

/// Finds a frame's glyph at a cell.
type GlyphAt = fn(&mut App, u16, u16) -> Result<Option<Position>>;

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
  preview: Option<i32>,
}

impl App {
  pub fn input_owner(&self) -> InputOwner {
    if self.prompt.is_some() {
      InputOwner::Prompt
    } else if self.hints.is_some() {
      InputOwner::Hints
    } else if self.region.as_ref().is_some_and(crate::region::Region::is_bar) {
      InputOwner::Region
    } else if self.context_menu.is_some() {
      InputOwner::Context
    } else if self.peek.as_ref().is_some_and(|peek| peek.origin == Origin::Key)
    {
      InputOwner::Peek
    } else if self.panel_focused() {
      InputOwner::Panel
    } else if self.overview.is_some() {
      InputOwner::Overview
    } else {
      InputOwner::Content
    }
  }

  pub(crate) fn dismiss_overlay(&mut self) {
    self.context_menu = None;
    self.end_gestures();
    self.peek = None;
    // The bar's key hint goes with it.
    if self.region.take().is_some() && self.reader.message == BAR_HINT {
      self.reader.message.clear();
    }
  }

  pub(crate) fn overlay_transfer(&mut self, action: Action) -> bool {
    if !matches!(
      action,
      Action::Window(_)
        | Action::Tab(_)
        | Action::LocalFocus
        | Action::History(_)
        | Action::Panel(_)
        | Action::ClosePanel
    ) {
      return false;
    }
    self.dismiss_overlay();
    true
  }

  /// Ends pending keys and notes.
  pub(crate) fn interrupt(&mut self) {
    self.disarm();
    self.clear_notes();
  }

  /// Drops the error and the message.
  pub(crate) fn clear_notes(&mut self) {
    self.error = None;
    self.reader.message.clear();
  }

  /// Whether a press only dismisses something.
  pub(crate) fn overlaid(&self) -> bool {
    self.clues.is_some()
      || matches!(
        self.input_owner(),
        InputOwner::Prompt
          | InputOwner::Hints
          | InputOwner::Peek
          | InputOwner::Region
          | InputOwner::Context
      )
  }

  pub(crate) fn dismiss_click(&mut self) -> Result<bool> {
    if !self.overlaid() {
      return Ok(false);
    }
    match self.input_owner() {
      InputOwner::Prompt => {
        self.cancel_input()?;
        return Ok(true);
      }
      InputOwner::Hints => self.hints = None,
      InputOwner::Peek => self.dismiss_overlay(),
      InputOwner::Region | InputOwner::Context => {
        self.dismiss_overlay();
        self.disarm();
        return Ok(true);
      }
      _ => {}
    }
    self.reader.cancel();
    self.disarm();
    Ok(true)
  }

  /// Rows the completion list or clues cover right above the status row.
  pub fn menu_rows(&self) -> u16 {
    let Some(menu) = self.prompt.as_ref().and_then(|p| p.menu.as_ref()) else {
      return self.clue_rows();
    };
    u16::try_from(menu.grid(self.screen).visible).unwrap_or(0)
  }

  /// Completions, by cells.
  pub(crate) fn completion_cells(&self) -> Items {
    let mut items = Items::default();
    let menu = self.prompt.as_ref().and_then(|prompt| prompt.menu.as_ref());
    let Some(menu) = menu else { return items };
    let grid = menu.grid(self.screen);
    for (cells, index) in grid.cells(menu.items.len(), self.menu_top()) {
      items.push(cells, index);
    }
    items
  }

  /// Top row of the menu.
  pub(crate) fn menu_top(&self) -> u16 {
    self.screen.rows.saturating_sub(1 + self.menu_rows())
  }

  /// Page area above the menu or clue rows; page graphics stop there.
  pub fn uncovered(&self) -> Screen {
    let mut screen = self.layout.screen;
    screen.rows = screen.rows.saturating_sub(self.menu_rows());
    screen
  }

  pub fn relayout(&mut self) -> Result<()> {
    let landed = self
      .target
      .filter(|&target| self.landed == Some(self.landed_view(target)));
    self.reflow_view()?;
    // The reflowed view keeps the landing.
    if let Some(target) = landed {
      self.landed = Some(self.landed_view(target));
    }
    Ok(())
  }

  fn reflow_view(&mut self) -> Result<()> {
    let anchor = self.view_anchor();
    let scrolled = self.reader.scroll;
    let size = |screen: Screen| {
      (screen.columns, screen.rows, screen.cell_width, screen.cell_height)
    };
    let before = size(self.layout.screen);
    let page = self.layout.pages[anchor.0.0].clone();
    self.blank = false;
    let columns = self.screen.columns;
    let covered = self.covered();
    let beside = match &self.panel {
      Some(panel) if !covered => {
        Panel::width(panel.kind, columns, self.panel_width)
      }
      _ => 0,
    };
    self.left = if covered { columns } else { beside };
    self.layout.screen = self.screen;
    self.layout.screen.columns = columns.saturating_sub(beside);
    self.layout.reflow(&self.doc.pages);
    if self.text_mode {
      self.text_top = self.clamp_text(self.text_top)?;
    }
    let after = &self.layout.pages[anchor.0.0];
    if before == size(self.layout.screen)
      && page.matrix == after.matrix
      && page.top.to_bits() == after.top.to_bits()
    {
      self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll);
      self.layout.pan =
        self.layout.pan.clamp(0.0, self.layout.pan_limit(anchor.0));
    } else {
      self.restore_view(anchor);
    }
    // Only user moves show the bar.
    if self.bar.seen.is_some_and(|seen| seen.to_bits() == scrolled.to_bits()) {
      self.bar.seen = Some(self.reader.scroll);
    }
    Ok(())
  }

  pub fn navigation_position(&self) -> Position {
    if !self.reader.viewport_mode() {
      return self.reader.cursor;
    }
    if self.text_mode {
      return Position { glyph: 0, ..self.text_top };
    }
    Position {
      page: self.layout.page_at(self.reader.scroll),
      ..Position::default()
    }
  }

  pub(crate) fn location(&mut self) -> Result<Location> {
    Ok(Location {
      cursor: self.reading_position()?,
      scroll: self.reader.scroll,
      anchor: Some(self.view_anchor()),
      text_top: self.text_mode.then_some(self.text_top),
      target: self.target,
    })
  }

  pub(crate) fn restore_location(&mut self, location: Location) -> Result<()> {
    self.animation = None;
    self.reader.cursor = crate::reader::clamp(location.cursor, &mut self.doc)?;
    match location.anchor {
      Some(anchor) => self.restore_view(anchor),
      None => self.reader.scroll = self.layout.clamp_scroll(location.scroll),
    }
    self.target = location.target;
    if let Some(top) = location.text_top {
      self.text_top = self.clamp_text(top)?;
    } else if self.text_mode {
      self.reveal_text()?;
    }
    if location.text_top.is_some() && !self.text_mode {
      self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
      self.ensure_cursor()?;
    }
    Ok(())
  }

  fn view_anchor(&self) -> (PageId, (f32, f32)) {
    self.anchor_at((self.layout.screen.width() / 2.0, 0.0))
  }

  /// Page point at a page-area pixel, for a view to return to.
  fn anchor_at(&self, at: (f32, f32)) -> (PageId, (f32, f32)) {
    self.pixel_point(at).map_or_else(
      || (self.layout.page_at(at.1 + self.reader.scroll), (0.0, 0.0)),
      |(page, x, y)| (page, (x, y)),
    )
  }

  fn restore_view(&mut self, anchor: (PageId, (f32, f32))) {
    self.restore_at(anchor, (self.layout.screen.width() / 2.0, 0.0));
  }

  /// Scrolls and pans so `anchor` shows at a page-area pixel.
  fn restore_at(
    &mut self,
    (page, (x, y)): (PageId, (f32, f32)),
    (left, top): (f32, f32),
  ) {
    self.animation = None;
    let rect =
      self.layout.project(page, Bounds { x0: x, x1: x, y0: y, y1: y }, 0.0);
    self.reader.scroll = self.layout.clamp_scroll(rect.y0 - top);
    let shown = self.layout.shown_pan(page);
    let max = self.layout.pan_limit(page);
    self.layout.pan = (shown + rect.x0 - left).clamp(0.0, max);
  }

  pub(crate) fn visible_position(
    &mut self,
    position: Position,
  ) -> Result<bool> {
    if self.text_mode {
      return Ok(self.text_cell(position)?.is_some());
    }
    let visible = self.layout.visible(self.reader.scroll);
    if !visible.contains(&position.page.0) {
      return Ok(false);
    }
    let text = self.doc.text(position.page)?;
    let Some(glyph) = text.glyph(position) else { return Ok(false) };
    Ok(self.bounds_shown(position.page, glyph.bounds))
  }

  /// Whether page-point `bounds` show, after crop.
  pub(crate) fn bounds_shown(&self, page: PageId, bounds: Bounds) -> bool {
    let crop = self.layout.crop.apply(self.doc.pages[page.0].bounds);
    if bounds.x1 <= crop.x0
      || bounds.x0 >= crop.x1
      || bounds.y1 <= crop.y0
      || bounds.y0 >= crop.y1
    {
      return false;
    }
    let rect = self.layout.project(page, bounds, self.reader.scroll);
    rect.x1 > 0.0
      && rect.x0 < self.layout.screen.width()
      && rect.y1 > 0.0
      && rect.y0 < self.uncovered().height()
  }

  pub(crate) fn explicit_target(&mut self) -> Result<Option<Position>> {
    let target = if self.reader.uses_cursor() {
      Some(self.reader.cursor)
    } else {
      self.target
    };
    if let Some(target) = target
      && self.visible_position(target)?
    {
      return Ok(Some(target));
    }
    self.reader.message = "Click text or jump to select a target".into();
    Ok(None)
  }

  pub(crate) fn enter_text(&mut self) -> Result<bool> {
    if let Some(target) = self.target
      && self.visible_position(target)?
    {
      self.reader.cursor = target;
      self.reader.desired = None;
      self.reader.message.clear();
      return Ok(true);
    }
    if let Some(position) = self.first_visible()? {
      self.reader.cursor = position;
      self.reader.desired = None;
      self.reader.message.clear();
      return Ok(true);
    }
    self.reader.message = "Selection unavailable: no visible text".into();
    Ok(false)
  }

  pub fn reading_position(&mut self) -> Result<Position> {
    if !self.reader.viewport_mode() {
      return Ok(self.reader.cursor);
    }
    if let Some(target) = self.target
      && self.visible_position(target)?
    {
      return Ok(target);
    }
    Ok(self.first_visible()?.unwrap_or_else(|| self.navigation_position()))
  }

  pub(crate) fn first_visible(&mut self) -> Result<Option<Position>> {
    if self.text_mode {
      return self.first_text();
    }
    for page in self.layout.visible(self.reader.scroll).map(PageId) {
      let text = self.doc.text(page)?;
      let positions = text.lines.iter().enumerate().flat_map(|(line, row)| {
        row
          .glyphs
          .iter()
          .enumerate()
          .filter(|(_, ch)| !ch.ch.is_whitespace())
          .map(move |(glyph, _)| Position { page, line, glyph })
      });
      for position in positions {
        if self.visible_position(position)? {
          return Ok(Some(position));
        }
      }
    }
    Ok(None)
  }

  /// Shows `page` from its top; `ensure_cursor` then moves least.
  pub(crate) fn show_page(&mut self, page: PageId) -> Result<()> {
    self.reader.scroll = self.layout.pages[page.0].top;
    if self.text_mode {
      let top = Position { page, ..Position::default() };
      self.text_top = self.clamp_text(top)?;
    }
    Ok(())
  }

  /// `show_page` unless text draws `position`'s line.
  pub(crate) fn show_page_of(&mut self, position: Position) -> Result<()> {
    if self.text_mode && self.text_row(position)?.is_some() {
      return Ok(());
    }
    self.show_page(position.page)
  }

  pub fn ensure_cursor(&mut self) -> Result<()> {
    self.animation = None;
    if self.text_mode {
      return self.reveal_text();
    }
    let cursor = self.reader.cursor;
    let Some(rect) = self.glyph_rect(cursor)? else {
      self.reader.scroll = self.layout.pages[cursor.page.0].top;
      return Ok(());
    };
    let margin = f32::from(self.screen.cell_height);
    if rect.y0 < self.reader.scroll + margin {
      self.reader.scroll = rect.y0 - margin;
    }
    if rect.y1 > self.reader.scroll + self.layout.screen.height() - margin {
      self.reader.scroll = rect.y1 - self.layout.screen.height() + margin;
    }
    let pan = self.layout.shown_pan(cursor.page);
    if rect.x0 < 0.0 {
      self.layout.pan = (pan + rect.x0).max(0.0);
    }
    if rect.x1 > self.layout.screen.width() {
      self.layout.pan = pan + rect.x1 - self.layout.screen.width();
    }
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll);
    Ok(())
  }

  /// Glyph pixels of `position` at scroll 0.
  fn glyph_rect(&mut self, position: Position) -> Result<Option<Bounds>> {
    let text = self.doc.text(position.page)?;
    let glyph = text.glyph(position);
    Ok(glyph.map(|glyph| self.layout.project(position.page, glyph.bounds, 0.0)))
  }

  /// Scroll that shows `position` below one row.
  fn landed_scroll(&mut self, position: Position) -> Result<Option<f32>> {
    let row = f32::from(self.screen.cell_height);
    let rect = self.glyph_rect(position)?;
    Ok(rect.map(|rect| self.layout.clamp_scroll(rect.y0 - row)))
  }

  /// Shows the cursor; scrolling keeps context above.
  pub(crate) fn land(&mut self) -> Result<()> {
    if self.text_mode {
      return self.land_text();
    }
    let before = self.reader.scroll;
    self.ensure_cursor()?;
    if self.reader.scroll.total_cmp(&before).is_eq() {
      return Ok(());
    }
    if let Some(scroll) = self.landed_scroll(self.reader.cursor)? {
      self.reader.scroll = scroll;
    }
    Ok(())
  }

  fn normal_input(&mut self, input: Input) -> Result<bool> {
    let count = input.count;
    match input.action {
      Action::Cursor => {
        if self.enter_text()? {
          self.reader.enter_cursor();
        }
        return Ok(true);
      }
      Action::Motion(Motion::Left | Motion::Right) => {
        let direction =
          if input.action == Action::Motion(Motion::Left) { -1.0 } else { 1.0 };
        self
          .pan(direction * count as f32 * f32::from(self.screen.cell_width))?;
        return Ok(true);
      }
      Action::Motion(Motion::Down | Motion::Up) => {
        let direction =
          if input.action == Action::Motion(Motion::Up) { -1.0 } else { 1.0 };
        self.scroll(
          direction * count as f32 * f32::from(self.screen.cell_height),
        )?;
        return Ok(true);
      }
      Action::Motion(
        Motion::First | Motion::Last | Motion::NextPage | Motion::PreviousPage,
      ) => {
        self.reader.cursor = self.reading_position()?;
      }
      Action::Motion(_) | Action::Object(..) | Action::Swap => {
        return Ok(true);
      }
      Action::Visual(_) | Action::Yank | Action::YankLine => {
        if !self.enter_text()? {
          return Ok(true);
        }
      }
      _ => {}
    }
    Ok(false)
  }

  pub fn navigate(&mut self, input: Input) -> Result<bool> {
    let count = input.count;
    let normal = self.reader.viewport_mode();
    let copying = self.reader.operator.is_some();
    if normal && self.normal_input(input)? {
      return Ok(true);
    }
    let previous_page = self.reader.cursor.page;
    match input.action {
      Action::Motion(motion) => {
        let boundary = matches!(
          motion,
          Motion::First
            | Motion::Last
            | Motion::NextPage
            | Motion::PreviousPage
        );
        let origin = if boundary && self.reader.operator.is_none() {
          Some(self.location()?)
        } else {
          None
        };
        self.reader.motion(motion, count, input.explicit, &mut self.doc)?;
        if let Some(origin) = origin {
          self.reader.record(origin);
        }
        if normal {
          self.target = Some(self.reader.cursor);
        }
      }
      Action::Section(forward) => {
        self.move_section(forward, count)?;
        return Ok(true);
      }
      Action::Yank => self.reader.yank(count, input.register, &mut self.doc)?,
      Action::YankLine => {
        self.reader.yank_lines(count, input.register, &mut self.doc)?;
      }
      Action::YankRaw => self.reader.yank_raw(count, &mut self.doc)?,
      Action::Visual(kind) => self.reader.visual(kind),
      Action::Object(kind, around) => {
        self.reader.select_object(kind, around, count, &mut self.doc)?;
      }
      Action::Swap => self.reader.swap(),
      Action::Reselect => {
        self.reader.reselect();
        if self.reader.selection.is_none() {
          return Ok(true);
        }
      }
      Action::History(forward) => {
        self.animation = None;
        let current = self.location()?;
        if let Some(location) = self.reader.recall(forward, current) {
          // History leaves the grid, as from panels.
          self.close_overview();
          self.restore_location(location)?;
          self.target = Some(self.reader.cursor);
        }
        return Ok(true);
      }
      Action::Quit => self.quit = true,
      _ => return Ok(false),
    }
    if matches!(
      input.action,
      Action::Yank | Action::YankLine | Action::YankRaw | Action::Visual(_)
    ) || (self.reader.selection.is_none()
      && copying
      && matches!(input.action, Action::Motion(_) | Action::Object(..)))
    {
      return Ok(true);
    }
    if self.reader.cursor.page != previous_page {
      self.show_page_of(self.reader.cursor)?;
    }
    self.ensure_cursor()?;
    Ok(true)
  }

  pub fn viewport(&mut self, input: Input) -> Result<bool> {
    let count = input.count as f32;
    match input.action {
      Action::Scroll(fraction) => {
        self.scroll(fraction * count * self.layout.screen.height())?;
      }
      Action::Rows(rows) => {
        self
          .scroll(rows as f32 * count * f32::from(self.screen.cell_height))?;
      }
      Action::Align(fraction) if self.text_mode => {
        if self.reader.viewport_mode() {
          self.reader.cursor = self.reading_position()?;
        }
        self.align_text(fraction)?;
      }
      Action::Align(fraction) => {
        self.animation = None;
        if self.reader.viewport_mode() {
          self.reader.cursor = self.reading_position()?;
        }
        if let Some(rect) = self.glyph_rect(self.reader.cursor)? {
          self.reader.scroll = self.layout.clamp_scroll(
            rect.y0 - fraction * (self.layout.screen.height() - rect.height()),
          );
        }
      }
      Action::Zoom(direction) => self.zoom_by(direction as f32 * count)?,
      Action::Fit => {
        self.layout.zoom = 1.0;
        self.relayout()?;
      }
      Action::FitPage => {
        let page = &self.layout.pages[self.navigation_position().page.0];
        self.layout.zoom *= (self.layout.screen.height() / page.height)
          .min(self.layout.screen.width() / page.width);
        self.relayout()?;
      }
      Action::Crop => self.toggle_crop()?,
      Action::Rotate => {
        self.layout.rotation = (self.layout.rotation + 90) % 360;
        self.relayout()?;
      }
      Action::Pan(direction) => {
        self.pan(
          direction as f32 * count * f32::from(self.screen.cell_width) * 4.0,
        )?;
      }
      _ => return Ok(false),
    }
    Ok(true)
  }

  /// Zooms by `steps` of 15 %.
  fn zoom_by(&mut self, steps: f32) -> Result<()> {
    self.layout.zoom =
      (self.layout.zoom * 1.15_f32.powf(steps)).clamp(0.1, 8.0);
    self.relayout()
  }

  /// Zooms keeping the page point at a page-area pixel in place, where
  /// the view allows.
  fn zoom_at(&mut self, steps: f32, at: (f32, f32)) -> Result<()> {
    let anchor = self.anchor_at(at);
    self.zoom_by(steps)?;
    self.restore_at(anchor, at);
    Ok(())
  }

  pub(crate) fn pan(&mut self, delta: f32) -> Result<()> {
    if self.text_mode {
      let columns = (delta / f32::from(self.screen.cell_width)).round();
      return self.pan_text(columns as isize);
    }
    let page = self.navigation_position().page;
    let max = self.layout.pan_limit(page);
    self.layout.pan = (self.layout.pan + delta).clamp(0.0, max);
    Ok(())
  }

  /// A screen report's cell in this pane.
  pub(crate) fn pane_cell(&self, mouse: MouseEvent) -> (u16, u16) {
    let column = mouse.column.wrapping_sub(self.at.column);
    (column, mouse.row.wrapping_sub(self.at.row))
  }

  /// Cancels pending keys, as `Esc` does.
  pub fn disarm(&mut self) {
    self.keys.clear();
    self.reader.disarm();
    self.after_key();
  }

  /// Closes key-driven state as focus leaves.
  pub fn blur(&mut self) -> Result<()> {
    // Focus loss closes this pane's overlays.
    self.dismiss_overlay();
    self.hints = None;
    self.drop_scrub();
    self.cancel_prompt()?;
    self.disarm();
    Ok(())
  }

  /// Acts on a routed pointer event.
  pub fn mouse(&mut self, route: Route) -> Result<()> {
    let Route { mouse, region, .. } = route;
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
      self.autoscroll = None;
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
      self.grab = None;
    }
    let right = matches!(
      mouse.kind,
      MouseEventKind::Drag(MouseButton::Right)
        | MouseEventKind::Up(MouseButton::Right)
    );
    if right && self.context_menu.is_some() {
      return self.menu_drag(route);
    }
    // The page still scrolls under a menu.
    let wheel = Wheel::of(mouse.kind).is_some();
    if self.prompt.is_some()
      || self.context_menu.is_some() && !wheel
      || self.region.as_ref().is_some_and(crate::region::Region::is_bar)
    {
      return Ok(());
    }
    match region {
      Region::Panel => return self.panel_mouse(mouse, route.clicks),
      Region::Overlay | Region::Status | Region::Actions | Region::Edge => {
        return Ok(());
      }
      // Only turns: presses go to them first.
      Region::Preview => return self.page_wheel(route),
      Region::Menu => return self.menu_wheel(route),
      Region::Page | Region::Overview => {}
    }
    if matches!(mouse.kind, MouseEventKind::Down(_)) && self.panel_focused() {
      self.blur()?;
      if let Some(panel) = &mut self.panel {
        panel.focused = false;
      }
      self.relayout()?;
    }
    if region == Region::Overview {
      return self.overview_mouse(mouse);
    }
    // Drags from before `gp` stay hidden.
    if self.overview.is_some() {
      return Ok(());
    }
    match mouse.kind {
      kind if Wheel::of(kind).is_some() => self.page_wheel(route),
      MouseEventKind::Down(MouseButton::Left)
        if self.on_bar(mouse.column) && mouse.modifiers.is_empty() =>
      {
        self.bar_mouse(&route)
      }
      MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left)
        if self.bar.scrub.is_some() =>
      {
        self.bar_mouse(&route)
      }
      MouseEventKind::Down(MouseButton::Left)
        if mouse.modifiers.contains(KeyModifiers::ALT) =>
      {
        self.page_alt(route)
      }
      MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left)
        if matches!(self.gesture, Gesture::Sweep(_) | Gesture::Dropped) =>
      {
        self.page_alt(route)
      }
      MouseEventKind::Down(MouseButton::Left)
      | MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left) => self.page_left(route),
      MouseEventKind::Down(MouseButton::Right) => {
        self.page_context(route);
        Ok(())
      }
      MouseEventKind::Down(MouseButton::Middle)
      | MouseEventKind::Drag(MouseButton::Middle)
      | MouseEventKind::Up(MouseButton::Middle) => self.page_middle(&route),
      _ => Ok(()),
    }
  }

  /// A turn over a menu moves the surface beneath it.
  fn menu_wheel(&mut self, route: Route) -> Result<()> {
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
  fn page_wheel(&mut self, route: Route) -> Result<()> {
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
    }
  }

  /// Left button: select, or follow links.
  fn page_left(&mut self, route: Route) -> Result<()> {
    let mouse = route.mouse;
    match mouse.kind {
      MouseEventKind::Up(_) => {
        return match std::mem::take(&mut self.link_press) {
          LinkPress::Held(target) => {
            self.link_press = LinkPress::Followed;
            self.follow(&target)
          }
          LinkPress::Note(page, index, _) => {
            self.link_press = LinkPress::Followed;
            self.show_note(page, index)
          }
          press => {
            self.link_press = press;
            Ok(())
          }
        };
      }
      // A followed run's later presses do nothing.
      _ if route.clicks > 1
        && matches!(self.link_press, LinkPress::Followed) =>
      {
        return Ok(());
      }
      MouseEventKind::Drag(_) => {
        // An icon press selects once it drags.
        if let LinkPress::Note(.., Some(from)) = self.link_press {
          self.click_select(from, 1)?;
        }
        self.link_press = LinkPress::Idle;
      }
      _ => {
        self.gesture = Gesture::Glyphs;
        // A broken link table still selects.
        let hit = (route.clicks == 1)
          .then(|| self.pressed_hit(mouse.column, mouse.row, route.pixel));
        let hit = hit.and_then(Result::ok).unwrap_or_default();
        if let Hit::Note { page, index, text: false } = hit {
          // The cursor stays until a drag.
          self.animation = None;
          let from = self.press_position(&route)?;
          self.link_press = LinkPress::Note(page, index, from);
          return Ok(());
        }
        let link = self.link_of(hit).ok().flatten();
        self.link_press = link.map_or(LinkPress::Idle, LinkPress::Held);
      }
    }
    self.animation = None;
    if matches!(mouse.kind, MouseEventKind::Drag(_)) {
      self.edge_follow(&route)?;
      return self.drag_to(&route);
    }
    let Some(pos) = self.press_position(&route)? else { return Ok(()) };
    self.click_select(pos, route.clicks)
  }

  /// Alt-left on the page.
  fn page_alt(&mut self, route: Route) -> Result<()> {
    self.link_press = LinkPress::Idle;
    self.animation = None;
    match route.mouse.kind {
      MouseEventKind::Down(_) => self.alt_press(&route),
      MouseEventKind::Drag(_) => self.alt_drag(&route),
      _ => self.alt_release(),
    }
  }

  /// Shown glyph nearest a press.
  pub(crate) fn press_position(
    &mut self,
    route: &Route,
  ) -> Result<Option<Position>> {
    let mouse = route.mouse;
    if self.text_mode {
      // Presses on rules, like page gaps, miss.
      let down = matches!(mouse.kind, MouseEventKind::Down(_));
      if down && self.text_break(mouse.row)? {
        return Ok(None);
      }
      return self.text_position(mouse.column, mouse.row).map(Some);
    }
    let point = self.page_point(mouse.column, mouse.row, route.pixel);
    let Some((page, x, y)) = point else { return Ok(None) };
    self.glyph_near(page, x, y).map(Some)
  }

  /// The glyph nearest a page point.
  pub(crate) fn glyph_near(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Position> {
    Ok(self.doc.text(page)?.nearest(page, x, y))
  }

  /// Link at a pressed, shown cell.
  pub(crate) fn link_pressed(
    &mut self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Option<crate::document::Target>> {
    let hit = self.pressed_hit(column, row, pixel)?;
    self.link_of(hit)
  }

  /// The target of a link hit.
  pub(crate) fn link_of(
    &mut self,
    hit: Hit,
  ) -> Result<Option<crate::document::Target>> {
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
    self.hit_at(Region::Page, column, row, pixel, Self::text_shown)
  }

  /// Page-area pixels at a cell: reported, else centre.
  pub(crate) fn cell_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> (f32, f32) {
    // Clamped drags report pixels past the cell.
    let inside = |at: Option<u16>, cell: u32, size: u16| {
      at.map_or(f32::from(size) / 2.0, |at| {
        let offset = u32::from(at).saturating_sub(cell * u32::from(size));
        let last = size.saturating_sub(1);
        f32::from(u16::try_from(offset).unwrap_or(last).min(last))
      })
    };
    let (width, height) = (self.screen.cell_width, self.screen.cell_height);
    let (screen_column, screen_row) = (
      u32::from(self.at.column) + u32::from(column),
      u32::from(self.at.row) + u32::from(row),
    );
    let left = f32::from(column.saturating_sub(self.left)) * f32::from(width);
    let x = left + inside(pixel.map(|p| p.x), screen_column, width);
    let top = f32::from(row) * f32::from(height);
    (x, top + inside(pixel.map(|p| p.y), screen_row, height))
  }

  /// Page-area pixels of pane cells.
  pub(crate) fn cell_bounds(&self, cells: crate::split::Rect) -> Bounds {
    let screen = self.layout.screen;
    let (width, height) =
      (f32::from(screen.cell_width), f32::from(screen.cell_height));
    let x0 = f32::from(cells.column.saturating_sub(self.left)) * width;
    let y0 = f32::from(cells.row) * height;
    let x1 = x0 + f32::from(cells.columns) * width;
    Bounds { x0, y0, x1, y1: y0 + f32::from(cells.rows) * height }
  }

  /// Page and point under the pointer in a pane cell.
  pub fn page_point(
    &self,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Option<(PageId, f32, f32)> {
    self.pixel_point(self.cell_point(column, row, pixel))
  }

  /// Page and point at a page-area pixel.
  fn pixel_point(&self, (x, y): (f32, f32)) -> Option<(PageId, f32, f32)> {
    let page = self.layout.page_at(y + self.reader.scroll);
    let (x, y) = self.layout.unproject(page, (x, y), self.reader.scroll)?;
    Some((page, x, y))
  }

  /// A cell's point in `page` coordinates.
  pub(crate) fn point_on(
    &self,
    page: PageId,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Option<(f32, f32)> {
    let at = self.cell_point(column, row, pixel);
    self.layout.unproject(page, at, self.reader.scroll)
  }

  /// What the pointer covers next frame.
  pub fn hovered(
    &mut self,
    region: Region,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
  ) -> Result<Hit> {
    self.hit_at(region, column, row, pixel, Self::text_hit)
  }

  /// What a cell covers; `glyph` reads text.
  fn hit_at(
    &mut self,
    region: Region,
    column: u16,
    row: u16,
    pixel: Option<Pixel>,
    glyph: GlyphAt,
  ) -> Result<Hit> {
    match region {
      Region::Page if self.on_bar(column) => return Ok(Hit::Scrollbar),
      Region::Page if self.hints.is_some() => {
        if let Some(index) = self.items(List::Hints).at(column, row) {
          return Ok(Hit::Item(List::Hints, Some(index)));
        }
      }
      Region::Page => {}
      Region::Panel => {
        let Some(panel) = &self.panel else { return Ok(Hit::Nothing) };
        if let Some(index) = panel.row_at(column, row) {
          return Ok(Hit::Row(index));
        }
        let card = panel.shelf.as_ref().is_some_and(|s| s.covers(column));
        return Ok(if card { Hit::Card(row) } else { Hit::Nothing });
      }
      Region::Overview => {
        let page = self.thumbnail_at(column, row);
        return Ok(page.map_or(Hit::Nothing, Hit::Thumbnail));
      }
      Region::Menu | Region::Actions => {
        let list =
          if region == Region::Menu { List::Menu } else { List::Actions };
        return Ok(Hit::Item(list, self.items(list).at(column, row)));
      }
      Region::Preview => {
        let peek = self.peek.as_ref();
        let follows = peek.is_some_and(crate::preview::Preview::follows);
        return Ok(Hit::Preview { follows });
      }
      Region::Overlay => {
        let list = self.owned_list();
        let list =
          list.filter(|list| matches!(list, List::Clues | List::Completions));
        let index = list.and_then(|list| self.items(list).at(column, row));
        return Ok(list.map_or(Hit::Nothing, |list| Hit::Item(list, index)));
      }
      Region::Status => {
        let item = self.chrome_at(column, row);
        return Ok(item.map_or(Hit::Nothing, |_| Hit::Chrome));
      }
      Region::Edge => return Ok(Hit::Nothing),
    }
    if self.text_mode {
      let Some(position) = glyph(self, column, row)? else {
        return Ok(Hit::Nothing);
      };
      let point = self.glyph_point(position)?;
      let Some((page, x, y)) = point else { return Ok(Hit::Nothing) };
      return Ok(self.marked_hit(page, x, y)?.unwrap_or(Hit::Text));
    }
    let Some((page, x, y)) = self.page_point(column, row, pixel) else {
      return Ok(Hit::Nothing);
    };
    if let Some(hit) = self.marked_hit(page, x, y)? {
      return Ok(hit);
    }
    let text = self.doc.text(page)?;
    Ok(if text.covers(x, y) { Hit::Text } else { Hit::Nothing })
  }

  /// A link, else a note, here.
  fn marked_hit(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<Hit>> {
    if let Some(index) = self.link_at(page, x, y)? {
      return Ok(Some(Hit::Link(page, index)));
    }
    let Some(index) = self.note_at(page, x, y)? else { return Ok(None) };
    let text = !self.note_icon(page, index);
    Ok(Some(Hit::Note { page, index, text }))
  }
}
