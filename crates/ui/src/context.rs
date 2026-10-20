use crate::Result;
use crate::keys::{self, Action, PanelKind};
use crate::paint::Frame;
use crate::panel::{Destination, Panel};
use crate::pointer::Items;
use crate::region::Operation;
use crate::split::Rect;
use crate::theme;
use crossterm::event::{MouseEvent, MouseEventKind};
use std::io::Write;
use unicode_width::UnicodeWidthStr;
use vpdf_model::document::Target;
use vpdf_model::geometry::Screen;
use vpdf_model::style::Role;
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::{Reader, SelectionKind};
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::style::sgr;

#[derive(Debug, PartialEq, Eq)]
pub enum Transition {
  Stay,
  Run,
  Close,
  Command,
  Transfer,
  Unhandled,
}

enum Route {
  Move(bool),
  Run,
  Close,
  Command,
  Transfer,
  Ignore,
}

fn route(action: Action) -> Route {
  match action {
    Action::Motion(Motion::Down) => Route::Move(true),
    Action::Motion(Motion::Up) => Route::Move(false),
    Action::Follow => Route::Run,
    Action::Quit | Action::Cancel => Route::Close,
    Action::Command => Route::Command,
    _ if crate::input::transfers_overlay(action) => Route::Transfer,
    _ => Route::Ignore,
  }
}

/// An open context menu.
pub struct ContextMenu<C = Command> {
  items: Vec<Item<C>>,
  selected: usize,
  /// Top-left pane cell.
  at: (u16, u16),
  /// Columns, kept inside the pane.
  width: u16,
  /// The opening right press's cell.
  press: (u16, u16),
  /// That drag went `ARM` cells away.
  armed: bool,
  /// Its items act on the panel's selected row.
  row: bool,
}

/// A menu row.
pub struct Item<C = Command> {
  pub label: std::borrow::Cow<'static, str>,
  /// Its key in help notation; empty when none.
  pub key: String,
  pub command: C,
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
  CopyAddress(String),
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
  Item { label: label.into(), key, command }
}

fn panel_bound(
  panel: &Panel,
  label: &'static str,
  action: Action,
  command: Command,
) -> Item {
  let key = panel.shortcut(action).map_or_default(keys::notation);
  Item { label: label.into(), key, command }
}

fn item(label: &'static str, key: &str, command: Command) -> Item {
  Item { label: label.into(), key: key.into(), command }
}

/// Copies a web address, as a middle click does.
fn copy_address(uri: String) -> Item {
  let copy = Command::CopyAddress(uri);
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

impl<C> ContextMenu<C> {
  /// A menu opened at a pane cell.
  pub fn new(
    items: Vec<Item<C>>,
    at: (u16, u16),
    columns: (u16, u16),
    bottom: u16,
    row: bool,
  ) -> Self {
    let mut menu =
      Self { items, selected: 0, at, width: 0, press: at, armed: false, row };
    menu.fit(columns, bottom);
    menu
  }

  pub fn entries(&self) -> &[Item<C>] {
    &self.items
  }
  pub fn selected(&self) -> usize {
    self.selected
  }
  pub fn at(&self) -> (u16, u16) {
    self.at
  }
  pub fn width(&self) -> u16 {
    self.width
  }
  pub fn is_row(&self) -> bool {
    self.row
  }
  pub fn select(&mut self, index: usize) {
    self.selected = index.min(self.items.len().saturating_sub(1));
  }
  pub fn into_command(self) -> Option<C> {
    self.items.into_iter().nth(self.selected).map(|item| item.command)
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
  pub fn items(&self) -> Items {
    let mut items = Items::default();
    let area = self.area();
    for (index, row) in (area.row..area.row + area.rows).enumerate() {
      items.push(Rect { row, rows: 1, ..area }, index);
    }
    items
  }
}

impl<C> ContextMenu<C> {
  fn fit(&mut self, (left, right): (u16, u16), bottom: u16) {
    // Short panes drop common items first.
    self.items.truncate(usize::from(bottom));
    let rows = u16::try_from(self.items.len()).unwrap_or(0);
    let room = right.saturating_sub(left);
    let width = u16::try_from(self.needed()).unwrap_or(u16::MAX).min(room);
    let (column, row) = self.at;
    let last = right.saturating_sub(width);
    let row = if row + rows <= bottom { row } else { row.saturating_sub(rows) };
    self.at = (column.min(last).max(left), row.min(bottom - rows));
    self.width = width;
  }

  pub fn allows(&self, action: Action) -> bool {
    !matches!(route(action), Route::Ignore)
  }

  pub fn input(&mut self, action: Action) -> Transition {
    let count = self.items.len().max(1);
    match route(action) {
      Route::Move(true) => self.selected = (self.selected + 1) % count,
      Route::Move(false) => self.selected = (self.selected + count - 1) % count,
      Route::Run => return Transition::Run,
      Route::Close => return Transition::Close,
      Route::Command => return Transition::Command,
      Route::Transfer => return Transition::Transfer,
      Route::Ignore => return Transition::Unhandled,
    }
    Transition::Stay
  }

  pub fn drag(&mut self, mouse: MouseEvent) -> bool {
    let (column, row) = (mouse.column, mouse.row);
    let far = column.abs_diff(self.press.0).max(row.abs_diff(self.press.1));
    self.armed |= far >= ARM;
    let item = self.items().at(column, row).filter(|_| self.armed);
    let Some(index) = item else { return false };
    self.select(index);
    matches!(mouse.kind, MouseEventKind::Up(_))
  }
  pub fn pixel_area(&self, screen: Screen, left: u16) -> Option<Bounds> {
    let area = self.area();
    let column = area.column.max(left);
    let columns = (area.column + area.columns).checked_sub(column);
    let columns = columns.filter(|columns| *columns > 0)?;
    Some(crate::paint::cell_bounds(
      screen,
      left,
      Rect { column, columns, ..area },
    ))
  }

  pub fn draw(
    &self,
    graphics: &mut Graphics,
    out: &mut impl Write,
    frame: Frame,
  ) -> Result<()> {
    let (left, top) = self.at;
    let keys = self.keys();
    let width = usize::from(self.width);
    let selected = self.selected;
    for (index, item) in self.items.iter().enumerate() {
      let row = top + u16::try_from(index).unwrap_or(0);
      let label = format!("{:<keys$}  {}", item.key, item.label);
      write!(out, "{}{}", frame.at.goto(row, left), sgr(Role::Bar))?;
      let shown = if index == selected {
        crate::grid::Shown::Chosen
      } else {
        crate::grid::Shown::Plain
      };
      crate::grid::cell(out, &label, width, keys, shown)?;
      write!(out, "{}", sgr(Role::Reset))?;
    }
    let Some(area) =
      self.pixel_area(frame.screen, frame.left).filter(|_| !frame.text)
    else {
      return Ok(());
    };
    let screen = frame.screen;
    let height = f32::from(screen.cell_height);
    let rows = ((area.y1 - area.y0) / height).round() as usize;
    for row in 0..rows {
      let color =
        if row == selected { theme::BADGE_FILL } else { theme::BAR_FILL };
      let y0 = area.y0 + row as f32 * height;
      let rect = Bounds { y0, y1: y0 + height, ..area };
      graphics.backdrop(rect, color, screen, frame.left, out)?;
    }
    Ok(())
  }
}
pub struct PageContext<'a> {
  pub reader: &'a Reader,
  pub link: Option<Target>,
  pub near: Option<Position>,
  pub glyph: Option<Position>,
  pub note: Option<(PageId, usize, String)>,
  pub figure: Option<(PageId, Bounds)>,
}

pub fn page_items(context: PageContext<'_>) -> Vec<Item> {
  let PageContext { reader, link, near, glyph, note, figure } = context;
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
  if let Some((page, index, contents)) = note {
    items.push(bound("Show note", Action::Peek, Command::Note(page, index)));
    let copy = Command::CopyText(contents, "Copied comment");
    items.push(item("Copy comment", "", copy));
  }
  if let Some(selection) = reader.selection {
    items.push(bound("Copy", Action::Yank, Command::Copy));
    // Region captures hold one page.
    if selection.anchor.page == reader.cursor.page {
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
  items
}
pub fn panel_items(
  panel: &Panel,
  destination: Option<&Destination>,
  covered: bool,
) -> Vec<Item> {
  let mut items = Vec::new();
  if let Some(destination) = destination {
    let address = match destination {
      Destination::Link(Target::Uri(uri)) => Some(uri.clone()),
      _ => None,
    };
    if !matches!(destination, Destination::None) {
      let label = if covered { "Open" } else { "Follow" };
      let action =
        if address.is_some() { Action::OpenUrl } else { Action::Follow };
      items.push(panel_bound(panel, label, action, Command::Key(action)));
      if panel.shortcut(Action::Swap).is_some() {
        let keep = Command::Key(Action::Swap);
        items.push(panel_bound(
          panel,
          "Follow, keep panel",
          Action::Swap,
          keep,
        ));
      }
    }
    // Addresses split by copying; the copy item does that.
    if !matches!(destination, Destination::None) && address.is_none() {
      items.push(item("Open in split", "", Command::RowSplit));
    }
    if panel.shortcut(Action::Peek).is_some() {
      items.push(panel_bound(
        panel,
        "Show note",
        Action::Peek,
        Command::Key(Action::Peek),
      ));
    }
    if let Some(uri) = address {
      items.push(copy_address(uri));
    } else if let Some(text) = panel.yank_text() {
      let copy = Command::CopyText(text, "Copied row");
      items.push(panel_bound(panel, "Copy row", Action::YankLine, copy));
    }
  }
  let close = Command::Key(Action::ClosePanel);
  items.push(panel_bound(panel, "Close panel", Action::ClosePanel, close));
  items
}
pub fn overview_items(page: Option<PageId>) -> Vec<Item> {
  let mut items = Vec::new();
  if let Some(page) = page {
    items.push(bound("Open", Action::Follow, Command::Key(Action::Follow)));
    let split = Command::Split(Target::Page(page));
    items.push(item("Open in split", "", split));
  }
  items.extend(history());
  let close = Command::Key(Action::Overview);
  items.push(bound("Close overview", Action::Quit, close));
  items
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
    ContextMenu::new(items, at, (0, 80), 40, false)
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
      items.iter().map(|i| (i.label.as_ref(), i.key.as_str())).collect();
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

  #[test]
  fn drag_release_and_navigation() {
    use crossterm::event::{KeyModifiers, MouseButton};
    let mut menu = menu((10, 5));
    let mouse = |column, row, kind| MouseEvent {
      column,
      row,
      kind,
      modifiers: KeyModifiers::NONE,
    };
    assert!(!menu.drag(mouse(11, 6, MouseEventKind::Up(MouseButton::Right))));
    assert_eq!(menu.selected(), 0);
    assert!(!menu.drag(mouse(12, 6, MouseEventKind::Drag(MouseButton::Right))));
    assert_eq!(menu.selected(), 1);
    assert!(menu.drag(mouse(11, 6, MouseEventKind::Up(MouseButton::Right))));
    assert_eq!(menu.input(Action::Motion(Motion::Down)), Transition::Stay);
    assert_eq!(menu.selected(), 0);
    assert_eq!(menu.input(Action::Motion(Motion::Up)), Transition::Stay);
    assert_eq!(menu.selected(), 1);
    assert_eq!(menu.input(Action::Follow), Transition::Run);
    assert_eq!(menu.input(Action::Command), Transition::Command);
    assert_eq!(menu.input(Action::Cancel), Transition::Close);
  }

  #[test]
  fn backdrops_follow_fitted_area() {
    let items = overview_items(Some(PageId(2)));
    let mut menu = ContextMenu::new(items, (28, 2), (4, 30), 3, true);
    assert_eq!(menu.entries().len(), 3);
    assert_eq!(menu.area().row, 0);
    assert!(menu.area().column >= 4);
    assert!(menu.area().column + menu.area().columns <= 30);
    menu.select(99);
    assert_eq!(menu.selected(), 2);
    assert!(menu.is_row());
    let screen = Screen {
      columns: 30,
      rows: 4,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let pixels = menu.pixel_area(screen, 20).expect("page overlap");
    assert_eq!(pixels, Bounds { x0: 0.0, y0: 0.0, x1: 80.0, y1: 48.0 });
    assert!(menu.pixel_area(screen, 30).is_none());
    let mut empty =
      ContextMenu::<()>::new(Vec::new(), (0, 0), (0, 0), 0, false);
    assert_eq!(empty.input(Action::Motion(Motion::Up)), Transition::Stay);
    assert_eq!(empty.into_command(), None);
  }
}
