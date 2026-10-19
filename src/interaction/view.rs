use super::{App, Items, Location, PageId, Panel, Position, Result, Screen};
use crate::pdf_text::PdfText;

impl App {
  pub(crate) fn reading_view(
    &mut self,
  ) -> vpdf_ui::reading::View<'_, PdfText<'_>> {
    let frame = vpdf_ui::reading::Frame {
      pane: vpdf_ui::paint::Frame {
        screen: self.screen,
        at: self.at,
        left: self.left,
        text: self.text_mode,
      },
      visible: self.uncovered(),
    };
    vpdf_ui::reading::View::new(
      &mut self.reader,
      &mut self.target,
      &mut self.animation,
      &mut self.layout,
      &mut self.text_view,
      frame,
      PdfText(&mut self.doc),
    )
  }

  pub(crate) fn page_view(
    &mut self,
  ) -> vpdf_ui::page::Viewport<'_, PdfText<'_>> {
    let screen = self.uncovered();
    vpdf_ui::page::Viewport::new(
      &mut self.layout,
      &mut self.reader.scroll,
      PdfText(&mut self.doc),
      screen,
    )
  }

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
    let mut screen = self.layout.screen();
    screen.rows = screen.rows.saturating_sub(self.menu_rows());
    screen
  }

  pub fn relayout(&mut self) -> Result<()> {
    let landed = self
      .target
      .filter(|&target| self.landing.matches(self.landed_view(target)));
    self.reflow_view()?;
    // The reflowed view keeps the landing.
    if let Some(target) = landed {
      self.landing.record(self.landed_view(target));
    }
    Ok(())
  }

  fn reflow_view(&mut self) -> Result<()> {
    let anchor = self.view_anchor();
    let scrolled = self.reader.scroll;
    let before = self.layout.checkpoint(anchor.0);
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
    let screen =
      Screen { columns: columns.saturating_sub(beside), ..self.screen };
    self.layout.resize(screen, self.doc.pages.iter().map(|page| page.bounds));
    if self.text_mode {
      self.text().reclamp()?;
    }
    if before == self.layout.checkpoint(anchor.0) {
      self.reader.scroll = self.layout.clamp_scroll(self.reader.scroll);
      self.layout.clamp_pan(anchor.0);
    } else {
      self.restore_view(anchor);
    }
    // Only user moves show the bar.
    self.bar.reflow(scrolled, self.reader.scroll);
    Ok(())
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
    Ok(Location {
      cursor: self.reading_position()?,
      scroll: self.reader.scroll,
      anchor: Some(self.view_anchor()),
      text_top: self.text_mode.then_some(self.text_view.top()),
      target: self.target,
    })
  }

  pub(crate) fn restore_location(&mut self, location: Location) -> Result<()> {
    self.animation.cancel();
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
      self.text().restore(top)?;
    }
    Ok(())
  }

  fn view_anchor(&self) -> (PageId, (f32, f32)) {
    self.anchor_at((self.layout.screen().width() / 2.0, 0.0))
  }

  /// Page point at a page-area pixel, for a view to return to.
  pub(super) fn anchor_at(&self, at: (f32, f32)) -> (PageId, (f32, f32)) {
    self.layout.anchor_at(at, self.reader.scroll)
  }

  fn restore_view(&mut self, anchor: (PageId, (f32, f32))) {
    self.restore_at(anchor, (self.layout.screen().width() / 2.0, 0.0));
  }

  /// Scrolls and pans so `anchor` shows at a page-area pixel.
  pub(super) fn restore_at(
    &mut self,
    (page, (x, y)): (PageId, (f32, f32)),
    (left, top): (f32, f32),
  ) {
    self.animation.cancel();
    self.reader.scroll = self.layout.restore_at(page, (x, y), (left, top));
  }

  pub(crate) fn visible_position(
    &mut self,
    position: Position,
  ) -> Result<bool> {
    self.reading_view().visible(position)
  }

  pub(crate) fn explicit_target(&mut self) -> Result<Option<Position>> {
    self.reading_view().explicit_target()
  }

  pub(crate) fn enter_text(&mut self) -> Result<bool> {
    self.reading_view().enter_text()
  }

  pub fn reading_position(&mut self) -> Result<Position> {
    self.reading_view().position()
  }

  pub(crate) fn first_visible(&mut self) -> Result<Option<Position>> {
    self.reading_view().first()
  }

  /// Shows `page` from its top; `ensure_cursor` then moves least.
  pub(crate) fn show_page(&mut self, page: PageId) -> Result<()> {
    self.reader.scroll = self.layout.pages()[page.0].top;
    if self.text_mode {
      let top = Position { page, ..Position::default() };
      self.text().restore(top)?;
    }
    Ok(())
  }

  /// `show_page` unless text draws `position`'s line.
  pub(crate) fn show_page_of(&mut self, position: Position) -> Result<()> {
    if self.text_mode && self.text().row(position)?.is_some() {
      return Ok(());
    }
    self.show_page(position.page)
  }

  pub fn ensure_cursor(&mut self) -> Result<()> {
    self.reading_view().reveal()
  }

  /// Shows the cursor; scrolling keeps context above.
  pub(crate) fn land(&mut self) -> Result<()> {
    self.reading_view().land()
  }
}
