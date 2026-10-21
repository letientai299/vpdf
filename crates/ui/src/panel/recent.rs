//! Recent documents beside a card for the selected one: its first page
//! and `:info` facts, read off the input thread.

use crate::Result;
use vpdf_model::cache::Cache;
use vpdf_model::scene::{Op, Surface};
use vpdf_model::style::Role;
const MIB: usize = 1 << 20;
use crate::layout::Screen;
use crate::panel::{self, Destination, Panel, Pointing, Row};
use std::io::Write;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;
use vpdf_model::document::{Facts, FactsRequest, Fit, LoadedFacts};
use vpdf_model::geometry::At;
use vpdf_terminal::graphics::Graphics;

/// Panels narrower than this show no card.
const NARROW: u16 = 60;
/// Thumbnail pixels kept across selections.
const BUDGET: usize = 16 * MIB;

#[derive(Clone)]
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
  facts: std::result::Result<Facts, String>,
}

/// Recent document cards and pending loads.
pub struct Shelf {
  entries: Vec<Entry>,
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
}

impl Shelf {
  pub fn new(entries: &[Entry]) -> Self {
    Self {
      entries: entries.to_vec(),
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
    }
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
      Pointing::Away => isize::MAX,
    };
    if panel.selected != self.selected {
      self.keyed = true;
    }
    // Resting at open is no move.
    if self.pointing.is_some_and(|before| before != pointing) {
      self.keyed = false;
    }
    (self.selected, self.pointing) = (panel.selected, Some(pointing));
    if self.keyed {
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

  pub fn take_request(&mut self) -> Option<FactsRequest> {
    self.pending.take()
  }

  pub fn accept(&mut self, path: PathBuf, loaded: LoadedFacts) {
    let LoadedFacts { fit, facts } = loaded;
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

  /// Columns of a `width` panel the list keeps, left of the card.
  pub fn list_columns(panel: &Panel, width: u16) -> u16 {
    width - Self::columns(panel, width)
  }

  /// Columns of a `width` panel the card takes; none without a file row.
  fn columns(panel: &Panel, width: u16) -> u16 {
    let files = panel
      .rows
      .iter()
      .any(|row| matches!(row.destination, Destination::File(_)));
    if width < NARROW || !files { 0 } else { width / 5 * 2 }
  }

  /// Draws `panel`'s list in `width` columns, the card on its right;
  /// `graphics` places the thumbnail.
  pub fn draw(
    &mut self,
    panel: &mut Panel,
    out: &mut impl Write,
    frame: Frame,
    graphics: Option<&mut Graphics>,
  ) -> Result<()> {
    let pictures = graphics.is_some();
    let mut scene = vpdf_terminal::painter::Painter::new(out);
    if let Some(graphics) = graphics {
      scene = scene.with_graphics(graphics);
    }
    let result = self.scene(panel, &mut scene, frame, pictures);
    scene.finish()?;
    result
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    panel: &mut Panel,
    scene: &mut S,
    frame: Frame,
    pictures: bool,
  ) -> std::result::Result<(), S::Error> {
    let Frame { at, width, screen, now } = frame;
    let list = Self::list_columns(panel, width);
    let card = width - list;
    panel.scene_list(scene, at, list, screen.rows);
    self.hide();
    let row = panel.rows.get(self.shown_row(panel));
    let Some(Destination::File(path)) = row.map(|row| &row.destination) else {
      return Ok(());
    };
    let Some(entry) = self.entries.iter().find(|e| e.path == *path).cloned()
    else {
      return Ok(());
    };
    if card < 2 {
      return Ok(());
    }
    if self.shown.as_ref() != Some(path) {
      self.shown = Some(path.clone());
      self.top = 0;
    }
    self.column = Some(list);
    let area = Area { at, column: list, columns: card, screen, now };
    self.card(scene, area, &entry, pictures)
  }

  fn card<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
    area: Area,
    entry: &Entry,
    pictures: bool,
  ) -> std::result::Result<(), S::Error> {
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
    let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
    if name.width() > inner {
      self.clipped.push((0, entry.path.display().to_string()));
    }
    crate::paint::move_to(scene, at, 0u16, column + 1);
    scene.push(Op::Style(Role::File));
    scene.push(Op::Text(vpdf_model::scene::Text::fit(&name, inner)));
    scene.push(Op::Style(Role::Reset));
    let picture = if pictures { rows.saturating_sub(1) / 2 } else { 0 };
    let (cell_width, cell_height) =
      (u32::from(screen.cell_width), u32::from(screen.cell_height));
    let fit = (picture >= 4 && inner >= 4)
      .then(|| (inner as u32 * cell_width, picture as u32 * cell_height));
    self.ask(entry, fit);
    let card = self.cards.get(&entry.path).filter(|card| card.fit == fit);
    let thumb = card.and_then(|card| card.facts.as_ref().ok()?.thumb.as_ref());
    if let (Some(thumb), true, Some((width, _))) = (thumb, pictures, fit) {
      let x = f32::from(column + 1) * cell_width as f32
        + (width as f32 - thumb.width() as f32) / 2.0;
      let y = cell_height as f32;
      scene.picture(card.unwrap().serial, thumb, (x, y), screen, 0)?;
    }
    // Text follows the thumbnail's own height; a loading card holds the
    // whole box so its text does not jump twice.
    let shown = match (thumb, card) {
      (Some(thumb), _) => Some((thumb.height() as u32).div_ceil(cell_height)),
      (None, None) => fit.map(|_| picture as u32),
      (None, Some(_)) => None,
    };
    let top = shown.map_or(2, |rows| rows as usize + 2);
    let texts = lines(entry, card, now);
    let lines: Vec<Row> = texts.iter().map(|line| Row::parse(line)).collect();
    let keys = panel::key_width(&lines, inner);
    let room = rows.saturating_sub(top);
    self.last_top = lines.len().saturating_sub(room);
    self.top = self.top.min(self.last_top);
    let shown = lines.iter().zip(&texts).skip(self.top).take(room);
    for (row, (line, text)) in (top..).zip(shown) {
      crate::paint::move_to(scene, at, row, column + 1);
      scene.push(Op::Style(Role::Reset));
      let used = line.scene(scene, Some(Role::Reset), inner, keys, "");
      let full = line.scene(&mut (), None, usize::MAX, keys, "");
      if full > used {
        self.clipped.push((row as u16, text.clone()));
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
      path: entry.path.clone(),
      cursor: entry.cursor,
      rotation: entry.rotation,
      fit,
    });
  }
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
  let page = entry.cursor.page.0 + 1;
  let mut lines = vec![
    "# Reading".to_owned(),
    match facts {
      Some(facts) => format!(
        "Page: {page} of {} ({}%)",
        facts.pages,
        page * 100 / facts.pages.max(1)
      ),
      None => format!("Page: {page}"),
    },
  ];
  if let Some(section) = facts.and_then(|facts| facts.section.as_deref()) {
    lines.push(format!("Section: {section}"));
  }
  let elapsed = now.saturating_sub(entry.used);
  lines.push(format!("Last read: {}", ago(elapsed)));
  if entry.marks > 0 {
    lines.push(format!("Marks: {}", entry.marks));
  }
  lines.push(String::new());
  match card.map(|card| &card.facts) {
    None => lines.push("Loading…".into()),
    Some(Err(error)) => lines.push(format!("Error: {error}")),
    Some(Ok(facts)) => {
      // The card's title already names the file.
      let parent = entry.path.parent().unwrap_or(&entry.path);
      let place = format!("Parent: {}", parent.display());
      lines.extend(panel::document_lines(place, facts.bytes, facts.pages));
      lines.push(String::new());
      lines.extend(panel::metadata_lines(&facts.metadata));
    }
  }
  lines
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
  use vpdf_model::input::{KeyModifiers, MouseEvent, MouseEventKind};
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
    let mut graphics = Graphics::default();
    let screen = Screen {
      columns: 100,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut draw = |panel: &mut Panel, screen: Screen| -> Result<String> {
      let mut out = Vec::new();
      panel.draw(
        &mut out,
        At::default(),
        screen.columns,
        screen,
        Some(&mut graphics),
        0,
      )?;
      Ok(String::from_utf8(out)?)
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
      LoadedFacts { fit: first.fit, facts: Err("Old path".into()) },
    );
    assert!(!draw(&mut panel, screen)?.contains("Old path"));
    assert!(panel.shelf_mut().unwrap().take_request().is_none());
    let resized = Screen { cell_width: 10, ..screen };
    draw(&mut panel, resized)?;
    let current = panel.shelf_mut().unwrap().take_request().unwrap();
    assert_ne!(current.fit, second.fit);
    panel.shelf_mut().unwrap().accept(
      second.path,
      LoadedFacts { fit: second.fit, facts: Err("Old fit".into()) },
    );
    let pending = draw(&mut panel, resized)?;
    assert!(pending.contains("Loading…") && !pending.contains("Old fit"));
    assert!(panel.shelf_mut().unwrap().take_request().is_none());
    panel.shelf_mut().unwrap().accept(
      current.path,
      LoadedFacts { fit: current.fit, facts: Err("Current error".into()) },
    );
    let ready = draw(&mut panel, resized)?;
    assert!(ready.contains("Current error") && !ready.contains("Loading…"));
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
    let screen = Screen {
      columns: 60,
      rows: 6,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
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
      let mut out = Vec::new();
      shelf.draw(
        panel,
        &mut out,
        Frame { at: At::default(), width: 60, screen, now: 0 },
        None,
      )?;
      Ok(String::from_utf8(out)?)
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
    let full = entries[1].path.display().to_string();
    assert_eq!(shelf.detail(0), Some(full));
    assert!(!draw(&mut shelf, &mut panel)?.contains("Loading"));
    // The wheel over the card scrolls it.
    panel.shelf = Some(shelf);
    panel.pointer(MouseEvent {
      kind: MouseEventKind::ScrollDown,
      column,
      row: 3,
      modifiers: KeyModifiers::NONE,
    });
    let mut shelf = panel.shelf.take().expect("shelf");
    assert_eq!(panel.top, 0);
    assert!(draw(&mut shelf, &mut panel)?.contains("Loading"));
    Ok(())
  }
}
