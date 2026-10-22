use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use vpdf_model::input::MouseEvent;
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_reader::reader::SelectionKind;
use vpdf_ui::context::{Command, ContextMenu, Item, PageContext, Transition};
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::pointer::{Hit, Route};
use vpdf_ui::region::Operation;

impl App {
  /// Opens the menu at a right press.
  pub(crate) fn page_context(&mut self, route: Route) {
    let (column, row) = (route.mouse.column, route.mouse.row);
    // Menus are advisory; lookups may fail.
    let hit = self.pressed_hit(column, row, route.pixel).unwrap_or_default();
    let link = self.link_of(hit).ok().flatten();
    let near = self.press_position(&route).ok().flatten();
    let glyph = self.glyph_under(&route, near).ok().flatten();
    let figure = self.figure_under(&route).ok().flatten();
    let note = self.note_of(hit);
    let items = vpdf_ui::context::page_items(PageContext {
      reader: &self.reader,
      link,
      near,
      glyph,
      note,
      figure,
    });
    self.open_menu(
      items,
      (column, row),
      (self.left, self.screen.columns),
      false,
    );
  }

  /// Opens the menu at a right press on a panel; a row press selects
  /// the row its items act on.
  pub(crate) fn panel_context(&mut self, mouse: MouseEvent) {
    let (column, row) = (mouse.column, mouse.row);
    let covered = self.covered();
    let Some(panel) = &mut self.panel else { return };
    let pressed = panel.view.row_at(column, row);
    if let Some(index) = pressed {
      panel.view.choose(index);
    }
    let destination =
      pressed.map(|index| &panel.view.rows()[index].destination);
    let items =
      vpdf_ui::context::panel_items(&panel.view, destination, covered);
    // Cards draw over cells right of the rows.
    let columns = self.screen.columns;
    let right = if covered {
      vpdf_ui::panel::recent::Shelf::list_columns(&panel.view, columns)
    } else {
      columns
    };
    self.open_menu(items, (column, row), (0, right), pressed.is_some());
  }

  /// Opens the menu at a right press on the overview; a thumbnail
  /// press selects its page.
  pub(crate) fn overview_context(
    &mut self,
    mouse: MouseEvent,
    page: Option<PageId>,
  ) {
    if let (Some(page), Some(overview)) = (page, self.overlays.overview_mut()) {
      overview.choose(page);
    }
    let items = vpdf_ui::context::overview_items(page);
    let at = (mouse.column, mouse.row);
    self.open_menu(items, at, (self.left, self.screen.columns), false);
  }

  /// Opens a menu at a pane cell, inside `columns`.
  fn open_menu(
    &mut self,
    items: Vec<Item>,
    at: (u16, u16),
    columns: (u16, u16),
    row: bool,
  ) {
    self.disarm();
    let bottom = self.menu_top();
    self.overlays.open_menu(ContextMenu::new(items, at, columns, bottom, row));
  }

  /// Keys while the menu is open.
  pub(crate) fn context_input(&mut self, input: Input) -> Result<()> {
    let Some(menu) = self.overlays.menu_mut() else { return Ok(()) };
    match menu.input(input.action) {
      Transition::Run => return self.context_run(),
      Transition::Close => drop(self.overlays.close_menu()),
      Transition::Command => {
        self.overlays.close_menu();
        return self.action(input);
      }
      Transition::Transfer => {
        self.dismiss_overlay();
        return self.action(input);
      }
      Transition::Stay | Transition::Unhandled => {}
    }
    Ok(())
  }

  /// Whether a press hit the menu.
  pub(crate) fn context_press(&self, mouse: MouseEvent) -> bool {
    let (column, row) = self.pane_cell(mouse);
    let menu = self.overlays.menu();
    menu.is_some_and(|menu| menu.area().contains(column, row))
  }

  /// Right drags pick; their release runs.
  pub(crate) fn menu_drag(&mut self, route: Route) -> Result<()> {
    let Some(menu) = self.overlays.menu_mut() else { return Ok(()) };
    if menu.drag(route.mouse) {
      return self.context_run();
    }
    Ok(())
  }

  /// Runs the selected item; the menu closes.
  pub(crate) fn context_run(&mut self) -> Result<()> {
    let Some(menu) = self.overlays.close_menu() else { return Ok(()) };
    let Some(command) = menu.into_command() else {
      return Ok(());
    };
    let key = Input::plain;
    match command {
      Command::CopyAddress(uri) => self.copy_address(uri),
      Command::Follow(target) => self.follow(&target),
      Command::Preview(target, near) => self.show_preview(target, near),
      Command::CopyText(text, message) => self.copy_text(text, message),
      Command::Split(target) => self.open_split(&target),
      Command::RowSplit => {
        let selected = self.panel.as_ref().map(|panel| panel.view.selected());
        selected.map_or(Ok(()), |index| self.panel_split(index))
      }
      Command::Copy => self.action(key(Action::Yank)),
      Command::CopyImage => {
        let Some((page, bounds)) = self.selection_bounds()? else {
          return Ok(());
        };
        self.select_region(page, bounds)?;
        self.region_operation(Operation::Image)
      }
      Command::SearchSelection => {
        let Some(text) =
          self.reader.search_text(&mut PdfText(&mut self.doc))?
        else {
          return Ok(());
        };
        self.reader.end_selection();
        self.start_search(&text, false, false)
      }
      Command::SelectWord(glyph) => self.select_unit(glyph, 2),
      Command::SelectLine(glyph) => self.select_unit(glyph, 3),
      Command::SearchWord(glyph) => {
        self.click_select(glyph, 1)?;
        self.action(key(Action::WordSearch))
      }
      Command::Figure(page, bounds, operation) => {
        self.select_region(page, bounds)?;
        self.region_operation(operation)
      }
      Command::Note(page, index) => self.show_note(page, index),
      Command::Key(action) => self.action(key(action)),
    }
  }

  /// A note hit and its comment.
  fn note_of(&mut self, hit: Hit) -> Option<(PageId, usize, String)> {
    let Hit::Note { page, index, .. } = hit else { return None };
    let annotations = self.doc.annotations(page).ok()?;
    Some((page, index, annotations.get(index)?.contents.clone()))
  }

  /// Selects like a click run, without drags.
  fn select_unit(&mut self, glyph: Position, clicks: u8) -> Result<()> {
    self.click_select(glyph, clicks)?;
    self.gesture.reset();
    Ok(())
  }

  /// The selection's first-page bounds.
  fn selection_bounds(&mut self) -> Result<Option<(PageId, Bounds)>> {
    let Some(selection) = self.reader.selection else { return Ok(None) };
    let range =
      vpdf_reader::motion::range(selection.anchor, self.reader.cursor);
    let linewise = selection.kind == SelectionKind::Line;
    let page = range.0.page;
    let text = self.doc.text(page)?;
    let rows = vpdf_reader::motion::covered_rows(&text, page, range, linewise);
    Ok(rows.reduce(Bounds::union).map(|bounds| (page, bounds)))
  }

  /// `near`, when the press hits it.
  fn glyph_under(
    &mut self,
    route: &Route,
    near: Option<Position>,
  ) -> Result<Option<Position>> {
    let (column, row) = (route.mouse.column, route.mouse.row);
    if self.text_mode {
      return self.text().shown(column, row);
    }
    let point = self.page_point(column, row, route.pixel);
    let (Some(near), Some((_, x, y))) = (near, point) else { return Ok(None) };
    let text = self.doc.text(near.page)?;
    let hit = text.glyph(near).is_some_and(|g| g.bounds.contains(x, y));
    Ok(hit.then_some(near))
  }

  /// A page-view figure under a press.
  fn figure_under(
    &mut self,
    route: &Route,
  ) -> Result<Option<(PageId, Bounds)>> {
    if self.text_mode {
      return Ok(None);
    }
    let (column, row) = (route.mouse.column, route.mouse.row);
    let Some((page, x, y)) = self.page_point(column, row, route.pixel) else {
      return Ok(None);
    };
    Ok(
      vpdf_ui::figures::at(
        &mut PdfText(&mut self.doc),
        vpdf_ui::page::Point { page, x, y },
      )?
      .map(|figure| (page, figure.bounds)),
    )
  }

  pub(crate) fn context_rect(&self) -> Option<Bounds> {
    self.overlays.menu()?.pixel_area(self.layout.screen(), self.left)
  }

  pub fn scene_context<'a>(
    &self,
    scene: &mut impl vpdf_model::scene::Draw<'a>,
  ) {
    let Some(menu) = self.overlays.menu() else { return };
    menu.scene(scene, self.frame(self.layout.screen()));
  }
}
