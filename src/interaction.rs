use crate::Result;
use crate::app::App;
use crate::keys::{Action, Input};
use crate::layout::Screen;
use crate::motion::Motion;
use crate::panel::Panel;
use crate::pointer::{Region, Route};
use crate::reader::{Location, SelectionKind};
use crate::text::{Bounds, PageId, Position};
use crossterm::event::{MouseButton, MouseEventKind};
use std::time::{Duration, Instant};

impl App {
  /// Rows the completion list or clues cover right above the status row.
  pub fn menu_rows(&self) -> u16 {
    let Some(menu) = self.prompt.as_ref().and_then(|p| p.menu.as_ref()) else {
      return self.clue_rows();
    };
    let limit = crate::grid::limit(self.screen);
    let layout = menu.layout(usize::from(self.screen.columns), limit);
    u16::try_from(layout.visible).unwrap_or(0)
  }

  /// Page area above the menu or clue rows; page graphics stop there.
  pub fn uncovered(&self) -> Screen {
    let mut screen = self.layout.screen;
    screen.rows = screen.rows.saturating_sub(self.menu_rows());
    screen
  }

  pub fn relayout(&mut self) -> Result<()> {
    let anchor = self.view_anchor();
    let size = |screen: Screen| {
      (screen.columns, screen.rows, screen.cell_width, screen.cell_height)
    };
    let before = size(self.layout.screen);
    let page = self.layout.pages[anchor.0.0].clone();
    self.blank = false;
    let columns = self.screen.columns;
    let covered = self.covered();
    let beside = match &self.panel {
      Some(panel) if !covered => Panel::width(panel.kind, columns),
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
      return Ok(());
    }
    self.restore_view(anchor);
    Ok(())
  }

  pub fn navigation_position(&self) -> Position {
    if self.reader.selection.is_some() || self.reader.operator.is_some() {
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
    let page = self.layout.page_at(self.reader.scroll);
    let center = (self.layout.screen.width() / 2.0, 0.0);
    let point = self
      .layout
      .unproject(page, center, self.reader.scroll)
      .unwrap_or((0.0, 0.0));
    (page, point)
  }

  fn restore_view(&mut self, (page, (x, y)): (PageId, (f32, f32))) {
    self.animation = None;
    let rect =
      self.layout.project(page, Bounds { x0: x, x1: x, y0: y, y1: y }, 0.0);
    self.reader.scroll = self.layout.clamp_scroll(rect.y0);
    let width = self.layout.screen.width();
    let max = self.layout.pan_limit(page);
    self.layout.pan = (self.layout.pan + rect.x0 - width / 2.0).clamp(0.0, max);
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
    let crop = self.layout.crop.apply(self.doc.pages[position.page.0].bounds);
    if glyph.bounds.x1 <= crop.x0
      || glyph.bounds.x0 >= crop.x1
      || glyph.bounds.y1 <= crop.y0
      || glyph.bounds.y0 >= crop.y1
    {
      return Ok(false);
    }
    let rect =
      self.layout.project(position.page, glyph.bounds, self.reader.scroll);
    Ok(
      rect.x1 > 0.0
        && rect.x0 < self.layout.screen.width()
        && rect.y1 > 0.0
        && rect.y0 < self.uncovered().height(),
    )
  }

  pub(crate) fn explicit_target(&mut self) -> Result<Option<Position>> {
    if let Some(target) = self.target
      && self.visible_position(target)?
    {
      return Ok(Some(target));
    }
    self.reader.message = "Click text or jump to select a target".into();
    Ok(None)
  }

  fn enter_text(&mut self) -> Result<bool> {
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
    if self.reader.selection.is_some() || self.reader.operator.is_some() {
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
    let pages = if self.text_mode {
      self.text_top.page.0..self.text_top.page.0 + 1
    } else {
      self.layout.visible(self.reader.scroll)
    };
    for page in pages.map(PageId) {
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

  pub fn ensure_cursor(&mut self) -> Result<()> {
    self.animation = None;
    if self.text_mode {
      return self.reveal_text();
    }
    let cursor = self.reader.cursor;
    let text = self.doc.text(cursor.page)?;
    let Some(glyph) = text.glyph(cursor) else {
      self.reader.scroll = self.layout.pages[cursor.page.0].top;
      return Ok(());
    };
    let rect = self.layout.project(cursor.page, glyph.bounds, 0.0);
    let margin = f32::from(self.screen.cell_height);
    if rect.y0 < self.reader.scroll + margin {
      self.reader.scroll = rect.y0 - margin;
    }
    if rect.y1 > self.reader.scroll + self.layout.screen.height() - margin {
      self.reader.scroll = rect.y1 - self.layout.screen.height() + margin;
    }
    let pan = (-self.layout.origin(cursor.page, 0.0).0).max(0.0);
    if rect.x0 < 0.0 {
      self.layout.pan = (pan + rect.x0).max(0.0);
    }
    if rect.x1 > self.layout.screen.width() {
      self.layout.pan = pan + rect.x1 - self.layout.screen.width();
    }
    self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll);
    Ok(())
  }

  fn normal_input(&mut self, input: Input) -> Result<bool> {
    let count = input.count;
    match input.action {
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
      Action::Motion(_) | Action::Object(_) | Action::Swap => {
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
    let normal =
      self.reader.selection.is_none() && self.reader.operator.is_none();
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
      Action::Visual(kind) => self.reader.visual(kind),
      Action::Object(around) => self.reader.object(around, &mut self.doc)?,
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
      Action::Yank | Action::YankLine | Action::Visual(_)
    ) || (self.reader.selection.is_none()
      && !normal
      && matches!(input.action, Action::Motion(_) | Action::Object(_)))
    {
      return Ok(true);
    }
    if self.reader.cursor.page != previous_page {
      self.reader.scroll = self.layout.pages[self.reader.cursor.page.0].top;
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
        if self.reader.selection.is_none() {
          self.reader.cursor = self.reading_position()?;
        }
        self.align_text(fraction)?;
      }
      Action::Align(fraction) => {
        self.animation = None;
        if self.reader.selection.is_none() {
          self.reader.cursor = self.reading_position()?;
        }
        let text = self.doc.text(self.reader.cursor.page)?;
        if let Some(glyph) = text.glyph(self.reader.cursor) {
          let rect =
            self.layout.project(self.reader.cursor.page, glyph.bounds, 0.0);
          self.reader.scroll = self.layout.clamp_scroll(
            rect.y0 - fraction * (self.layout.screen.height() - rect.height()),
          );
        }
      }
      Action::Zoom(direction) => {
        self.layout.zoom = (self.layout.zoom
          * 1.15_f32.powf(direction as f32 * count))
        .clamp(0.1, 8.0);
        self.relayout()?;
      }
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

  fn pan(&mut self, delta: f32) -> Result<()> {
    if self.text_mode {
      let columns = (delta / f32::from(self.screen.cell_width)).round();
      return self.pan_text(columns as isize);
    }
    let page = self.navigation_position().page;
    let max = self.layout.pan_limit(page);
    self.layout.pan = (self.layout.pan + delta).clamp(0.0, max);
    Ok(())
  }

  /// Cancels pending keys, as `Esc` does.
  pub fn disarm(&mut self) {
    self.keys.clear();
    self.reader.disarm();
    self.after_key();
  }

  /// Closes key-driven state as focus leaves.
  pub fn blur(&mut self) -> Result<()> {
    // Panes share one preview image; only the focused pane shows one.
    self.peek = None;
    self.hints = None;
    self.cancel_prompt()?;
    self.disarm();
    Ok(())
  }

  /// Acts on a routed pointer event.
  pub fn mouse(&mut self, route: Route) -> Result<()> {
    let Route { mouse, region, .. } = route;
    self.hints = None;
    self.peek = None;
    match region {
      Region::Panel => return self.panel_mouse(mouse),
      Region::Overlay | Region::Status => return Ok(()),
      Region::Page | Region::Overview => {}
    }
    if matches!(mouse.kind, MouseEventKind::Down(_))
      && let Some(panel) = &mut self.panel
    {
      panel.focused = false;
    }
    if region == Region::Overview {
      return self.overview_mouse(mouse);
    }
    // Drags from before `gp` stay hidden.
    if self.overview.is_some() {
      return Ok(());
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        return self.scroll(3.0 * f32::from(self.screen.cell_height));
      }
      MouseEventKind::ScrollUp => {
        return self.scroll(-3.0 * f32::from(self.screen.cell_height));
      }
      MouseEventKind::Down(MouseButton::Left)
      | MouseEventKind::Drag(MouseButton::Left) => {}
      _ => return Ok(()),
    }
    self.animation = None;
    let pos = if self.text_mode {
      self.text_position(mouse.column, mouse.row)?
    } else {
      let y = f32::from(mouse.row) * f32::from(self.screen.cell_height);
      let page = self.layout.page_at(y + self.reader.scroll);
      let x = f32::from(mouse.column.saturating_sub(self.left))
        * f32::from(self.screen.cell_width);
      let Some((x, y)) =
        self.layout.unproject(page, (x, y), self.reader.scroll)
      else {
        return Ok(());
      };
      self.doc.text(page)?.nearest(page, x, y)
    };
    if matches!(mouse.kind, MouseEventKind::Drag(_)) {
      if self.reader.selection.is_none() {
        self.reader.visual(SelectionKind::Character);
      }
      self.reader.cursor = pos;
    } else {
      self.reader.end_selection();
      self.reader.cursor = pos;
      self.target = Some(pos);
      if self.last_click.is_some_and(|(when, old)| {
        old == pos && when.elapsed() < Duration::from_millis(350)
      }) {
        self.reader.object(false, &mut self.doc)?;
      }
      self.last_click = Some((Instant::now(), pos));
    }
    Ok(())
  }
}
