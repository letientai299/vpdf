//! Right-click context menu.

use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::keys::{self, Action, Input, PanelKind};
use crate::panel::Destination;
use crate::pdf_text::PdfText;
use crate::pointer::{Hit, Items, List, Route};
use crate::region::Operation;
use crate::split::Rect;
use crate::text::{Bounds, PageId, Position};
use crate::theme;
use crossterm::event::{MouseEvent, MouseEventKind};
use std::io::Write;
use unicode_width::UnicodeWidthStr;
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::SelectionKind;

/// An open context menu.
pub struct ContextMenu {
  pub items: Vec<Item>,
  pub selected: usize,
  /// Top-left pane cell.
  pub at: (u16, u16),
  /// Columns, kept inside the pane.
  pub width: u16,
  /// The opening right press's cell.
  pub press: (u16, u16),
  /// That drag went `ARM` cells away.
  pub armed: bool,
  /// Its items act on the panel's selected row.
  pub row: bool,
}

/// A menu row.
pub struct Item {
  pub label: &'static str,
  /// Its key in help notation; empty when none.
  pub key: String,
  pub command: Command,
}

#[derive(Clone, Debug)]
pub enum Command {
  Follow(Target),
  /// Previews a target beside a glyph.
  Preview(Target, Position),
  /// Copies text, saying `message`.
  CopyText(String, &'static str),
  /// As a middle click does.
  Split(Target),
  /// The selected panel row in a split, as a middle click does.
  RowSplit,
  Copy,
  CopyImage,
  SearchSelection,
  SelectWord(Position),
  SelectLine(Position),
  SearchWord(Position),
  /// Copies or saves a figure.
  Figure(PageId, Bounds, Operation),
  /// Shows a note's popup.
  Note(PageId, usize),
  /// Runs a key action.
  Key(Action),
}

/// An item showing `action`'s key.
fn bound(label: &'static str, action: Action, command: Command) -> Item {
  let key = keys::key_for(action).map_or_default(keys::notation);
  Item { label, key, command }
}

fn item(label: &'static str, key: &str, command: Command) -> Item {
  Item { label, key: key.into(), command }
}

/// Copies a web address, as a middle click does.
fn copy_address(uri: String) -> Item {
  let copy = Command::CopyText(uri, crate::commands::COPIED_LINK);
  item("Copy address", "", copy)
}

/// A figure's items; `gF` opens the bar for the selected figure and
/// copies nothing, so they show no key.
fn figure_items(page: PageId, bounds: Bounds) -> [Item; 2] {
  [("Copy image", Operation::Image), ("Save image", Operation::Save)].map(
    |(label, operation)| {
      item(label, "", Command::Figure(page, bounds, operation))
    },
  )
}

/// Back and Forward.
fn history() -> [Item; 2] {
  [("Back", false), ("Forward", true)].map(|(label, forward)| {
    let action = Action::History(forward);
    bound(label, action, Command::Key(action))
  })
}

/// Right drags pick past this distance.
const ARM: u16 = 2;

impl ContextMenu {
  /// A menu opened at a pane cell.
  fn new(items: Vec<Item>, at: (u16, u16)) -> Self {
    let (armed, row) = (false, false);
    Self { items, selected: 0, at, width: 0, press: at, armed, row }
  }

  /// Widest key, for the key column.
  fn keys(&self) -> usize {
    self.items.iter().map(|item| item.key.width()).max().unwrap_or(0)
  }

  /// Columns the rows need.
  fn needed(&self) -> usize {
    let label = self.items.iter().map(|item| item.label.width()).max();
    self.keys() + 2 + label.unwrap_or(0) + 2
  }

  /// Pane cells the rows cover.
  pub fn area(&self) -> Rect {
    let (column, row) = self.at;
    let rows = u16::try_from(self.items.len()).unwrap_or(u16::MAX);
    Rect { column, row, columns: self.width, rows }
  }

  /// Each row's cells.
  pub(crate) fn items(&self) -> Items {
    let mut items = Items::default();
    let area = self.area();
    for (index, row) in (area.row..area.row + area.rows).enumerate() {
      items.push(Rect { row, rows: 1, ..area }, index);
    }
    items
  }
}

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
    let mut items = Vec::new();
    match (link, near) {
      (Some(Target::Uri(uri)), _) => {
        let follow = Command::Follow(Target::Uri(uri.clone()));
        items.push(bound("Follow", Action::OpenUrl, follow));
        items.push(copy_address(uri));
      }
      (Some(target), near) => {
        let follow = Command::Follow(target.clone());
        items.push(bound("Follow", Action::Follow, follow));
        if let Some(near) = near {
          let preview = Command::Preview(target.clone(), near);
          items.push(bound("Preview", Action::Peek, preview));
        }
        items.push(item("Open in split", "", Command::Split(target)));
      }
      (None, _) => {}
    }
    if let Some((page, index, contents)) = self.note_of(hit) {
      items.push(bound("Show note", Action::Peek, Command::Note(page, index)));
      let copy = Command::CopyText(contents, "Copied comment");
      items.push(item("Copy comment", "", copy));
    }
    if let Some(selection) = self.reader.selection {
      items.push(bound("Copy", Action::Yank, Command::Copy));
      // Region captures hold one page.
      if selection.anchor.page == self.reader.cursor.page {
        items.push(item("Copy as image", "", Command::CopyImage));
      }
      items.push(item("Search for selection", "", Command::SearchSelection));
    } else if let Some(glyph) = glyph {
      items.push(item("Select word", "viw", Command::SelectWord(glyph)));
      let lines = Action::Visual(SelectionKind::Line);
      items.push(bound("Select line", lines, Command::SelectLine(glyph)));
      let search = Command::SearchWord(glyph);
      items.push(bound("Search word", Action::WordSearch, search));
    }
    if let Some((page, bounds)) = figure {
      items.extend(figure_items(page, bounds));
    }
    items.extend(history());
    items.extend(
      [
        ("Fit width", Action::Fit),
        ("Fit page", Action::FitPage),
        ("Outline", Action::Panel(PanelKind::Outline)),
        ("Overview", Action::Overview),
      ]
      .map(|(label, action)| bound(label, action, Command::Key(action))),
    );
    self.open_menu(items, (column, row), (self.left, self.screen.columns));
  }

  /// Opens the menu at a right press on a panel; a row press selects
  /// the row its items act on.
  pub(crate) fn panel_context(&mut self, mouse: MouseEvent) {
    let (column, row) = (mouse.column, mouse.row);
    let covered = self.covered();
    let Some(panel) = &mut self.panel else { return };
    let mut items = Vec::new();
    let pressed = panel.row_at(column, row);
    if let Some(index) = pressed {
      panel.selected = index;
      let destination = &panel.rows[index].destination;
      let address = match destination {
        Destination::Link(Target::Uri(uri)) => Some(uri.clone()),
        _ => None,
      };
      if !matches!(destination, Destination::None) {
        let label = if covered { "Open" } else { "Follow" };
        let action =
          if address.is_some() { Action::OpenUrl } else { Action::Follow };
        items.push(bound(label, action, Command::Key(action)));
        if !panel.is_recent_view() {
          let keep = Command::Key(Action::Swap);
          items.push(bound("Follow, keep panel", Action::Swap, keep));
        }
      }
      // Addresses split by copying; the copy item does that.
      if !matches!(destination, Destination::None) && address.is_none() {
        items.push(item("Open in split", "", Command::RowSplit));
      }
      if panel.kind == PanelKind::Annotations {
        items.push(bound(
          "Show note",
          Action::Peek,
          Command::Key(Action::Peek),
        ));
      }
      if let Some(uri) = address {
        items.push(copy_address(uri));
      } else if let Some(text) = panel.yank_text() {
        let copy = Command::CopyText(text, "Copied row");
        items.push(item("Copy row", "yy", copy));
      }
    }
    let close = Command::Key(Action::ClosePanel);
    items.push(bound("Close panel", Action::Quit, close));
    // Cards draw over cells right of the rows.
    let columns = self.screen.columns;
    let right = if covered {
      crate::recent::Shelf::list_columns(panel, columns)
    } else {
      columns
    };
    self.open_menu(items, (column, row), (0, right));
    if let Some(menu) = &mut self.context_menu {
      menu.row = pressed.is_some();
    }
  }

  /// Opens the menu at a right press on the overview; a thumbnail
  /// press selects its page.
  pub(crate) fn overview_context(
    &mut self,
    mouse: MouseEvent,
    page: Option<PageId>,
  ) {
    let mut items = Vec::new();
    if let Some(page) = page {
      if let Some(overview) = &mut self.overview {
        overview.selected = page.0;
      }
      items.push(bound("Open", Action::Follow, Command::Key(Action::Follow)));
      let split = Command::Split(Target::Page(page));
      items.push(item("Open in split", "", split));
    }
    items.extend(history());
    let close = Command::Key(Action::Overview);
    items.push(bound("Close overview", Action::Quit, close));
    let at = (mouse.column, mouse.row);
    self.open_menu(items, at, (self.left, self.screen.columns));
  }

  /// Opens a menu at a pane cell, inside `columns`.
  fn open_menu(
    &mut self,
    items: Vec<Item>,
    at: (u16, u16),
    columns: (u16, u16),
  ) {
    self.disarm();
    let mut menu = ContextMenu::new(items, at);
    self.context_fit(&mut menu, columns);
    self.context_menu = Some(menu);
  }

  /// Keeps the menu above the status rows and between `left` and
  /// `right`.
  fn context_fit(&self, menu: &mut ContextMenu, (left, right): (u16, u16)) {
    let bottom = self.screen.rows.saturating_sub(1 + self.menu_rows());
    // Short panes drop common items first.
    menu.items.truncate(usize::from(bottom));
    let rows = u16::try_from(menu.items.len()).unwrap_or(0);
    let room = right.saturating_sub(left);
    let width = u16::try_from(menu.needed()).unwrap_or(u16::MAX).min(room);
    let (column, row) = menu.at;
    let last = right.saturating_sub(width);
    let row = if row + rows <= bottom { row } else { row.saturating_sub(rows) };
    menu.at = (column.min(last).max(left), row.min(bottom - rows));
    menu.width = width;
  }

  /// Keys while the menu is open.
  pub(crate) fn context_input(&mut self, input: Input) -> Result<()> {
    let Some(menu) = &mut self.context_menu else { return Ok(()) };
    let count = menu.items.len().max(1);
    match input.action {
      Action::Motion(Motion::Down) => {
        menu.selected = (menu.selected + 1) % count;
      }
      Action::Motion(Motion::Up) => {
        menu.selected = (menu.selected + count - 1) % count;
      }
      Action::Follow => return self.context_run(),
      Action::Quit | Action::Cancel => self.context_menu = None,
      Action::Command => {
        self.context_menu = None;
        return self.action(input);
      }
      action if self.overlay_transfer(action) => return self.action(input),
      _ => {}
    }
    Ok(())
  }

  /// Whether a press hit the menu.
  pub(crate) fn context_press(&self, mouse: MouseEvent) -> bool {
    let (column, row) = self.pane_cell(mouse);
    let menu = self.context_menu.as_ref();
    menu.is_some_and(|menu| menu.area().contains(column, row))
  }

  /// Right drags pick; their release runs.
  pub(crate) fn menu_drag(&mut self, route: Route) -> Result<()> {
    let Some(menu) = &mut self.context_menu else { return Ok(()) };
    let (column, row) = (route.mouse.column, route.mouse.row);
    // Slips stay near the press.
    let far = column.abs_diff(menu.press.0).max(row.abs_diff(menu.press.1));
    menu.armed |= far >= ARM;
    let item = menu.items().at(column, row).filter(|_| menu.armed);
    let Some(index) = item else { return Ok(()) };
    self.pick_item(List::Menu, index);
    if matches!(route.mouse.kind, MouseEventKind::Up(_)) {
      return self.context_run();
    }
    Ok(())
  }

  /// Runs the selected item; the menu closes.
  pub(crate) fn context_run(&mut self) -> Result<()> {
    let Some(menu) = self.context_menu.take() else { return Ok(()) };
    let Some(item) = menu.items.into_iter().nth(menu.selected) else {
      return Ok(());
    };
    let key = Input::plain;
    match item.command {
      Command::Follow(target) => self.follow(&target),
      Command::Preview(target, near) => self.show_preview(target, near),
      Command::CopyText(text, message) => self.copy_text(text, message),
      Command::Split(target) => self.open_split(&target),
      Command::RowSplit => {
        let selected = self.panel.as_ref().map(|panel| panel.selected);
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
    self.gesture = crate::gestures::Gesture::Glyphs;
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
    let rows = crate::canvas::covered_rows(&text, page, range, linewise);
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
      return self.text_shown(column, row);
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
    Ok(self.figure_at(page, x, y)?.map(|figure| (page, figure.bounds)))
  }

  /// The menu's page-area pixels; panel cells hold no picture.
  pub(crate) fn context_rect(&self) -> Option<Bounds> {
    let area = self.context_menu.as_ref()?.area();
    let column = area.column.max(self.left);
    let columns = (area.column + area.columns).checked_sub(column);
    let columns = columns.filter(|columns| *columns > 0)?;
    Some(self.cell_bounds(Rect { column, columns, ..area }))
  }

  /// Draws the menu, key first, like clues.
  pub fn draw_context(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(menu) = &self.context_menu else { return Ok(()) };
    let (left, top) = menu.at;
    let keys = menu.keys();
    let width = usize::from(menu.width);
    let selected = menu.selected;
    for (index, item) in menu.items.iter().enumerate() {
      let row = top + u16::try_from(index).unwrap_or(0);
      let label = format!("{:<keys$}  {}", item.key, item.label);
      write!(out, "{}{}", self.at.goto(row, left), theme::BAR)?;
      let shown = if index == selected {
        crate::grid::Shown::Chosen
      } else {
        crate::grid::Shown::Plain
      };
      crate::grid::cell(out, &label, width, keys, shown)?;
      write!(out, "{}", theme::RESET)?;
    }
    let Some(area) = self.context_rect().filter(|_| !self.text_mode) else {
      return Ok(());
    };
    let screen = self.layout.screen;
    let height = f32::from(screen.cell_height);
    let rows = ((area.y1 - area.y0) / height).round() as usize;
    for row in 0..rows {
      let color =
        if row == selected { theme::BADGE_FILL } else { theme::BAR_FILL };
      let y0 = area.y0 + row as f32 * height;
      let rect = Bounds { y0, y1: y0 + height, ..area };
      self.graphics.backdrop(rect, color, screen, self.left, out)?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn menu(at: (u16, u16)) -> ContextMenu {
    let items = vec![
      bound(
        "Back",
        Action::History(false),
        Command::Key(Action::History(false)),
      ),
      bound("Fit width", Action::Fit, Command::Key(Action::Fit)),
    ];
    let mut menu = ContextMenu::new(items, at);
    menu.width = u16::try_from(menu.needed()).unwrap_or(0);
    menu
  }

  #[test]
  fn items_hit_by_cell() {
    let menu = menu((10, 5));
    assert_eq!(menu.width, 18);
    // (column, row) -> item
    let cases = [
      ((10, 5), Some(0)),
      ((27, 6), Some(1)),
      ((28, 6), None),
      ((9, 5), None),
      ((10, 7), None),
      ((10, 4), None),
    ];
    for ((column, row), want) in cases {
      assert_eq!(menu.items().at(column, row), want, "{column},{row}");
    }
  }

  /// U19: figure items show no key; `gF` only opens the bar.
  #[test]
  fn figure_items_show_no_key() {
    let items = figure_items(PageId(0), Bounds::default());
    let shown: Vec<_> =
      items.iter().map(|i| (i.label, i.key.as_str())).collect();
    assert_eq!(shown, [("Copy image", ""), ("Save image", "")]);
  }

  /// Shown keys name their actions, in help notation.
  #[test]
  fn keys_match_bindings() {
    use vpdf_reader::reader::SelectionKind::Line;
    // (shown, binding, action)
    let cases = [
      ("<CR>", "Enter", Action::Follow),
      ("K", "K", Action::Peek),
      ("y", "y", Action::Yank),
      ("V", "V", Action::Visual(Line)),
      ("*", "*", Action::WordSearch),
      ("<C-o>", "C-o", Action::History(false)),
      ("<C-i>", "C-i", Action::History(true)),
      ("=", "=", Action::Fit),
      ("zf", "zf", Action::FitPage),
      ("go", "go", Action::Panel(PanelKind::Outline)),
      ("gp", "gp", Action::Overview),
      ("o", "o", Action::Swap),
      ("q", "q", Action::Quit),
    ];
    for (shown, binding, action) in cases {
      assert_eq!(keys::binding(binding), Some(action), "{binding}");
      assert_eq!(bound("", action, Command::Key(action)).key, shown);
    }
  }
}
