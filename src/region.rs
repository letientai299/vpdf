use crate::Result;
use crate::app::App;
use crate::document::{Document, Target};
use crate::keys::{Action, Input};
use crate::reader::Yank;
use crate::text::{Bounds, PageId, Text};
use crate::{clipboard, terminal, theme, tiles};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

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
  Bar { snapshot: Snapshot, action: Operation, status: String },
}

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
    let valid = self.doc.pages.get(selected.page.0).is_some_and(|page| {
      crate::layout::clip(selected.bounds, page.bounds).is_some()
    });
    let visible = if self.text_mode {
      selected.page == self.navigation_position().page
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
    });
    self.reader.message = "h/l choose; Enter runs; Esc closes".into();
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
        crate::motion::Motion::Left | crate::motion::Motion::Up,
      ) => {
        *action = ACTIONS[(*action as usize + 3) % ACTIONS.len()].1;
      }
      Action::Motion(
        crate::motion::Motion::Right | crate::motion::Motion::Down,
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
    let id = selected.id;
    std::thread::spawn(move || {
      let result = run(&path, &password, &selected, rotation, operation, &stop)
        .map_err(|error| error.to_string());
      crate::input::notify(send.send(result)).ok();
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

  pub fn draw_region(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(Region::Bar { snapshot, action, status }) = &self.region else {
      return Ok(());
    };
    let screen = self.layout.screen;
    let rect =
      self.layout.project(snapshot.page, snapshot.bounds, self.reader.scroll);
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
        self.graphics.backdrop(
          bounds,
          [0, 0, 0, 100],
          screen,
          self.left,
          out,
        )?;
      }
      for bounds in [
        Bounds { y1: y0 + 2.0, x0, x1, y0 },
        Bounds { y0: y1 - 2.0, x0, x1, y1 },
        Bounds { x1: x0 + 2.0, x0, y0, y1 },
        Bounds { x0: x1 - 2.0, x1, y0, y1 },
      ] {
        self.graphics.highlight(
          bounds,
          [60, 120, 230, 220],
          screen,
          self.left,
          out,
        )?;
      }
    }
    let options = ACTIONS
      .iter()
      .enumerate()
      .map(|(index, (label, _))| {
        if index == *action as usize {
          format!("[{label}]")
        } else {
          (*label).into()
        }
      })
      .collect::<Vec<_>>()
      .join(" · ");
    let mut label = format!("p{} {options}", snapshot.page.0 + 1);
    if label.chars().count() > usize::from(screen.columns) {
      label =
        format!("[{}] p{}", ACTIONS[*action as usize].0, snapshot.page.0 + 1);
    }
    if !status.is_empty() {
      label.push_str(" │ ");
      label.push_str(status);
    }
    let text = terminal::clean(&label, usize::from(screen.columns));
    let row = ((rect.y1 / f32::from(screen.cell_height)).ceil() as usize)
      .min(usize::from(screen.rows.saturating_sub(2)));
    if !self.text_mode {
      let y = row as f32 * f32::from(screen.cell_height);
      let backdrop = Bounds {
        x0: 0.0,
        y0: y,
        x1: screen.width(),
        y1: y + f32::from(screen.cell_height),
      };
      self.graphics.backdrop(
        backdrop,
        [60, 120, 230, 255],
        screen,
        self.left,
        out,
      )?;
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
  let mut doc = Document::open(path, password)?;
  let capture = tiles::capture(&mut doc, region.page, region.bounds, rotation)?;
  anyhow::ensure!(
    !cancel.load(Ordering::Relaxed),
    "Region operation cancelled"
  );
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
    Operation::Ocr => {
      let output = crate::ocr::tesseract(capture.png, "eng", cancel)?;
      let text = crate::ocr::parse_tsv(&output, Bounds::default(), 1.0)?;
      Ok(Outcome::Text(
        text
          .lines
          .iter()
          .map(crate::text::Line::text)
          .collect::<Vec<_>>()
          .join("\n"),
      ))
    }
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
