use super::{
  App, Bounds, Items, Location, PageId, Panel, Position, Result, Screen,
};
use crate::pdf_text::PdfText;

impl App {
  /// Rows the completion list or clues cover right above the status row.
  pub fn menu_rows(&self) -> u16 {
    let Some(menu) = self.prompt.as_ref().and_then(|p| p.menu()) else {
      return self.clue_rows();
    };
    u16::try_from(menu.grid(self.screen).visible).unwrap_or(0)
  }

  /// Completions, by cells.
  pub(crate) fn completion_cells(&self) -> Items {
    let mut items = Items::default();
    let menu = self.prompt.as_ref().and_then(|prompt| prompt.menu());
    let Some(menu) = menu else { return items };
    let grid = menu.grid(self.screen);
    for (cells, index) in grid.cells(menu.items().len(), self.menu_top()) {
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
        Panel::width(panel.view.kind(), columns, self.panel_width)
      }
      _ => 0,
    };
    self.left = if covered { columns } else { beside };
    self.layout.screen = self.screen;
    self.layout.screen.columns = columns.saturating_sub(beside);
    self.layout.reflow(self.doc.pages.iter().map(|page| page.bounds));
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
    self.reader.cursor =
      vpdf_reader::reader::clamp(location.cursor, &mut PdfText(&mut self.doc))?;
    self.reader.desired = None;
    self.target = location.target;
    // A view from the other presentation is stale; only its position
    // lands, judged against the current view.
    if location.text_top.is_some() != self.text_mode {
      return self.land();
    }
    match location.anchor {
      Some(anchor) => self.restore_view(anchor),
      None => self.reader.scroll = self.layout.clamp_scroll(location.scroll),
    }
    if let Some(top) = location.text_top {
      self.text_top = self.clamp_text(top)?;
    }
    Ok(())
  }

  fn view_anchor(&self) -> (PageId, (f32, f32)) {
    self.anchor_at((self.layout.screen.width() / 2.0, 0.0))
  }

  /// Page point at a page-area pixel, for a view to return to.
  pub(super) fn anchor_at(&self, at: (f32, f32)) -> (PageId, (f32, f32)) {
    self.pixel_point(at).map_or_else(
      || (self.layout.page_at(at.1 + self.reader.scroll), (0.0, 0.0)),
      |(page, x, y)| (page, (x, y)),
    )
  }

  fn restore_view(&mut self, anchor: (PageId, (f32, f32))) {
    self.restore_at(anchor, (self.layout.screen.width() / 2.0, 0.0));
  }

  /// Scrolls and pans so `anchor` shows at a page-area pixel.
  pub(super) fn restore_at(
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
    let crop = self.layout.crop_for(page).apply(self.doc.pages[page.0].bounds);
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
  pub(super) fn glyph_rect(
    &mut self,
    position: Position,
  ) -> Result<Option<Bounds>> {
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
}
