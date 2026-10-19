use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::figures::Source;
use crate::keys::{Action, Input};
use crate::pointer::Items;
use crate::split::Rect;
use crate::text::{Bounds, PageId, Text};
use crate::{clipboard, theme};
use std::fs::OpenOptions;
use std::io::Write;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use unicode_width::UnicodeWidthStr;
use vpdf_reader::reader::Yank;
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureId(u64);

impl CaptureId {
  pub(crate) fn next() -> Self {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Self(NEXT.fetch_add(1, Ordering::Relaxed))
  }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
  pub id: CaptureId,
  pub page: PageId,
  pub bounds: Bounds,
}

pub enum Region {
  Selected(Snapshot),
  Bar {
    snapshot: Snapshot,
    action: Operation,
    status: String,
    /// Where the last frame drew it.
    cells: Option<BarCells>,
  },
}

/// A drawn action bar's pane cells.
#[derive(Clone, Debug)]
pub struct BarCells {
  area: Rect,
  items: Items,
}

/// Bar label items: columns and `ACTIONS` index.
type Spans = Vec<(Range<usize>, usize)>;

impl Region {
  pub fn is_bar(&self) -> bool {
    matches!(self, Self::Bar { .. })
  }

  pub fn snapshot(&self) -> &Snapshot {
    match self {
      Self::Selected(snapshot) | Self::Bar { snapshot, .. } => snapshot,
    }
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
  Image,
  Text,
  Save,
  Ocr,
}

/// The open bar's status hint.
pub(crate) const BAR_HINT: &str = "h/l choose; Enter runs; Esc closes";

const ACTIONS: [(&str, Operation); 4] = [
  ("Copy image", Operation::Image),
  ("Copy text", Operation::Text),
  ("Save PNG", Operation::Save),
  ("OCR text", Operation::Ocr),
];

pub struct Transfer {
  pub id: CaptureId,
  pub png: Vec<u8>,
}

enum Outcome {
  Remote(Vec<u8>),
  Text(String),
  Message(String),
}

pub struct Job {
  id: CaptureId,
  cancel: Arc<AtomicBool>,
  result: mpsc::Receiver<std::result::Result<Outcome, String>>,
}

impl Drop for Job {
  fn drop(&mut self) {
    self.cancel.store(true, Ordering::Relaxed);
  }
}

impl App {
  pub fn move_figure(&mut self, forward: bool, count: usize) -> Result<()> {
    self.check_region();
    let current = self.figure_point()?;
    let mut remaining = count.max(1);
    let mut chosen = None;
    let pages = self.doc.pages.len();
    for offset in 0..pages {
      let page = if forward {
        current.0.0.checked_add(offset).filter(|p| *p < pages)
      } else {
        current.0.0.checked_sub(offset)
      };
      let Some(page) = page.map(PageId) else { break };
      let figures = self.doc.figures(page)?;
      for offset in 0..figures.len() {
        let index = if forward { offset } else { figures.len() - offset - 1 };
        let bounds = figures[index].bounds;
        let order = compare((page, bounds.y0, bounds.x0), current);
        if order
          != if forward {
            std::cmp::Ordering::Greater
          } else {
            std::cmp::Ordering::Less
          }
        {
          continue;
        }
        chosen = Some((page, bounds));
        remaining -= 1;
        if remaining == 0 {
          break;
        }
      }
      if remaining == 0 {
        break;
      }
    }
    let Some((page, bounds)) = chosen else {
      self.reader.message = "No more figures".into();
      return Ok(());
    };
    self.follow(&Target::Point { page, x: bounds.x0, y: bounds.y0 })?;
    if !self.text_mode {
      let rect = self.layout.project(page, bounds, 0.0);
      self.reader.scroll = self.layout.clamp_scroll(rect.y0);
    }
    self.region = Some(Region::Selected(snapshot(page, bounds)));
    Ok(())
  }

  /// The figure at a page point.
  pub(crate) fn figure_at(
    &mut self,
    page: PageId,
    x: f32,
    y: f32,
  ) -> Result<Option<crate::figures::Figure>> {
    let figures = self.doc.figures(page)?;
    let under = figures.iter().filter(|figure| figure.bounds.contains(x, y));
    // Graphics win over lone captions.
    let figure = under.min_by_key(|figure| figure.source != Source::Graphic);
    Ok(figure.cloned())
  }

  fn figure_point(&mut self) -> Result<(PageId, f32, f32)> {
    if let Some(region) = &self.region {
      let selected = region.snapshot();
      return Ok((selected.page, selected.bounds.y0, selected.bounds.x0));
    }
    let position = self.reading_position()?;
    let text = self.doc.text(position.page)?;
    if let Some(glyph) = text.glyph(position) {
      return Ok((position.page, glyph.bounds.y0, glyph.bounds.x0));
    }
    let page = self.navigation_position().page;
    let (x, y) = self
      .layout
      .unproject(page, (0.0, 0.0), self.reader.scroll)
      .unwrap_or((0.0, 0.0));
    Ok((page, y, x))
  }

  fn figure_snapshot(&mut self) -> Result<Snapshot> {
    self.check_region();
    if let Some(region) = &self.region {
      return Ok(region.snapshot().clone());
    }
    let (page, y, x) = self.figure_point()?;
    let figures = self.doc.figures(page)?;
    let figure = figures
      .iter()
      .min_by(|a, b| {
        distance(a.bounds, x, y).total_cmp(&distance(b.bounds, x, y))
      })
      .ok_or_else(|| anyhow::anyhow!("No figure on this page"))?;
    Ok(snapshot(page, figure.bounds))
  }

  fn check_region(&mut self) {
    let Some(Region::Selected(selected)) = &self.region else { return };
    let page = selected.page;
    let valid = self.doc.pages.get(selected.page.0).is_some_and(|page| {
      crate::layout::clip(selected.bounds, page.bounds).is_some()
    });
    let visible = if self.text_mode {
      let pages = self.text_pages();
      pages.is_ok_and(|pages| pages.contains(&page.0))
    } else if valid {
      let rect =
        self.layout.project(selected.page, selected.bounds, self.reader.scroll);
      rect.x1 > 0.0
        && rect.y1 > 0.0
        && rect.x0 < self.layout.screen.width()
        && rect.y0 < self.layout.screen.height()
    } else {
      false
    };
    if !valid || !visible {
      self.region = None;
    }
  }

  pub fn fit_figure(&mut self) -> Result<()> {
    anyhow::ensure!(!self.text_mode, "Figure fit requires page view");
    let snapshot = self.figure_snapshot()?;
    let rect = self.layout.project(snapshot.page, snapshot.bounds, 0.0);
    let screen = self.layout.screen;
    let margin = f32::from(screen.cell_height);
    self.layout.zoom *= ((screen.width() - margin).max(1.0) / rect.width())
      .min((screen.height() - margin * 2.0).max(1.0) / rect.height());
    self.layout.zoom = self.layout.zoom.clamp(0.1, 8.0);
    self.relayout()?;
    let rect = self.layout.project(snapshot.page, snapshot.bounds, 0.0);
    self.reader.scroll = self
      .layout
      .clamp_scroll(rect.y0 - (screen.height() - rect.height()) / 2.0);
    self.layout.pan = (self.layout.pan + rect.x0
      - (screen.width() - rect.width()) / 2.0)
      .clamp(0.0, self.layout.pan_limit(snapshot.page));
    self.animation = None;
    self.region = Some(Region::Selected(snapshot));
    Ok(())
  }

  pub fn open_region(&mut self) -> Result<()> {
    let selected = self.figure_snapshot()?;
    self.select_region(selected.page, selected.bounds)
  }

  pub fn select_region(&mut self, page: PageId, bounds: Bounds) -> Result<()> {
    self.doc.validate(page)?;
    let bounds = crate::layout::clip(bounds, self.doc.pages[page.0].bounds)
      .ok_or_else(|| anyhow::anyhow!("Empty capture region"))?;
    let snapshot = snapshot(page, bounds);
    self.peek = None;
    self.animation = None;
    self.disarm();
    self.region = Some(Region::Bar {
      snapshot,
      action: Operation::Image,
      status: String::new(),
      cells: None,
    });
    self.reader.message = BAR_HINT.into();
    Ok(())
  }

  pub(crate) fn region_input(&mut self, input: Input) -> Result<bool> {
    let Some(Region::Bar { action, .. }) = &mut self.region else {
      if !matches!(
        input.action,
        Action::Figure(_) | Action::FigureFit | Action::Region
      ) {
        self.region = None;
      }
      return Ok(false);
    };
    match input.action {
      Action::Motion(
        vpdf_reader::motion::Motion::Left | vpdf_reader::motion::Motion::Up,
      ) => {
        *action = ACTIONS[(*action as usize + 3) % ACTIONS.len()].1;
      }
      Action::Motion(
        vpdf_reader::motion::Motion::Right | vpdf_reader::motion::Motion::Down,
      ) => {
        *action = ACTIONS[(*action as usize + 1) % ACTIONS.len()].1;
      }
      Action::Follow => {
        let operation = *action;
        self.region_operation(operation)?;
      }
      Action::Quit | Action::Cancel | Action::Region => {
        self.dismiss_overlay();
        self.disarm();
      }
      Action::Command => return Ok(false),
      action if self.overlay_transfer(action) => return Ok(false),
      _ => {}
    }
    Ok(true)
  }

  pub fn region_operation(&mut self, operation: Operation) -> Result<()> {
    anyhow::ensure!(
      self.region_job.is_none() && self.region_send.is_none(),
      "Region operation running"
    );
    let mut selected = self
      .region
      .as_ref()
      .ok_or_else(|| anyhow::anyhow!("No selected region"))?
      .snapshot()
      .clone();
    selected.id = CaptureId::next();
    if let Some(Region::Bar { snapshot, .. }) = &mut self.region {
      snapshot.id = selected.id;
    }
    if matches!(operation, Operation::Text) {
      let source = self.doc.text(selected.page)?;
      let text = region_text(&source, selected.bounds);
      self.region_text(text)?;
      self.region_status(selected.id, "Text copied".into());
      return Ok(());
    }
    let path = self.path.clone();
    let password = self.password.clone();
    let rotation = self.layout.rotation;
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancel);
    let (send, result) = mpsc::channel();
    let send = self.wake.sender(send);
    let id = selected.id;
    std::thread::spawn(move || {
      let result = run(&path, &password, &selected, rotation, operation, &stop)
        .map_err(|error| error.to_string());
      send.send(result).ok();
    });
    self.region_job = Some(Job { id, cancel, result });
    self.region_status(id, "Region operation running".into());
    Ok(())
  }

  pub(crate) fn region_updates(&mut self) -> Result<bool> {
    let Some(job) = &self.region_job else { return Ok(false) };
    let result = match job.result.try_recv() {
      Ok(result) => result,
      Err(mpsc::TryRecvError::Empty) => return Ok(false),
      Err(mpsc::TryRecvError::Disconnected) => {
        Err("Region worker disconnected".into())
      }
    };
    let id = job.id;
    self.region_job = None;
    match result {
      Ok(Outcome::Remote(png)) => {
        self.region_send = Some(Transfer { id, png });
      }
      Ok(Outcome::Text(text)) => {
        self.region_text(text)?;
        self.region_status(id, "OCR text copied".into());
      }
      Ok(Outcome::Message(message)) => self.region_status(id, message),
      Err(error) => self.region_status(id, error),
    }
    Ok(true)
  }

  fn region_text(&mut self, text: String) -> Result<()> {
    anyhow::ensure!(!text.trim().is_empty(), "No text in region");
    self.reader.pending_yank = Some(Yank { text, register: '"' });
    Ok(())
  }

  pub(crate) fn region_status(&mut self, id: CaptureId, message: String) {
    match &mut self.region {
      Some(Region::Bar { snapshot, status, .. }) if snapshot.id == id => {
        status.clone_from(&message);
      }
      Some(Region::Bar { .. }) => return,
      _ => {}
    }
    self.reader.message = message;
  }

  pub fn draw_region(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    if let crate::gestures::Gesture::Sweep(sweep) = self.gesture
      && let Some(bounds) = sweep.bounds()
    {
      let mut rect =
        self.layout.project(sweep.page, bounds, self.reader.scroll);
      // Quirk overlays cache by size.
      if graphics.crop_quirk() {
        let (width, height) = (
          f32::from(self.layout.screen.cell_width),
          f32::from(self.layout.screen.cell_height),
        );
        rect = Bounds {
          x0: (rect.x0 / width).floor() * width,
          y0: (rect.y0 / height).floor() * height,
          x1: (rect.x1 / width).ceil() * width,
          y1: (rect.y1 / height).ceil() * height,
        };
      }
      self.frame_capture(graphics, rect, out)?;
    }
    let Some(Region::Bar { snapshot, action, status, .. }) = &self.region
    else {
      return Ok(());
    };
    let (page, action, status) = (snapshot.page, *action, status.clone());
    let rect = self.layout.project(page, snapshot.bounds, self.reader.scroll);
    self.frame_capture(graphics, rect, out)?;
    let screen = self.layout.screen;
    let (mut label, mut items) = bar_label(page, action);
    if label.chars().count() > usize::from(screen.columns) {
      let name = ACTIONS[action as usize].0;
      label = format!("[{name}] p{}", page.0 + 1);
      items = vec![(0..name.len() + 2, action as usize)];
    }
    if !status.is_empty() {
      label.push_str(" │ ");
      label.push_str(&status);
    }
    let text = terminal::clean(&label, usize::from(screen.columns));
    let last = usize::from(screen.rows.saturating_sub(2));
    let row = bar_row(rect, f32::from(screen.cell_height), last);
    // Graphics fill the row; text stops.
    let width =
      if self.text_mode { text.width() } else { usize::from(screen.columns) };
    self.place_bar(row, width, items);
    if let Some(cells) = self.bar_cells().filter(|_| !self.text_mode) {
      let backdrop = self.cell_bounds(cells);
      graphics.backdrop(backdrop, theme::BADGE_FILL, screen, self.left, out)?;
    }
    write!(
      out,
      "{}{}{}{}",
      self.at.goto(row, usize::from(self.left)),
      theme::SELECTED,
      text,
      theme::RESET
    )?;
    Ok(())
  }

  /// Records the drawn bar's cells.
  fn place_bar(&mut self, row: usize, width: usize, spans: Spans) {
    let column = |offset: usize| {
      u16::try_from(usize::from(self.left) + offset.min(width))
        .unwrap_or(u16::MAX)
    };
    let row = u16::try_from(row).unwrap_or(u16::MAX);
    let cells = |span: Range<usize>| {
      let (start, end) = (column(span.start), column(span.end));
      Rect { column: start, row, columns: end - start, rows: 1 }
    };
    let mut items = Items::default();
    for (span, index) in spans {
      items.push(cells(span), index);
    }
    let area = cells(0..width);
    if let Some(Region::Bar { cells, .. }) = &mut self.region {
      *cells = Some(BarCells { area, items });
    }
  }

  fn drawn_bar(&self) -> Option<&BarCells> {
    match &self.region {
      Some(Region::Bar { cells, .. }) => cells.as_ref(),
      _ => None,
    }
  }

  /// Pane cells the drawn bar covers.
  pub fn bar_cells(&self) -> Option<Rect> {
    self.drawn_bar().map(|cells| cells.area)
  }

  pub(crate) fn bar_items(&self) -> Items {
    self.drawn_bar().map_or_default(|cells| cells.items.clone())
  }

  /// Selects a bar item by `ACTIONS` index.
  pub(crate) fn pick_action(&mut self, index: usize) {
    if let Some(Region::Bar { action, .. }) = &mut self.region
      && let Some((_, operation)) = ACTIONS.get(index)
    {
      *action = *operation;
    }
  }

  pub(crate) fn run_action(&mut self, index: usize) -> Result<()> {
    self.pick_action(index);
    let Some((_, operation)) = ACTIONS.get(index) else { return Ok(()) };
    self.region_operation(*operation)
  }

  /// Dims around a rect; outlines it.
  fn frame_capture(
    &mut self,
    graphics: &mut Graphics,
    rect: Bounds,
    out: &mut impl Write,
  ) -> Result<()> {
    let screen = self.layout.screen;
    if !self.text_mode {
      let w = screen.width();
      let h = screen.height();
      let x0 = rect.x0.clamp(0.0, w);
      let x1 = rect.x1.clamp(x0, w);
      let y0 = rect.y0.clamp(0.0, h);
      let y1 = rect.y1.clamp(y0, h);
      for bounds in [
        Bounds { x0: 0.0, y0: 0.0, x1: w, y1: y0 },
        Bounds { x0: 0.0, y0: y1, x1: w, y1: h },
        Bounds { x0: 0.0, y0, x1: x0, y1 },
        Bounds { x0: x1, y0, x1: w, y1 },
      ] {
        graphics.backdrop(bounds, [0, 0, 0, 100], screen, self.left, out)?;
      }
      for bounds in frame_edges(Bounds { x0, y0, x1, y1 }, (w, h)) {
        graphics.highlight(
          bounds,
          [60, 120, 230, 220],
          screen,
          self.left,
          out,
        )?;
      }
    }
    Ok(())
  }
}

/// Bar text and item spans; `action` bracketed.
fn bar_label(page: PageId, action: Operation) -> (String, Spans) {
  let mut label = format!("p{}", page.0 + 1);
  let mut items = Vec::with_capacity(ACTIONS.len());
  for (index, (name, operation)) in ACTIONS.iter().enumerate() {
    label.push_str(if index == 0 { " " } else { " · " });
    let start = label.chars().count();
    if *operation == action {
      label.extend(["[", name, "]"]);
    } else {
      label.push_str(name);
    }
    items.push((start..label.chars().count(), index));
  }
  (label, items)
}

/// A 2 px frame within `(w, h)`.
fn frame_edges(rect: Bounds, (w, h): (f32, f32)) -> [Bounds; 4] {
  // Overlays round out to whole pixels.
  let (x0, y0) = (rect.x0.floor(), rect.y0.floor());
  let (x1, y1) = (rect.x1.ceil(), rect.y1.ceil());
  // U26: outside where room allows.
  let top = band(y0, -1.0, y0 >= 2.0);
  let bottom = band(y1, 1.0, y1 + 2.0 <= h);
  let left = band(x0, -1.0, x0 >= 2.0);
  let right = band(x1, 1.0, x1 + 2.0 <= w);
  [
    Bounds { x0: left.0, x1: right.1, y0: top.0, y1: top.1 },
    Bounds { x0: left.0, x1: right.1, y0: bottom.0, y1: bottom.1 },
    Bounds { x0: left.0, x1: left.1, y0, y1 },
    Bounds { x0: right.0, x1: right.1, y0, y1 },
  ]
}

/// A 2 px band, outward if room.
fn band(edge: f32, dir: f32, fits: bool) -> (f32, f32) {
  let far = if fits { edge + 2.0 * dir } else { edge - 2.0 * dir };
  (edge.min(far), edge.max(far))
}

/// The bar's row: below the frame, else above.
fn bar_row(rect: Bounds, cell: f32, last: usize) -> usize {
  let below = ((rect.y1.ceil() + 2.0) / cell).ceil() as usize;
  let above = ((rect.y0.floor() - 2.0) / cell).floor() as isize - 1;
  match usize::try_from(above) {
    _ if below <= last => below,
    Ok(above) => above,
    Err(_) => last,
  }
}

fn snapshot(page: PageId, bounds: Bounds) -> Snapshot {
  Snapshot { id: CaptureId::next(), page, bounds }
}

fn compare(a: (PageId, f32, f32), b: (PageId, f32, f32)) -> std::cmp::Ordering {
  a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2))
}

fn distance(bounds: Bounds, x: f32, y: f32) -> f32 {
  let dx = (bounds.x0 - x).max(x - bounds.x1).max(0.0);
  let dy = (bounds.y0 - y).max(y - bounds.y1).max(0.0);
  dx * dx + dy * dy
}

pub fn region_text(text: &Text, bounds: Bounds) -> String {
  text
    .lines
    .iter()
    .map(|line| {
      line
        .glyphs
        .iter()
        .filter(|glyph| {
          let (x, y) = glyph.bounds.centre();
          bounds.contains(x, y)
        })
        .map(|glyph| glyph.ch)
        .collect::<String>()
    })
    .filter(|line| !line.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn run(
  path: &Path,
  password: &str,
  region: &Snapshot,
  rotation: u16,
  operation: Operation,
  cancel: &AtomicBool,
) -> Result<Outcome> {
  let capture = vpdf_pdf::jobs::capture::capture(
    path,
    password,
    region.page,
    region.bounds,
    rotation,
    cancel,
  )?;
  match operation {
    Operation::Image if clipboard::remote() => Ok(Outcome::Remote(capture.png)),
    Operation::Image => {
      clipboard::image_native(capture.png)?;
      Ok(Outcome::Message("Image copied".into()))
    }
    Operation::Save => {
      let saved = save(path, region.page, &capture.png)?;
      Ok(Outcome::Message(format!("Saved {}", saved.display())))
    }
    Operation::Ocr => Ok(Outcome::Text(vpdf_pdf::jobs::capture::recognize(
      capture.png,
      "eng",
      cancel,
    )?)),
    Operation::Text => unreachable!(),
  }
}

fn save(path: &Path, page: PageId, png: &[u8]) -> Result<PathBuf> {
  let stem = path.file_stem().unwrap_or_default().to_string_lossy();
  for index in 1..=1000 {
    let name = format!("{stem}-p{}-region-{index}.png", page.0 + 1);
    let path = path.with_file_name(name);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
      Ok(mut file) => {
        if let Err(error) = file.write_all(png) {
          let _ = std::fs::remove_file(&path);
          return Err(error.into());
        }
        return Ok(path);
      }
      Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
      Err(error) => return Err(error.into()),
    }
  }
  anyhow::bail!("Region PNG filename unavailable")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Glyph, Line};

  #[test]
  fn results_wake_the_loop() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("region.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let screen = crate::layout::Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&path, "", true, screen)?;
    let (wake, woken) = crate::fixtures::wake();
    app.wake = wake;
    app.select_region(
      PageId(0),
      Bounds { x0: 10.0, y0: 10.0, x1: 50.0, y1: 50.0 },
    )?;
    app.region_operation(Operation::Save)?;
    woken.recv_timeout(std::time::Duration::from_secs(10))?;
    let result = app.region_job.as_ref().unwrap().result.try_recv()?;
    assert!(matches!(result, Ok(Outcome::Message(_))));
    let png = std::fs::read(dir.path().join("region-p1-region-1.png"))?;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(woken.try_iter().count(), 0);
    Ok(())
  }

  #[test]
  fn dropping_cancels_region() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("region.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let region = Snapshot {
      id: CaptureId::next(),
      page: PageId(0),
      bounds: Bounds { x0: 10.0, y0: 10.0, x1: 50.0, y1: 50.0 },
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let (_send, result) = mpsc::channel();
    let job = Job { id: region.id, cancel: Arc::clone(&cancel), result };
    drop(job);
    assert!(cancel.load(Ordering::Relaxed));
    let result = run(&path, "", &region, 0, Operation::Save, &cancel);
    assert_eq!(result.err().unwrap().to_string(), "Region operation cancelled");
    assert!(!dir.path().join("region-p1-region-1.png").exists());
    Ok(())
  }

  #[test]
  fn text_centers_preserve_order() {
    let text = Text {
      lines: ["abcd", "efgh"]
        .into_iter()
        .enumerate()
        .map(|(line, value)| Line {
          paragraph: true,
          glyphs: value
            .chars()
            .enumerate()
            .map(|(index, ch)| Glyph {
              ch,
              bounds: Bounds {
                x0: index as f32 * 10.0,
                x1: index as f32 * 10.0 + 10.0,
                y0: line as f32 * 10.0,
                y1: line as f32 * 10.0 + 10.0,
              },
            })
            .collect(),
        })
        .collect(),
      truncated: false,
    };
    let bounds = Bounds { x0: 10.0, y0: 0.0, x1: 30.0, y1: 20.0 };
    assert_eq!(region_text(&text, bounds), "bc\nfg");
    assert_eq!(region_text(&text, Bounds::default()), "");
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn region_job_survives_focus() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("paper.pdf");
    std::fs::copy(root.join("paper.pdf"), &path)?;
    let screen = crate::layout::Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&path, "", false, screen)?;
    app.move_figure(true, 1)?;
    app.open_region()?;
    let page = app.region.as_ref().unwrap().snapshot().page;
    app.region_operation(Operation::Save)?;
    let original = app.region_job.as_ref().unwrap().id;
    app.blur()?;
    assert!(app.region.is_none());
    assert_eq!(app.region_job.as_ref().unwrap().id, original);
    app.focused = false;
    app.reader.cursor.page = PageId(app.doc.pages.len() - 1);
    let start = std::time::Instant::now();
    while app.region_job.is_some() {
      app.region_updates()?;
      anyhow::ensure!(start.elapsed().as_secs() < 10, "Region save timeout");
      std::thread::yield_now();
    }
    let name = format!("paper-p{}-region-1.png", page.0 + 1);
    let png = std::fs::read(directory.path().join(name))?;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(app.reader.message.starts_with("Saved "));
    Ok(())
  }

  /// U26: frames sit outside when room allows.
  #[test]
  fn frames_sit_outside_captures() {
    let (w, h) = (100.0, 50.0);
    let rect = |x0, y0, x1, y1| Bounds { x0, y0, x1, y1 };
    // (capture, inside: top, bottom, left, right)
    for (capture, inside) in [
      (rect(10.0, 10.0, 60.0, 40.0), [false; 4]),
      (rect(10.5, 10.5, 60.5, 40.5), [false; 4]),
      (rect(0.0, 0.0, 100.0, 50.0), [true; 4]),
      (rect(1.0, 20.0, 99.5, 50.0), [false, true, true, true]),
    ] {
      for (edge, inside) in frame_edges(capture, (w, h)).into_iter().zip(inside)
      {
        let covers = edge.x0.max(capture.x0) < edge.x1.min(capture.x1)
          && edge.y0.max(capture.y0) < edge.y1.min(capture.y1);
        assert_eq!(covers, inside, "{edge:?} on {capture:?}");
        assert_eq!(edge.width().min(edge.height()), 2.0, "{edge:?}");
        let within = edge.x0 >= 0.0 && edge.y0 >= 0.0;
        assert!(within && edge.x1 <= w && edge.y1 <= h, "{edge:?}");
        // Bands touch the whole-pixel capture.
        let touches = edge.x0 <= capture.x1.ceil()
          && edge.x1 >= capture.x0.floor()
          && edge.y0 <= capture.y1.ceil()
          && edge.y1 >= capture.y0.floor();
        assert!(touches, "{edge:?} off {capture:?}");
      }
    }
  }

  /// The bar never covers the capture.
  #[test]
  fn bars_clear_captures() {
    let rect = |y0, y1| Bounds { x0: 0.0, y0, x1: 10.0, y1 };
    // (capture, row); 20 px cells, last 38
    for (capture, row) in [
      (rect(100.0, 300.0), 16),
      (rect(100.0, 770.0), 3),
      (rect(10.0, 770.0), 38),
      (rect(100.0, 736.0), 37),
    ] {
      assert_eq!(bar_row(capture, 20.0, 38), row, "{capture:?}");
    }
  }

  #[test]
  fn saving_preserves_existing_files() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let document = directory.path().join("paper.pdf");
    let first = save(&document, PageId(2), b"first")?;
    let second = save(&document, PageId(2), b"second")?;
    assert_ne!(first, second);
    assert_eq!(std::fs::read(first)?, b"first");
    assert_eq!(std::fs::read(second)?, b"second");
    Ok(())
  }
}
