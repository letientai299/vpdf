//! Right-click context menu.

use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::keys::{self, Action, Input, PanelKind};
use crate::motion::Motion;
use crate::pointer::{Hit, Items, List, Route};
use crate::reader::SelectionKind;
use crate::region::Operation;
use crate::split::Rect;
use crate::text::{Bounds, PageId, Position};
use crate::theme;
use crossterm::event::{MouseEvent, MouseEventKind};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

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
}

/// A menu row.
pub struct Item {
  pub label: &'static str,
  /// Its key; empty when none.
  pub key: &'static str,
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
  Item { label, key: keys::key_for(action).unwrap_or(""), command }
}

fn item(label: &'static str, key: &'static str, command: Command) -> Item {
  Item { label, key, command }
}

/// Right drags pick past this distance.
const ARM: u16 = 2;

impl ContextMenu {
  /// A menu opened at a pane cell.
  fn new(items: Vec<Item>, at: (u16, u16)) -> Self {
    Self { items, selected: 0, at, width: 0, press: at, armed: false }
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
        items.push(bound("Follow", Action::Follow, follow));
        let copy = Command::CopyText(uri, crate::commands::COPIED_LINK);
        items.push(item("Copy address", "", copy));
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
      let copy = Command::Figure(page, bounds, Operation::Image);
      items.push(bound("Copy image", Action::Region, copy));
      let save = Command::Figure(page, bounds, Operation::Save);
      items.push(item("Save image", "", save));
    }
    items.extend(
      [
        ("Back", Action::History(false)),
        ("Forward", Action::History(true)),
        ("Fit width", Action::Fit),
        ("Fit page", Action::FitPage),
        ("Outline", Action::Panel(PanelKind::Outline)),
        ("Overview", Action::Overview),
      ]
      .map(|(label, action)| bound(label, action, Command::Key(action))),
    );
    self.disarm();
    let mut menu = ContextMenu::new(items, (column, row));
    self.context_fit(&mut menu);
    self.context_menu = Some(menu);
  }

  /// Keeps the menu inside the page area.
  fn context_fit(&self, menu: &mut ContextMenu) {
    let bottom = self.screen.rows.saturating_sub(1 + self.menu_rows());
    // Short panes drop common items first.
    menu.items.truncate(usize::from(bottom));
    let rows = u16::try_from(menu.items.len()).unwrap_or(0);
    let room = self.screen.columns.saturating_sub(self.left);
    let width = u16::try_from(menu.needed()).unwrap_or(u16::MAX).min(room);
    let (column, row) = menu.at;
    let right = self.screen.columns.saturating_sub(width);
    let row = if row + rows <= bottom { row } else { row.saturating_sub(rows) };
    menu.at = (column.min(right).max(self.left), row.min(bottom - rows));
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
      Command::Copy => self.action(key(Action::Yank)),
      Command::CopyImage => {
        let Some((page, bounds)) = self.selection_bounds()? else {
          return Ok(());
        };
        self.select_region(page, bounds)?;
        self.region_operation(Operation::Image)
      }
      Command::SearchSelection => {
        let Some(text) = self.reader.search_text(&mut self.doc)? else {
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
    let range = crate::motion::range(selection.anchor, self.reader.cursor);
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

  /// The menu's page-area pixels.
  pub(crate) fn context_rect(&self) -> Option<Bounds> {
    let menu = self.context_menu.as_ref()?;
    Some(self.cell_bounds(menu.area()))
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
      crate::grid::cell(out, &label, width, keys, index == selected)?;
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
    assert_eq!(menu.width, 16);
    // (column, row) -> item
    let cases = [
      ((10, 5), Some(0)),
      ((25, 6), Some(1)),
      ((26, 6), None),
      ((9, 5), None),
      ((10, 7), None),
      ((10, 4), None),
    ];
    for ((column, row), want) in cases {
      assert_eq!(menu.items().at(column, row), want, "{column},{row}");
    }
  }

  /// Shown keys name their actions.
  #[test]
  fn keys_match_bindings() {
    use crate::reader::SelectionKind::{Character, Line};
    // (key, action)
    let cases = [
      ("Enter", Action::Follow),
      ("K", Action::Peek),
      ("y", Action::Yank),
      ("V", Action::Visual(Line)),
      ("v", Action::Visual(Character)),
      ("*", Action::WordSearch),
      ("gF", Action::Region),
      ("C-o", Action::History(false)),
      ("C-i", Action::History(true)),
      ("=", Action::Fit),
      ("zf", Action::FitPage),
      ("go", Action::Panel(PanelKind::Outline)),
      ("gp", Action::Overview),
    ];
    for (key, action) in cases {
      assert_eq!(keys::binding(key), Some(action), "{key}");
      if key != "v" {
        assert_eq!(keys::key_for(action), Some(key), "{action:?}");
      }
    }
  }
}
