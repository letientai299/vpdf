//! Recent documents beside a card for the selected one: its first page
//! and `:info` facts, read off the input thread.

use crate::Result;
use crate::cache::Cache;
use crate::document::{Handle, MIB};
use crate::graphics::Graphics;
use crate::layout::{Screen, page_matrix};
use crate::panel::{self, Destination, Panel, Row};
use crate::state::Entry;
use crate::terminal::{self, At};
use crate::text::{Bounds, PageId};
use crate::theme;
use mupdf::IRect;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, mpsc};

/// Panels narrower than this show no card.
const NARROW: u16 = 60;
/// Thumbnail pixels kept across selections.
const BUDGET: usize = 16 * MIB;

/// Pixel box a thumbnail fits; none without graphics.
type Fit = Option<(u32, u32)>;

/// The first page, fit to the card.
struct Thumb {
  serial: u64,
  width: i32,
  height: i32,
  rgb: Vec<u8>,
}

/// What a worker reads from a recent document.
struct Facts {
  pages: usize,
  bytes: u64,
  metadata: Vec<(String, String)>,
  section: Option<String>,
  thumb: Option<Thumb>,
}

struct Card {
  fit: Fit,
  facts: std::result::Result<Facts, String>,
}

struct Ask {
  entry: Entry,
  fit: Fit,
}

/// Recent documents and their cards; dropping it ends the worker.
pub struct Shelf {
  entries: Vec<Entry>,
  cards: Cache<PathBuf, Card>,
  /// The last load sent, so a frame asks once.
  asked: Option<(PathBuf, Fit)>,
  asks: mpsc::Sender<Ask>,
  loaded: mpsc::Receiver<(PathBuf, Card)>,
}

impl Shelf {
  pub fn new(entries: &[Entry], password: &str) -> Self {
    let (asks, inbox) = mpsc::channel();
    let (send, loaded) = mpsc::channel();
    let password = password.to_owned();
    std::thread::spawn(move || work(&password, &inbox, &send));
    Self {
      entries: entries.to_vec(),
      cards: Cache::new(BUDGET),
      asked: None,
      asks,
      loaded,
    }
  }

  /// Keeps finished loads; true when any arrived.
  pub fn updates(&mut self) -> bool {
    let mut arrived = false;
    while let Ok((path, card)) = self.loaded.try_recv() {
      let answers =
        |(asked, fit): &(PathBuf, Fit)| *asked == path && *fit == card.fit;
      if self.asked.as_ref().is_some_and(answers) {
        self.asked = None;
      }
      let thumb = card.facts.as_ref().ok().and_then(|f| f.thumb.as_ref());
      let weight = thumb.map_or(1, |thumb| thumb.rgb.len());
      self.cards.insert(path, card, weight);
      arrived = true;
    }
    arrived
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
    at: At,
    width: u16,
    screen: Screen,
    graphics: Option<&mut Graphics>,
  ) -> Result<()> {
    let card = Self::columns(panel, width);
    let list = width - card;
    panel.list(out, at, list, screen.rows)?;
    let row = panel.rows.get(panel.selected).map(|row| &row.destination);
    let Some(Destination::File(path)) = row else { return Ok(()) };
    let Some(entry) = self.entries.iter().find(|e| e.path == *path).cloned()
    else {
      return Ok(());
    };
    if card < 2 {
      return Ok(());
    }
    let area = Area { at, column: list, columns: card, screen };
    self.card(out, area, &entry, graphics)
  }

  fn card(
    &mut self,
    out: &mut impl Write,
    area: Area,
    entry: &Entry,
    graphics: Option<&mut Graphics>,
  ) -> Result<()> {
    let Area { at, column, columns, screen } = area;
    let rows = usize::from(screen.rows.saturating_sub(1));
    for row in 0..rows {
      let border = at.goto(row, column + columns - 1);
      write!(out, "{border}{}│{}", theme::MUTED, theme::RESET)?;
    }
    // One column of margin on each side of the text.
    let inner = usize::from(columns).saturating_sub(3);
    let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
    write!(
      out,
      "{}{}{}{}",
      at.goto(0u16, column + 1),
      theme::FILE,
      terminal::clean(&name, inner),
      theme::RESET
    )?;
    let picture =
      if graphics.is_some() { rows.saturating_sub(1) / 2 } else { 0 };
    let (cell_width, cell_height) =
      (u32::from(screen.cell_width), u32::from(screen.cell_height));
    let fit = (picture >= 4 && inner >= 4)
      .then(|| (inner as u32 * cell_width, picture as u32 * cell_height));
    self.ask(entry, fit);
    let card = self.cards.get(&entry.path).filter(|card| card.fit == fit);
    let thumb = card.and_then(|card| card.facts.as_ref().ok()?.thumb.as_ref());
    if let (Some(thumb), Some(graphics), Some((width, _))) =
      (thumb, graphics, fit)
    {
      let x = f32::from(column + 1) * cell_width as f32
        + (width as f32 - thumb.width as f32) / 2.0;
      let y = cell_height as f32;
      let rect = Bounds {
        x0: x,
        y0: y,
        x1: x + thumb.width as f32,
        y1: y + thumb.height as f32,
      };
      let image = (thumb.width, thumb.height, thumb.rgb.as_slice());
      graphics.picture(thumb.serial, image, rect, screen, 0, out)?;
    }
    // Text follows the thumbnail's own height; a loading card holds the
    // whole box so its text does not jump twice.
    let shown = match (thumb, card) {
      (Some(thumb), _) => Some((thumb.height as u32).div_ceil(cell_height)),
      (None, None) => fit.map(|_| picture as u32),
      (None, Some(_)) => None,
    };
    let top = shown.map_or(2, |rows| rows as usize + 2);
    let lines: Vec<Row> =
      lines(entry, card).iter().map(|line| Row::parse(line)).collect();
    let keys = panel::key_width(&lines, inner);
    for (index, line) in lines.iter().take(rows.saturating_sub(top)).enumerate()
    {
      write!(out, "{}{}", at.goto(top + index, column + 1), theme::RESET)?;
      line.write(out, theme::RESET, inner, keys, "")?;
    }
    write!(out, "{}", theme::RESET)?;
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
    let _ = self.asks.send(Ask { entry: entry.clone(), fit });
  }
}

/// The card's place in a pane.
#[derive(Clone, Copy)]
struct Area {
  at: At,
  column: u16,
  columns: u16,
  screen: Screen,
}

/// Loads each newest ask until the shelf drops.
fn work(
  password: &str,
  inbox: &mpsc::Receiver<Ask>,
  send: &mpsc::Sender<(PathBuf, Card)>,
) {
  while let Ok(mut ask) = inbox.recv() {
    // A selection moved past is not worth reading.
    while let Ok(newer) = inbox.try_recv() {
      ask = newer;
    }
    let facts = read(&ask, password).map_err(|error| error.to_string());
    let card = Card { fit: ask.fit, facts };
    if crate::input::notify(send.send((ask.entry.path, card))).is_err() {
      return;
    }
  }
}

fn read(ask: &Ask, password: &str) -> Result<Facts> {
  let entry = &ask.entry;
  let handle = Handle::open(&entry.path, password)?;
  let thumb =
    ask.fit.map(|fit| thumbnail(&handle, fit, entry.rotation)).transpose()?;
  let outline = handle.outline().unwrap_or_default();
  let cursor = entry.cursor;
  let section = if outline.is_empty() {
    None
  } else {
    let y = handle
      .text(cursor.page)
      .ok()
      .and_then(|text| text.glyph(cursor).map(|g| g.bounds.y1))
      .unwrap_or(f32::MAX);
    panel::section(&outline, cursor.page, y)
      .map(|index| outline[index].title.clone())
  };
  Ok(Facts {
    pages: handle.pages,
    bytes: std::fs::metadata(&entry.path)?.len(),
    metadata: handle.metadata(),
    section,
    thumb,
  })
}

/// The first page at the largest scale that fits `fit` pixels.
fn thumbnail(
  handle: &Handle,
  (width, height): (u32, u32),
  rotation: u16,
) -> Result<Thumb> {
  let page = PageId(0);
  let bounds = handle.bounds(page)?;
  let (w, h) = if rotation % 180 == 0 {
    (bounds.width(), bounds.height())
  } else {
    (bounds.height(), bounds.width())
  };
  anyhow::ensure!(w >= 1.0 && h >= 1.0, "Empty first page");
  let fit = (width as f32 / w).min(height as f32 / h);
  // `page_matrix` takes 1024ths.
  let scale = (fit * 1024.0) as u32;
  let size = |side: f32| (side * scale as f32 / 1024.0) as i32;
  let (pixels_wide, pixels_high) = (size(w), size(h));
  anyhow::ensure!(pixels_wide > 0 && pixels_high > 0, "Thumbnail too small");
  // A list read once needs no cache.
  let lists = Mutex::new(Cache::new(0));
  let list = handle.display(page, &lists)?;
  let matrix = page_matrix(bounds, scale, rotation);
  let area = IRect::new(0, 0, pixels_wide, pixels_high);
  let pixmap = crate::tiles::draw(&list, &matrix, area)?;
  Ok(Thumb {
    serial: crate::graphics::serial(),
    width: pixels_wide,
    height: pixels_high,
    rgb: pixmap.samples().to_vec(),
  })
}

/// Reading state first, then `:info`'s document facts.
fn lines(entry: &Entry, card: Option<&Card>) -> Vec<String> {
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
  let elapsed = crate::state::now().saturating_sub(entry.used);
  lines.push(format!("Last read: {}", ago(elapsed)));
  if !entry.marks.is_empty() {
    lines.push(format!("Marks: {}", entry.marks.len()));
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
  use std::time::{Duration, Instant};

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
  #[ignore = "requires verified public PDF corpus"]
  fn cards_show_facts_and_the_first_page() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let file = root.join("paper.pdf");
    let entry = crate::state::tests::entry(&file.to_string_lossy(), 1, 0);
    let entries = std::slice::from_ref(&entry);
    let screen = Screen {
      columns: 100,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut shelf = Shelf::new(entries, "");
    let mut panel = Panel::new(PanelKind::Recent, panel::recent_rows(entries));
    let mut graphics = Graphics::default();
    let mut draw = |shelf: &mut Shelf| -> Result<String> {
      let mut out = Vec::new();
      let graphics = Some(&mut graphics);
      shelf.draw(&mut panel, &mut out, At::default(), 100, screen, graphics)?;
      Ok(String::from_utf8(out)?)
    };
    assert!(draw(&mut shelf)?.contains("Loading…"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !shelf.updates() {
      anyhow::ensure!(Instant::now() < deadline, "Card never loaded");
      std::thread::sleep(Duration::from_millis(5));
    }
    let frame = draw(&mut shelf)?;
    for want in ["Page", "of", "Parent", "Pages", "Size", "Metadata", "a=t,"] {
      assert!(frame.contains(want), "{want}");
    }
    // A drawn card asks nothing more.
    draw(&mut shelf)?;
    assert!(shelf.loaded.recv_timeout(Duration::from_millis(200)).is_err());
    Ok(())
  }
}
