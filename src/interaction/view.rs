use super::{App, Items, Location, PageId, Panel, Position, Result, Screen};
use crate::pdf_text::PdfText;

pub(crate) struct Reading<'a> {
  pub view: vpdf_ui::reading::View<'a, PdfText<'a>>,
  pub search: &'a mut vpdf_ui::find::Search,
  pub bar: &'a mut vpdf_ui::scrollbar::Bar,
  pub gesture: &'a mut vpdf_ui::input::gesture::Gesture,
  pub outline: &'a mut vpdf_ui::reading::Outline,
  pub panel: Option<&'a mut Panel>,
}

impl App {
  pub fn reading_view(&mut self) -> vpdf_ui::reading::View<'_, PdfText<'_>> {
    self.reading_parts().view
  }

  pub(crate) fn reading_parts(&mut self) -> Reading<'_> {
    let frame = vpdf_ui::reading::Frame {
      pane: vpdf_ui::paint::Frame {
        screen: self.screen,
        at: self.at,
        left: self.left,
        text: self.text_mode,
      },
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
      outline: &mut self.outline,
      panel: self.panel.as_mut().map(|panel| &mut panel.view),
    }
  }

  pub(crate) fn page_view(
    &mut self,
  ) -> vpdf_ui::page::Viewport<'_, PdfText<'_>> {
    let screen = self.uncovered();
    self.reader.page(&mut self.layout, PdfText(&mut self.doc), screen)
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

  pub(crate) fn visible_position(
    &mut self,
    position: Position,
  ) -> Result<bool> {
    self.reading_view().visible(position)
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
