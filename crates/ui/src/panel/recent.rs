//! Recent documents beside a card for the selected one: its first page
//! and `:info` facts, read off the input thread.
#[cfg(test)]
use anyhow::Result;

use vpdf_model::cache::Cache;
use vpdf_model::scene::{Op, Stage, Surface};
use vpdf_model::style::Role;
mod preferences;
pub use preferences::Preferences;
use preferences::Ratio;

const MIB: usize = 1 << 20;
use crate::layout::Screen;
use crate::panel::{self, Destination, Panel, Pointing, Row};
use std::collections::BTreeMap;
use std::path::PathBuf;
use vpdf_model::document::{
  CountsRequest, Facts, FactsRequest, Fit, LoadedFacts, RECENT_COUNTS,
};
use vpdf_model::geometry::At;

/// Panels narrower than this show no card.
const NARROW: u16 = 88;
/// Thumbnail pixels kept across selections.
const BUDGET: usize = 16 * MIB;

#[derive(Clone, Eq, PartialEq)]
pub struct Entry {
  pub path: PathBuf,
  pub cursor: vpdf_model::text::Position,
  pub rotation: u16,
  pub used: u64,
  pub marks: usize,
}

#[derive(Clone, Copy)]
pub struct Frame {
  pub at: At,
  pub width: u16,
  pub screen: Screen,
  pub now: u64,
}

struct Card {
  fit: Fit,
  serial: vpdf_model::image::PictureId,
  facts: Result<Facts, String>,
}

/// Recent document cards and pending loads.
pub struct Shelf {
  entries: Vec<Entry>,
  index: BTreeMap<PathBuf, usize>,
  logo: Option<(vpdf_model::image::PictureId, vpdf_model::image::Rgb)>,
  totals: BTreeMap<PathBuf, Result<usize, String>>,
  generation: u64,
  visible: Vec<PathBuf>,
  count_request: Option<CountsRequest>,
  cards: Cache<PathBuf, Card>,
  /// The last load sent, so a frame asks once.
  asked: Option<(PathBuf, Fit)>,
  pending: Option<FactsRequest>,
  /// The document the card shows.
  shown: Option<PathBuf>,
  /// First text line shown, and its limit.
  top: usize,
  last_top: usize,
  /// The card's first column, once drawn.
  column: Option<u16>,
  /// Clipped lines in full, by row.
  clipped: Vec<(u16, String)>,
  /// Last frame's selection and pointer.
  selected: usize,
  /// Pointed screen line, card `MIN`, away `MAX`.
  pointing: Option<isize>,
  /// Keys moved the selection since.
  keyed: bool,
  geometry: Geometry,
  preferences: Preferences,
  dragging: bool,
  last_press: Option<std::time::Instant>,
}

impl Shelf {
  pub fn new(entries: &[Entry]) -> Self {
    Self {
      entries: entries.to_vec(),
      index: entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.path.clone(), index))
        .collect(),
      logo: None,
      totals: BTreeMap::new(),
      generation: 0,
      visible: Vec::new(),
      count_request: None,
      cards: Cache::new(BUDGET),
      asked: None,
      pending: None,
      shown: None,
      top: 0,
      last_top: 0,
      column: None,
      clipped: Vec::new(),
      selected: 0,
      pointing: None,
      keyed: true,
      geometry: Geometry::default(),
      preferences: Preferences::default(),
      dragging: false,
      last_press: None,
    }
  }

  pub(crate) fn refresh(&mut self, entries: &[Entry]) -> bool {
    if self.entries == entries {
      return false;
    }
    for old in &self.entries {
      if !entries.iter().any(|entry| entry.path == old.path) {
        self.cards.remove(&old.path);
      }
    }
    let contains =
      |path: &PathBuf| entries.iter().any(|entry| &entry.path == path);
    if self.asked.as_ref().is_some_and(|(path, _)| !contains(path)) {
      self.asked = None;
    }
    if self.pending.as_ref().is_some_and(|request| !contains(&request.path)) {
      self.pending = None;
    }
    if self.shown.as_ref().is_some_and(|path| !contains(path)) {
      self.shown = None;
      self.top = 0;
      self.hide();
    }
    self.entries = entries.to_vec();
    self.index = entries
      .iter()
      .enumerate()
      .map(|(index, entry)| (entry.path.clone(), index))
      .collect();
    self.generation = self.generation.wrapping_add(1);
    self.totals.clear();
    self.cards = Cache::new(BUDGET);
    self.asked = None;
    self.pending = None;
    self.visible.clear();
    self.count_request =
      Some(CountsRequest { generation: self.generation, files: Vec::new() });
    true
  }

  pub(crate) fn suspend(&mut self) {
    self.visible.clear();
    self.count_request =
      Some(CountsRequest { generation: self.generation, files: Vec::new() });
    self.hide();
    self.geometry = Geometry::default();
    self.release_pointer();
  }

  /// Forgets the card while not drawn.
  pub(crate) fn hide(&mut self) {
    self.column = None;
    self.clipped.clear();
  }

  /// The card's first column, once drawn.
  pub const fn start(&self) -> Option<u16> {
    self.column
  }

  /// Whether the card covers a pane column.
  pub fn covers(&self, column: u16) -> bool {
    self.column.is_some_and(|start| column >= start)
  }

  /// A clipped card line in full.
  pub fn detail(&self, row: u16) -> Option<String> {
    let row = row.checked_sub(self.geometry.preview.row)?;
    let line = self.clipped.iter().find(|(at, _)| *at == row);
    line.map(|(_, text)| text.clone())
  }

  /// Scrolls the card's text by `rows`.
  pub(crate) fn scroll(&mut self, rows: isize) {
    self.top = self.top.saturating_add_signed(rows).min(self.last_top);
  }

  pub fn top(&self) -> usize {
    self.top
  }

  /// The row whose card shows.
  fn shown_row(&mut self, panel: &Panel) -> usize {
    // Keys win until the pointer moves.
    let pointing = match panel.pointing {
      Pointing::Row(index) => panel.screen_line(index),
      Pointing::Card => isize::MIN,
      Pointing::Away | Pointing::Control(_) => isize::MAX,
    };
    if panel.selected != self.selected {
      self.keyed = true;
    }
    // Resting at open is no move.
    if self.pointing.is_some_and(|before| before != pointing) {
      self.keyed = false;
    }
    (self.selected, self.pointing) = (panel.selected, Some(pointing));
    if self.keyed
      || panel
        .rows
        .get(panel.selected)
        .is_some_and(|row| matches!(row.destination, Destination::Directory(_)))
    {
      return panel.selected;
    }
    let file = |index: &usize| {
      matches!(
        panel.rows.get(*index).map(|row| &row.destination),
        Some(Destination::File(_))
      )
    };
    let card = panel.pointing == Pointing::Card;
    let kept = self.shown.as_ref().filter(|_| card).and_then(|path| {
      panel.rows.iter().position(|row| {
        matches!(&row.destination, Destination::File(file) if file == path)
      })
    });
    panel.pointing.row().filter(file).or(kept).unwrap_or(panel.selected)
  }

  pub fn set_logo(&mut self, image: vpdf_model::image::Rgb) {
    self.logo = Some((vpdf_model::image::PictureId::fresh(), image));
  }

  pub(crate) fn welcome<'a, S: Surface<'a>>(
    &self,
    scene: &mut S,
    screen: Screen,
    at: At,
    picker: crate::home::Picker,
    pictures: bool,
  ) -> Result<(), S::Error> {
    let logo = self.logo.as_ref().filter(|(_, image)| {
      pictures
        && screen.cell_width > 0
        && screen.cell_height > 0
        && screen.width() >= image.width() as f32
        && u32::from(screen.rows)
          >= (image.height() as u32).div_ceil(u32::from(screen.cell_height)) + 7
    });
    let image_rows = logo.map_or(0, |(_, image)| {
      (image.height() as u32).div_ceil(u32::from(screen.cell_height)) as u16 + 1
    });
    crate::home::welcome::commands(scene, screen, at, picker, image_rows);
    if let Some((id, image)) = logo {
      let top = screen.rows.saturating_sub(1 + image_rows + 4) / 2;
      let origin = (
        (screen.width() - image.width() as f32) / 2.0,
        f32::from(top * screen.cell_height),
      );
      scene.picture(*id, image, origin, Stage { screen, left: 0 })?;
    }
    Ok(())
  }

  pub fn invalidate(&mut self, generation: u64, path: &PathBuf) {
    if generation != self.generation {
      return;
    }
    self.cards.remove(path);
    if self.asked.as_ref().is_some_and(|(asked, _)| asked == path) {
      self.asked = None;
    }
  }

  pub fn set_pages(&mut self, path: PathBuf, pages: usize) {
    self.accept_count(self.generation, path, Ok(pages));
  }

  pub fn take_counts(&mut self) -> Option<CountsRequest> {
    self.count_request.take()
  }

  pub fn accept_count(
    &mut self,
    generation: u64,
    path: PathBuf,
    pages: Result<usize, String>,
  ) {
    if generation == self.generation && self.index.contains_key(&path) {
      self.totals.insert(path, pages);
    }
  }

  fn progress(&mut self, panel: &mut Panel, width: u16, height: u16) {
    let sources: Vec<_> = (panel.top..panel.line_count())
      .take(usize::from(height.saturating_sub(2)))
      .filter_map(|line| {
        let source = panel.line_source(line);
        let blank = panel.layout.as_ref().is_some_and(|layout| {
          layout.lines[line].row.label.is_empty()
            && layout.lines[line].row.destination == Destination::None
        });
        (!blank).then_some(source)
      })
      .collect();
    let visible: Vec<_> = sources
      .iter()
      .filter_map(|&source| match &panel.rows[source].destination {
        Destination::File(path) => Some(path.clone()),
        _ => None,
      })
      .take(RECENT_COUNTS)
      .collect();
    if visible != self.visible {
      let pending = visible
        .iter()
        .map(|path| {
          let known = self
            .totals
            .get(path)
            .and_then(|pages| pages.as_ref().ok())
            .copied();
          (path.clone(), known)
        })
        .collect();
      self.count_request =
        Some(CountsRequest { generation: self.generation, files: pending });
      self.visible = visible;
    }
    let cells = usize::from(width.saturating_sub(34)).min(24);
    for source in sources {
      let row = &mut panel.rows[source];
      let Destination::File(path) = &row.destination else { continue };
      let total =
        self.totals.get(path).and_then(|result| result.as_ref().ok()).copied();
      let page = self
        .index
        .get(path)
        .and_then(|&index| self.entries.get(index))
        .map_or(1, |entry| entry.cursor.page.0.saturating_add(1));
      let detail = if matches!(self.totals.get(path), Some(Err(_))) {
        "?".into()
      } else {
        reading_position(page, total, cells)
      };
      let width = if cells > 0 { cells + 5 } else { 4 };
      row.detail = format!("{detail:<width$}");
    }
  }

  pub fn take_request(&mut self) -> Option<FactsRequest> {
    self.pending.take()
  }

  pub fn accept(&mut self, path: PathBuf, loaded: LoadedFacts) {
    if !self.index.contains_key(&path) {
      return;
    }
    if loaded.generation != self.generation {
      return;
    }
    let LoadedFacts { fit, facts, .. } = loaded;
    if let Ok(facts) = &facts {
      self.totals.insert(path.clone(), Ok(facts.pages));
    }
    let card =
      Card { fit, facts, serial: vpdf_model::image::PictureId::fresh() };
    let answers =
      |(asked, fit): &(PathBuf, Fit)| *asked == path && *fit == card.fit;
    if self.asked.as_ref().is_some_and(answers) {
      self.asked = None;
    }
    let thumb = card.facts.as_ref().ok().and_then(|f| f.thumb.as_ref());
    let weight = thumb.map_or(1, |thumb| thumb.pixels().len());
    self.cards.insert(path, card, weight);
  }

  pub fn list_columns(panel: &Panel, width: u16) -> u16 {
    panel.shelf.as_ref().map_or(width, |shelf| {
      Geometry::new(
        width,
        panel.height,
        shelf.preferences.beside,
        shelf.preferences.below,
      )
      .list
      .columns
    })
  }

  pub fn contains(&self, column: u16, row: u16) -> bool {
    self.geometry.preview.contains(column, row)
  }

  pub(crate) fn divider_input(
    &mut self,
    mouse: vpdf_model::input::MouseEvent,
  ) -> bool {
    use vpdf_model::input::{MouseButton, MouseEventKind};
    let hit = self.geometry.divider.contains(mouse.column, mouse.row);
    match mouse.kind {
      MouseEventKind::Down(MouseButton::Left) if hit => {
        let now = std::time::Instant::now();
        let double = self
          .last_press
          .is_some_and(|last| now.duration_since(last).as_millis() < 350);
        self.last_press = Some(now);
        self.dragging = !double;
        if double {
          if self.geometry.stacked {
            self.preferences.below = None;
          } else {
            self.preferences.beside = None;
          }
        }
        true
      }
      MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
        if self.geometry.stacked {
          self.preferences.below = Some(Ratio::new(
            mouse.row.clamp(7, self.geometry.body() - 6),
            self.geometry.body(),
          ));
        } else {
          self.preferences.beside = Some(Ratio::new(
            mouse
              .column
              .saturating_add(1)
              .clamp(32, self.geometry.width() - 24),
            self.geometry.width(),
          ));
        }
        true
      }
      MouseEventKind::Up(MouseButton::Left) if self.dragging => {
        self.dragging = false;
        true
      }
      MouseEventKind::Down(_) => {
        self.dragging = false;
        self.last_press = None;
        false
      }
      _ => hit,
    }
  }

  pub fn dragging(&self) -> bool {
    self.dragging
  }

  pub fn release_pointer(&mut self) -> bool {
    let held = self.dragging;
    self.dragging = false;
    self.last_press = None;
    held
  }

  pub(crate) fn extend(
    &mut self,
    direction: crate::split::Direction,
    cells: u16,
  ) {
    use crate::split::Direction;
    let geometry = self.geometry;
    if geometry.preview.columns == 0 {
      return;
    }
    if geometry.stacked {
      let rows = geometry.divider.row;
      let position = match direction {
        Direction::Down => rows.saturating_add(cells),
        Direction::Up => rows.saturating_sub(cells),
        _ => return,
      };
      self.preferences.below = Some(Ratio::new(
        position.clamp(7, geometry.body() - 6),
        geometry.body(),
      ));
    } else {
      let columns = geometry.list.columns;
      let position = match direction {
        Direction::Right => columns.saturating_add(cells),
        Direction::Left => columns.saturating_sub(cells),
        _ => return,
      };
      self.preferences.beside = Some(Ratio::new(
        position.clamp(32, geometry.width() - 24),
        geometry.width(),
      ));
    }
  }

  pub fn preferences(&self) -> Preferences {
    self.preferences
  }

  pub fn set_preferences(&mut self, preferences: Preferences) {
    self.preferences = preferences;
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    panel: &mut Panel,
    scene: &mut S,
    frame: Frame,
    pictures: bool,
  ) -> Result<(), S::Error> {
    let Frame { at, width, screen, now } = frame;
    let geometry = Geometry::new(
      width,
      screen.rows,
      self.preferences.beside,
      self.preferences.below,
    );
    if geometry != self.geometry {
      if geometry.stacked != self.geometry.stacked {
        self.dragging = false;
      }
      panel.resize(geometry.list.rows);
      panel.reveal();
    }
    self.geometry = geometry;
    panel.reflow(usize::from(
      geometry.list.columns.saturating_sub(if geometry.list.columns >= 24 {
        3
      } else {
        1
      }),
    ));
    self.progress(panel, geometry.list.columns, geometry.list.rows);
    panel.scene_list(scene, at, geometry.list.columns, geometry.list.rows);
    self.hide();
    let preview = geometry.preview;
    if preview.columns == 0 || preview.rows == 0 {
      return Ok(());
    }
    let row = panel.rows.get(self.shown_row(panel));
    let target = row.map(|row| &row.destination);
    let preview_at =
      At { column: at.column + preview.column, row: at.row + preview.row };
    if geometry.stacked {
      crate::paint::move_to(scene, at, geometry.divider.row, 0u16);
      scene.push(Op::Style(Role::Muted));
      scene.text(&"─".repeat(usize::from(width)));
      scene.push(Op::Style(Role::Reset));
    }
    if let Some(Destination::Directory(path)) = target {
      self.shown = None;
      self.top = 0;
      let members = panel.recent_members(&Destination::Directory(path.clone()));
      let selected = members
        .iter()
        .filter(|path| panel.removal.checked.contains(*path))
        .count();
      scene.push(Op::Style(Role::Heading));
      crate::paint::line(
        scene,
        preview_at,
        0u16,
        "Directory selection",
        preview.columns,
      );
      scene.push(Op::Style(Role::Reset));
      crate::paint::line(
        scene,
        preview_at,
        2u16.min(preview.rows - 1),
        &format!("{selected}/{} files selected", members.len()),
        preview.columns,
      );
      return Ok(());
    }
    let Some(Destination::File(path)) = target else { return Ok(()) };
    let Some(entry) =
      self.index.get(path).and_then(|&index| self.entries.get(index)).cloned()
    else {
      return Ok(());
    };
    if self.shown.as_ref() != Some(path) {
      self.shown = Some(path.clone());
      self.top = 0;
    }
    self.column = (!geometry.stacked).then_some(preview.column);
    let screen = Screen {
      columns: preview.columns,
      rows: preview.rows + 1,
      origin: (screen.origin.0 + preview.column, screen.origin.1 + preview.row),
      ..screen
    };
    let area =
      Area { at: preview_at, column: 0, columns: preview.columns, screen, now };
    self.card(scene, area, &entry, pictures)
  }

  fn card<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
    area: Area,
    entry: &Entry,
    pictures: bool,
  ) -> Result<(), S::Error> {
    let Area { at, column, columns, screen, now } = area;
    let rows = usize::from(screen.rows.saturating_sub(1));
    for row in 0..rows {
      crate::paint::move_to(scene, at, row, column + columns - 1);
      scene.push(Op::Style(Role::Muted));
      scene.text("│");
      scene.push(Op::Style(Role::Reset));
    }
    // One column of margin on each side of the text.
    let inner = usize::from(columns).saturating_sub(3);
    let picture = if pictures { rows.saturating_sub(9) } else { 0 };
    let (cell_width, cell_height) =
      (u32::from(screen.cell_width), u32::from(screen.cell_height));
    let fit = (picture >= 4 && inner >= 4).then(|| {
      preview_fit(inner as u32 * cell_width, picture as u32 * cell_height)
    });
    self.ask(entry, fit);
    let card = self.cards.get(&entry.path).filter(|card| card.fit == fit);
    let thumb = card.and_then(|card| card.facts.as_ref().ok()?.thumb.as_ref());
    if let (Some(thumb), true, Some(_)) = (thumb, pictures, fit) {
      let x = f32::from(column + 1) * cell_width as f32
        + (inner as f32 * cell_width as f32 - thumb.width() as f32) / 2.0;
      let y = cell_height as f32;
      let stage = Stage { screen, left: 0 };
      scene.picture(card.unwrap().serial, thumb, (x, y), stage)?;
    }
    // Text follows the thumbnail's own height; a loading card holds the
    // whole box so its text does not jump twice.
    let shown = match (thumb, card) {
      (Some(thumb), _) => Some((thumb.height() as u32).div_ceil(cell_height)),
      (None, None) => fit.map(|_| picture as u32),
      (None, Some(_)) => None,
    };
    let top = shown.map_or(1, |rows| rows as usize + 2);
    let texts = lines(entry, card, now);
    let lines: Vec<Row> = texts.iter().map(|line| Row::parse(line)).collect();
    let lines = super::layout::Layout::help(&lines, inner).lines;
    let room = rows.saturating_sub(top);
    self.last_top = lines.len().saturating_sub(room);
    self.top = self.top.min(self.last_top);
    let shown = lines.iter().skip(self.top).take(room);
    for (row, line) in (top..).zip(shown) {
      let keys = line.keys;
      let line = &line.row;
      crate::paint::move_to(scene, at, row, column + 1);
      scene.push(Op::Style(Role::Reset));
      let used = line.scene(scene, Some(Role::Reset), inner, keys, "");
      let full = line.scene(&mut (), None, usize::MAX, keys, "");
      if full > used {
        self.clipped.push((row as u16, line.label.clone()));
      }
    }
    scene.push(Op::Style(Role::Reset));
    Ok(())
  }

  /// Sends a load unless the card or a load for it is already here.
  fn ask(&mut self, entry: &Entry, fit: Fit) {
    let have = self.cards.get(&entry.path).is_some_and(|card| card.fit == fit);
    let asked = self
      .asked
      .as_ref()
      .is_some_and(|(path, asked)| *path == entry.path && *asked == fit);
    if have || asked {
      return;
    }
    self.asked = Some((entry.path.clone(), fit));
    self.pending = Some(FactsRequest {
      generation: self.generation,
      path: entry.path.clone(),
      cursor: entry.cursor,
      rotation: entry.rotation,
      fit,
    });
  }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Geometry {
  list: crate::split::Rect,
  divider: crate::split::Rect,
  preview: crate::split::Rect,
  stacked: bool,
}

impl Geometry {
  fn width(self) -> u16 {
    self.list.columns + if self.stacked { 0 } else { self.preview.columns }
  }

  fn body(self) -> u16 {
    if self.stacked {
      self.divider.row + 1 + self.preview.rows
    } else {
      self.list.rows.saturating_sub(1)
    }
  }

  fn new(
    width: u16,
    rows: u16,
    beside: Option<Ratio>,
    below: Option<Ratio>,
  ) -> Self {
    use crate::split::Rect;
    let body = rows.saturating_sub(1);
    if width >= NARROW && body >= 4 {
      let list = beside
        .map_or(width - width / 4, |ratio| ratio.cells(width))
        .clamp(32, width - 24);
      return Self {
        list: Rect { columns: list, rows, ..Rect::default() },
        divider: Rect {
          column: list - 1,
          columns: 1,
          rows: body,
          ..Rect::default()
        },
        preview: Rect {
          column: list,
          columns: width - list,
          rows: body,
          ..Rect::default()
        },
        stacked: false,
      };
    }
    if width >= 32 && body >= 17 {
      let split = below
        .map_or(body * 2 / 3, |ratio| ratio.cells(body))
        .clamp(7, body - 6);
      return Self {
        list: Rect { columns: width, rows: split + 1, ..Rect::default() },
        divider: Rect {
          row: split,
          columns: width,
          rows: 1,
          ..Rect::default()
        },
        preview: Rect {
          row: split + 1,
          columns: width,
          rows: body - split - 1,
          ..Rect::default()
        },
        stacked: true,
      };
    }
    Self {
      list: Rect { columns: width, rows, ..Rect::default() },
      ..Self::default()
    }
  }
}

fn preview_fit(width: u32, height: u32) -> (u32, u32) {
  let pixels = f64::from(width) * f64::from(height);
  let scale = ((BUDGET / 3) as f64 / pixels.max(1.0)).sqrt().min(1.0);
  ((f64::from(width) * scale) as u32, (f64::from(height) * scale) as u32)
}

fn reading_position(page: usize, total: Option<usize>, cells: usize) -> String {
  let Some(total) = total.filter(|&total| total > 0) else {
    return "…".into();
  };
  let ratio = page.min(total) as f64 / total as f64;
  let percent = (ratio * 100.0).round() as usize;
  if cells == 0 {
    return format!("{percent:>3}%");
  }
  let full = (ratio * cells as f64).round() as usize;
  let empty = cells.saturating_sub(full);
  format!("{}{} {percent:>3}%", "▄".repeat(full), " ".repeat(empty))
}

/// The card's place in a pane.
#[derive(Clone, Copy)]
struct Area {
  at: At,
  column: u16,
  columns: u16,
  screen: Screen,
  now: u64,
}

/// Reading state first, then `:info`'s document facts.
fn lines(entry: &Entry, card: Option<&Card>, now: u64) -> Vec<String> {
  let facts = card.and_then(|card| card.facts.as_ref().ok());
  let mut lines = Vec::new();
  if let Some(title) = facts.and_then(|facts| {
    facts
      .metadata
      .iter()
      .find(|(name, value)| name == "Title" && !value.trim().is_empty())
      .map(|(_, title)| title)
  }) {
    lines.push(format!("# {title}"));
    lines.push(String::new());
  }
  lines.push(format!("Page: {}", entry.cursor.page.0.saturating_add(1)));
  if let Some(section) = facts.and_then(|facts| facts.section.as_deref()) {
    lines.push(format!("Section: {section}"));
  }
  lines.push(format!("Last read: {}", ago(now.saturating_sub(entry.used))));
  if let Some(facts) = facts {
    lines.extend(
      panel::document_lines(String::new(), facts.bytes, facts.pages)
        .into_iter()
        .filter(|line| line.starts_with("Size:")),
    );
  }
  if entry.marks > 0 {
    lines.push(format!("Marks: {}", entry.marks));
  }
  match card.map(|card| &card.facts) {
    None => lines.push("Loading…".into()),
    Some(Err(error)) => lines.push(format!("Error: {error}")),
    Some(Ok(_)) => {}
  }
  lines
}

pub(super) fn scene_progress<'a>(
  scene: &mut impl vpdf_model::scene::Draw<'a>,
  detail: &str,
  base: Role,
) {
  let tail = super::rows::quiet(base);
  let track = detail.get(..detail.len().saturating_sub(5)).filter(|track| {
    !track.is_empty() && track.chars().all(|ch| ch == '▄' || ch == ' ')
  });
  let Some(track) = track else {
    scene.push(Op::Style(tail));
    scene.text(detail);
    return;
  };
  let full = track.trim_end_matches(' ');
  scene.push(Op::Style(Role::Mode));
  scene.text(full);
  scene.push(Op::Style(Role::Muted));
  scene.text(&"▄".repeat(track.len() - full.len()));
  scene.push(Op::Style(tail));
  scene.text(&detail[track.len()..]);
  scene.push(Op::Style(base));
}

/// `milliseconds` as the largest whole unit.
fn ago(milliseconds: u64) -> String {
  let minutes = milliseconds / 60_000;
  let (count, unit) = match minutes {
    0 => return "just now".into(),
    1..60 => (minutes, "minute"),
    60..1440 => (minutes / 60, "hour"),
    1440..43_200 => (minutes / 1440, "day"),
    43_200..525_600 => (minutes / 43_200, "month"),
    _ => (minutes / 525_600, "year"),
  };
  let plural = if count == 1 { "" } else { "s" };
  format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::keys::PanelKind;
  use vpdf_model::input::{MouseEvent, MouseEventKind};
  #[test]
  fn refresh_preserves_paths_and_releases_cards() {
    let mut entries: Vec<_> = (0..30)
      .map(|index| Entry {
        path: format!("{index}.pdf").into(),
        cursor: vpdf_model::text::Position::default(),
        rotation: 0,
        used: 0,
        marks: 0,
      })
      .collect();
    let rows = |entries: &[Entry]| {
      entries
        .iter()
        .map(|entry| {
          Row::new(
            entry.path.display().to_string(),
            entry.used.to_string(),
            Destination::File(entry.path.clone()),
          )
        })
        .collect()
    };
    let mut panel = Panel::recent(rows(&entries), &entries);
    panel.selected = 22;
    panel.top = 18;
    panel.pointing = Pointing::Row(24);
    let removed = entries.remove(0);
    let shelf = panel.shelf_mut().unwrap();
    shelf.ask(&removed, None);
    shelf.accept(
      removed.path.clone(),
      LoadedFacts { generation: 0, fit: None, facts: Err("missing".into()) },
    );
    shelf.shown = Some(removed.path.clone());
    assert_eq!(shelf.cards.used(), 1);
    entries[21].used = 2;
    panel.refresh_recent(rows(&entries), &entries);
    assert_eq!(panel.selected(), 21);
    assert_eq!(panel.top(), 17);
    assert_eq!(panel.pointing, Pointing::Row(23));
    let shelf = panel.shelf_mut().unwrap();
    assert_eq!(shelf.cards.used(), 0);
    assert!(shelf.pending.is_none());
    assert!(shelf.shown.is_none());
    shelf.accept(
      removed.path,
      LoadedFacts { generation: 0, fit: None, facts: Err("late".into()) },
    );
    assert_eq!(shelf.cards.used(), 0);
  }

  #[test]
  fn ago_names_the_largest_unit() {
    let minute = 60_000;
    for (elapsed, want) in [
      (0, "just now"),
      (minute, "1 minute ago"),
      (59 * minute, "59 minutes ago"),
      (60 * minute, "1 hour ago"),
      (3 * 1440 * minute, "3 days ago"),
      (65 * 1440 * minute, "2 months ago"),
      (800 * 1440 * minute, "2 years ago"),
    ] {
      assert_eq!(ago(elapsed), want, "{elapsed}");
    }
  }

  #[test]
  fn replies_match_path_and_fit() -> Result<()> {
    let entries: Vec<_> = ["first.pdf", "second.pdf"]
      .into_iter()
      .map(|path| Entry {
        path: path.into(),
        cursor: vpdf_model::text::Position::default(),
        rotation: 0,
        used: 0,
        marks: 0,
      })
      .collect();
    let rows = entries
      .iter()
      .map(|entry| {
        Row::new(
          entry.path.display().to_string(),
          String::new(),
          Destination::File(entry.path.clone()),
        )
      })
      .collect();
    let mut panel = Panel::recent(rows, &entries);

    let screen = crate::testing::screen::cells(100, 40);
    let draw = |panel: &mut Panel, screen: Screen| -> Result<String> {
      let mut recorder = crate::testing::empty();
      panel.scene(
        &mut recorder,
        Frame { at: At::default(), width: screen.columns, screen, now: 0 },
        true,
      )?;
      Ok(crate::testing::text(&recorder.into_scene()))
    };
    assert!(draw(&mut panel, screen)?.contains("Loading…"));
    let first = panel.shelf_mut().unwrap().take_request().unwrap();
    draw(&mut panel, screen)?;
    assert!(panel.shelf_mut().unwrap().take_request().is_none());
    panel.choose(1);
    draw(&mut panel, screen)?;
    let second = panel.shelf_mut().unwrap().take_request().unwrap();
    assert_ne!(first.path, second.path);
    panel.shelf_mut().unwrap().accept(
      first.path,
      LoadedFacts {
        generation: 0,
        fit: first.fit,
        facts: Err("Old path".into()),
      },
    );
    assert!(!draw(&mut panel, screen)?.contains("Old path"));
    assert!(panel.shelf_mut().unwrap().take_request().is_none());
    let resized = Screen { cell_width: 10, ..screen };
    draw(&mut panel, resized)?;
    let current = panel.shelf_mut().unwrap().take_request().unwrap();
    assert_ne!(current.fit, second.fit);
    panel.shelf_mut().unwrap().accept(
      second.path,
      LoadedFacts {
        generation: 0,
        fit: second.fit,
        facts: Err("Old fit".into()),
      },
    );
    let pending = draw(&mut panel, resized)?;
    assert!(pending.contains("Loading…") && !pending.contains("Old fit"));
    assert!(panel.shelf_mut().unwrap().take_request().is_none());
    panel.shelf_mut().unwrap().accept(
      current.path,
      LoadedFacts {
        generation: 0,
        fit: current.fit,
        facts: Err("Current error".into()),
      },
    );
    let ready = draw(&mut panel, resized)?;
    assert!(ready.contains("Current") && ready.contains("error"));
    assert!(!ready.contains("Loading…"));
    assert_eq!(panel.shelf().unwrap().clipped, []);
    assert!(panel.shelf_mut().unwrap().take_request().is_none());
    Ok(())
  }

  /// U29: the card follows the pointer.
  #[test]
  fn cards_follow_the_pointer() -> Result<()> {
    let names = [
      "first.pdf",
      "a-very-long-recent-document-name.pdf",
      "third.pdf",
      "fourth.pdf",
      "fifth.pdf",
      "sixth.pdf",
    ];
    let entries: Vec<_> = names
      .iter()
      .map(|name| Entry {
        path: PathBuf::from(name),
        cursor: vpdf_model::text::Position::default(),
        rotation: 0,
        used: 0,
        marks: 0,
      })
      .collect();
    let screen = crate::testing::screen::cells(100, 6);
    let mut shelf = Shelf::new(&entries);
    let mut panel = Panel::new(
      PanelKind::Recent,
      entries
        .iter()
        .map(|entry| {
          Row::new(
            entry.path.display().to_string(),
            String::new(),
            Destination::File(entry.path.clone()),
          )
        })
        .collect(),
    );
    let draw = |shelf: &mut Shelf, panel: &mut Panel| -> Result<String> {
      let mut recorder = crate::testing::empty();
      shelf.scene(
        panel,
        &mut recorder,
        Frame { at: At::default(), width: screen.columns, screen, now: 0 },
        false,
      )?;
      Ok(crate::testing::text(&recorder.into_scene()))
    };
    // (pointing, list top, selected, shown)
    let (row, card, away) = (Pointing::Row, Pointing::Card, Pointing::Away);
    for (step, (pointing, top, selected, shown)) in [
      (row(2), 0, 0, 0),
      (row(1), 0, 0, 1),
      (card, 0, 0, 1),
      (away, 0, 0, 0),
      (row(0), 0, 0, 0),
      (card, 0, 0, 0),
      (card, 0, 2, 2),
      (row(1), 0, 2, 1),
      (row(1), 0, 0, 0),
      (row(3), 2, 5, 5),
      (row(4), 2, 5, 4),
    ]
    .into_iter()
    .enumerate()
    {
      panel.pointing = pointing;
      panel.top = top;
      panel.selected = selected;
      draw(&mut shelf, &mut panel)?;
      let path = &entries[shown].path;
      assert_eq!(shelf.shown.as_ref(), Some(path), "step {step}");
    }
    panel.top = 0;
    panel.pointing = Pointing::Row(1);
    draw(&mut shelf, &mut panel)?;
    let column = shelf.column.expect("card drawn");
    assert!(shelf.covers(column) && !shelf.covers(column - 1));
    assert_eq!(shelf.detail(0), None);
    shelf.accept(
      entries[1].path.clone(),
      LoadedFacts {
        generation: 0,
        fit: None,
        facts: Ok(Facts {
          pages: 10,
          bytes: 100,
          metadata: vec![("Title".into(), "Preview title".into())],
          section: Some("Section one".into()),
          thumb: None,
        }),
      },
    );
    let before = draw(&mut shelf, &mut panel)?;
    assert!(before.contains("Preview title") && !before.contains("Size"));
    panel.shelf = Some(shelf);
    panel.pointer(MouseEvent::new(MouseEventKind::ScrollDown, column, 3));
    let mut shelf = panel.shelf.take().expect("shelf");
    assert_eq!(panel.top, 0);
    let after = draw(&mut shelf, &mut panel)?;
    assert!(after.contains("Size") && !after.contains("Preview title"));
    Ok(())
  }
}

#[cfg(test)]
mod design_tests {
  use super::*;
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};

  #[test]
  fn positions_round_and_clip() {
    for (page, total, cells, expected) in [
      (1, None, 24, "…"),
      (1, Some(0), 24, "…"),
      (1, Some(3), 0, " 33%"),
      (20, Some(10), 0, "100%"),
    ] {
      assert_eq!(reading_position(page, total, cells), expected);
    }
  }

  #[test]
  fn tiny_progress_stays_empty() {
    let mut scene = vpdf_model::scene::Scene::new();
    scene_progress(
      &mut scene,
      &reading_position(1, Some(1000), 24),
      Role::Reset,
    );
    assert!(
      crate::testing::spans(&scene)
        .iter()
        .all(|(role, text)| { *role != Role::Mode || text.is_empty() })
    );
  }

  #[test]
  fn layout_reserves_status_and_divider() {
    for (width, rows, stacked, preview) in [
      (160, 40, false, true),
      (120, 36, false, true),
      (80, 28, true, true),
      (48, 32, true, true),
      (12, 4, false, false),
    ] {
      let geometry = Geometry::new(width, rows, None, None);
      assert_eq!(geometry.stacked, stacked);
      assert_eq!(geometry.preview.rows > 0, preview);
      assert!(geometry.preview.row + geometry.preview.rows < rows);
      assert!(geometry.preview.column + geometry.preview.columns <= width);
      if preview {
        assert!(
          !geometry
            .preview
            .contains(geometry.divider.column, geometry.divider.row)
        );
      }
    }
  }

  #[test]
  fn dividers_clamp_and_reset() {
    let mut shelf = Shelf::new(&[]);
    for (width, rows) in [(120, 36), (48, 32)] {
      shelf.geometry = Geometry::new(width, rows, None, None);
      let divider = shelf.geometry.divider;
      assert!(shelf.divider_input(MouseEvent::new(
        MouseEventKind::Down(MouseButton::Left),
        divider.column,
        divider.row
      )));
      assert!(shelf.divider_input(MouseEvent::new(
        MouseEventKind::Drag(MouseButton::Left),
        u16::MAX,
        u16::MAX
      )));
      shelf.geometry = Geometry::new(
        width,
        rows,
        shelf.preferences.beside,
        shelf.preferences.below,
      );
      let clamped = shelf.geometry;
      assert!(clamped.preview.columns >= 24);
      assert!(clamped.preview.rows >= 5);
      assert!(shelf.divider_input(MouseEvent::new(
        MouseEventKind::Up(MouseButton::Left),
        0,
        0
      )));
      shelf.last_press = None;
    }
  }

  #[test]
  fn refresh_rejects_old_counts_and_cards() {
    let mut entries = vec![Entry {
      path: "/books/a.pdf".into(),
      cursor: vpdf_model::text::Position::default(),
      rotation: 0,
      used: 0,
      marks: 0,
    }];
    let mut shelf = Shelf::new(&entries);
    shelf.accept_count(0, entries[0].path.clone(), Ok(12));
    entries[0].used = 1;
    shelf.refresh(&entries);
    shelf.accept_count(0, entries[0].path.clone(), Ok(12));
    shelf.accept(
      entries[0].path.clone(),
      LoadedFacts { generation: 0, fit: None, facts: Err("stale".into()) },
    );
    assert!(shelf.totals.is_empty());
    assert_eq!(shelf.cards.used(), 0);
    shelf.accept_count(1, entries[0].path.clone(), Ok(7));
    assert_eq!(shelf.totals[&entries[0].path], Ok(7));
  }
}

#[cfg(test)]
mod viewport_tests {
  use super::*;
  use crate::keys::{Action, Input};
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};

  #[test]
  fn scrolling_uses_list_room() -> Result<()> {
    for (columns, rows) in [(160, 40), (120, 36), (80, 28), (48, 32), (8, 4)] {
      let entries: Vec<_> = (0..100)
        .map(|index| Entry {
          path: format!("/books/日本 語 long document {index}.pdf").into(),
          cursor: vpdf_model::text::Position::default(),
          rotation: 0,
          used: 0,
          marks: 0,
        })
        .collect();
      let mut panel = Panel::recent(
        entries
          .iter()
          .map(|entry| Row::recent(&entry.path, entry.cursor.page, None))
          .collect(),
        &entries,
      );
      let screen = crate::testing::screen::cells(columns, rows);
      let mut scene = crate::testing::empty();
      panel.scene(
        &mut scene,
        Frame { at: At::default(), width: columns, screen, now: 0 },
        false,
      )?;
      crate::testing::lines(&scene.into_scene(), columns);
      let height = panel.height;
      panel.navigate(Input::plain(Action::Scroll(0.5)), usize::from(rows));
      assert_eq!(panel.selected(), usize::from(height.saturating_sub(2)) / 2);
      assert!(panel.selected_row().is_some());
      let requests = panel.shelf_mut().unwrap().take_counts().unwrap();
      assert!(
        requests.files.len()
          <= usize::from(height.saturating_sub(2)).min(RECENT_COUNTS)
      );
    }
    Ok(())
  }

  #[test]
  fn divider_double_click_resets() {
    for (width, rows) in [(120, 36), (48, 32)] {
      let mut shelf = Shelf::new(&[]);
      shelf.preferences.beside = Some(Ratio::new(40, width));
      shelf.preferences.below = Some(Ratio::new(10, rows - 1));
      shelf.geometry = Geometry::new(
        width,
        rows,
        shelf.preferences.beside,
        shelf.preferences.below,
      );
      let divider = shelf.geometry.divider;
      for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Down(MouseButton::Left),
      ] {
        assert!(shelf.divider_input(MouseEvent::new(
          kind,
          divider.column,
          divider.row
        )));
      }
      assert!(!shelf.dragging());
      if shelf.geometry.stacked {
        assert!(shelf.preferences.below.is_none());
      } else {
        assert!(shelf.preferences.beside.is_none());
      }
    }
  }
}

#[cfg(test)]
mod presentation_tests {
  use super::*;
  use crate::panel::Style;
  use unicode_width::UnicodeWidthStr;

  #[test]
  fn groups_keep_spacing_and_quiet_tracks() -> Result<()> {
    let paths = ["/a/long/parent/path/books/a.pdf", "/other/reference/b.pdf"];
    let entries: Vec<_> = paths
      .iter()
      .map(|path| Entry {
        path: path.into(),
        cursor: vpdf_model::text::Position::default(),
        rotation: 0,
        used: 0,
        marks: 0,
      })
      .collect();
    let mut rows = Vec::new();
    for entry in &entries {
      rows.push(Row {
        style: Style::Heading,
        ..Row::new(
          entry.path.parent().unwrap().display().to_string(),
          "1 file".into(),
          Destination::Directory(entry.path.parent().unwrap().into()),
        )
      });
      rows.push(Row {
        style: Style::Path,
        depth: 1,
        ..Row::new(
          entry.path.file_name().unwrap().to_string_lossy().into_owned(),
          String::new(),
          Destination::File(entry.path.clone()),
        )
      });
    }
    let mut panel = Panel::recent(rows, &entries);
    panel.shelf_mut().unwrap().set_pages(entries[0].path.clone(), 10);
    let screen = crate::testing::screen::cells(48, 32);
    let mut recorder = crate::testing::empty();
    panel.scene(
      &mut recorder,
      Frame { at: At::default(), width: 48, screen, now: 0 },
      false,
    )?;
    let scene = recorder.into_scene();
    let lines = crate::testing::lines(&scene, 48);
    assert_eq!(lines[&2].split("a.pdf").next().unwrap().width(), 7);
    assert!(lines[&1].contains("books"));
    assert_eq!(panel.row_at(8, 3), None);
    assert_eq!(panel.row_at(8, 4), Some(2));
    assert_eq!(panel.rows[1].detail.width(), panel.rows[3].detail.width());
    let spans = crate::testing::spans(&scene);
    assert!(
      spans
        .iter()
        .any(|(role, text)| *role == Role::File && text.contains("a.pdf"))
    );
    Ok(())
  }
}

#[cfg(test)]
mod review_tests {
  use super::*;
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};

  #[test]
  fn ratios_survive_resizing_and_clamping() {
    let mut shelf = Shelf::new(&[]);
    shelf.geometry = Geometry::new(120, 40, None, None);
    shelf.divider_input(MouseEvent::new(
      MouseEventKind::Down(MouseButton::Left),
      89,
      10,
    ));
    shelf.divider_input(MouseEvent::new(
      MouseEventKind::Drag(MouseButton::Left),
      59,
      10,
    ));
    let preferences = shelf.preferences();
    for (width, expected) in [(240, 120), (88, 44), (160, 80)] {
      let geometry =
        Geometry::new(width, 40, preferences.beside, preferences.below);
      assert_eq!(geometry.list.columns, expected);
    }
    shelf.geometry = Geometry::new(80, 40, None, None);
    shelf.extend(crate::split::Direction::Up, 6);
    let preferences = shelf.preferences();
    assert_eq!(
      Geometry::new(80, 79, preferences.beside, preferences.below).divider.row,
      40
    );
    assert_eq!(
      Geometry::new(48, 18, preferences.beside, preferences.below).divider.row,
      9
    );
  }

  #[test]
  fn preview_uses_available_pixels() -> Result<()> {
    let entry = Entry {
      path: "/books/a.pdf".into(),
      cursor: vpdf_model::text::Position::default(),
      rotation: 0,
      used: 0,
      marks: 0,
    };
    let mut panel = Panel::recent(
      vec![Row::recent(&entry.path, entry.cursor.page, None)],
      &[entry],
    );
    let screen = crate::testing::screen::cells(120, 60);
    panel.resize(screen.rows);
    assert_eq!(Shelf::list_columns(&panel, screen.columns), 90);
    let mut scene = crate::testing::empty();
    panel.scene(
      &mut scene,
      Frame { at: At::default(), width: 120, screen, now: 0 },
      true,
    )?;
    let request = panel.shelf_mut().unwrap().take_request().unwrap();
    let (width, height) = request.fit.unwrap();
    assert_eq!(width, 27 * u32::from(screen.cell_width));
    assert!(height > 14 * u32::from(screen.cell_height));
    assert!(
      !crate::testing::text(&scene.into_scene()).contains("First-page preview")
    );
    Ok(())
  }

  #[test]
  fn oversized_previews_fit_the_cache() {
    assert_eq!(preview_fit(216, 448), (216, 448));
    for (width, height) in [(8000, 4000), (u32::MAX, u32::MAX)] {
      let (w, h) = preview_fit(width, height);
      assert!(u64::from(w) * u64::from(h) * 3 <= BUDGET as u64);
      assert!(w > 0 && h > 0);
      assert!(
        (f64::from(w) / f64::from(h) - f64::from(width) / f64::from(height))
          .abs()
          < 0.01
      );
    }
  }

  #[test]
  fn titles_and_sections_wrap() {
    let rows = super::super::layout::Layout::help(
      &[
        Row::parse("# PostgreSQL 18.6 Documentation"),
        Row::parse("Section: Multi Head Attention"),
      ],
      20,
    )
    .lines;
    assert_eq!(
      rows.iter().map(|line| line.row.label.as_str()).collect::<Vec<_>>(),
      [
        "PostgreSQL 18.6",
        "Documentation",
        "Section: Multi Head",
        "         Attention"
      ]
    );
    assert_eq!(rows[1].row.style, super::super::Style::Heading);
  }

  #[test]
  fn selected_rows_avoid_muted() {
    let heading = Row {
      style: super::super::Style::Heading,
      ..Row::new(
        "/a/books".into(),
        String::new(),
        Destination::Directory("/a/books".into()),
      )
    };
    let command = Row {
      style: super::super::Style::CommandKey,
      ..Row::new(
        ":edit <file>: open".into(),
        String::new(),
        Destination::Directory("/a/books".into()),
      )
    };
    for (base, tail) in [
      (Role::Reset, Role::Muted),
      (Role::Selected, Role::Selected),
      (Role::Current, Role::Current),
    ] {
      for detail in [reading_position(2, Some(5), 5), "12 pages".into()] {
        let mut scene = vpdf_model::scene::Scene::new();
        scene_progress(&mut scene, &detail, base);
        let spans = crate::testing::spans(&scene);
        assert_eq!(spans.last().unwrap().0, tail, "{base:?}: {detail}");
      }
      let mut scene = vpdf_model::scene::Scene::new();
      heading.scene(&mut scene, Some(base), 40, 0, "");
      let spans = crate::testing::spans(&scene);
      assert_eq!(spans[0].0, tail, "{base:?}: heading");
      let mut scene = vpdf_model::scene::Scene::new();
      command.scene(&mut scene, Some(base), 40, 12, "");
      let quiet = crate::testing::quiet_on_selected(&scene);
      assert!(quiet.is_empty(), "{base:?}: {quiet:?}");
      let argument = crate::testing::spans(&scene)
        .into_iter()
        .any(|(role, text)| role == Role::Argument && text == "<file>");
      assert_eq!(argument, base == Role::Reset, "{base:?}");
    }
  }
}
