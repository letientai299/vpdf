use crate::Result;
#[cfg(test)]
use crate::SeedReader;
use crate::app::App;
use crate::clipboard;
use crate::pdf_text::PdfText;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use vpdf_model::document::Target;
use vpdf_model::text::{Bounds, PageId};
use vpdf_pdf::jobs::capture;
use vpdf_reader::reader::Yank;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::figures;
use vpdf_ui::keys::Input;
use vpdf_ui::page::Point;
use vpdf_ui::region::{
  BAR_HINT, CaptureId, Operation, Region, Snapshot, Transition,
};

pub(crate) fn next_capture() -> CaptureId {
  static NEXT: AtomicU64 = AtomicU64::new(1);
  CaptureId::new(NEXT.fetch_add(1, Ordering::Relaxed))
}

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
  work: capture::Job<Outcome>,
}

impl App {
  pub fn move_figure(&mut self, forward: bool, count: usize) -> Result<()> {
    self.check_region();
    let current = self.figure_point()?;
    let chosen =
      figures::advance(&mut PdfText(&mut self.doc), current, forward, count)?;
    let Some((page, bounds)) = chosen else {
      self.reader.notice("No more figures");
      return Ok(());
    };
    self.follow(&Target::Point { page, x: bounds.x0, y: bounds.y0 })?;
    self.reading_view().align_region(page, bounds);
    self.overlays.open_region(Region::selected(snapshot(page, bounds)));
    Ok(())
  }

  fn figure_point(&mut self) -> Result<Point> {
    let mut reading = self.reading_parts();
    reading.view.figure_point(reading.overlays.region())
  }

  fn figure_snapshot(&mut self) -> Result<Snapshot> {
    self.check_region();
    if let Some(region) = self.overlays.region() {
      return Ok(region.snapshot().clone());
    }
    let point = self.figure_point()?;
    let bounds = figures::nearest(&mut PdfText(&mut self.doc), point)?
      .ok_or_else(|| anyhow::anyhow!("No figure on this page"))?;
    Ok(snapshot(point.page, bounds))
  }

  fn check_region(&mut self) {
    let mut reading = self.reading_parts();
    reading.view.check_region(reading.overlays);
  }

  pub fn fit_figure(&mut self) -> Result<()> {
    anyhow::ensure!(!self.text_mode, "Figure fit requires page view");
    let snapshot = self.figure_snapshot()?;
    let fit = self.layout.fit_region(snapshot.page, snapshot.bounds);
    self.relayout()?;
    self.reading_view().finish_fit(fit);
    self.overlays.open_region(Region::selected(snapshot));
    Ok(())
  }

  pub fn open_region(&mut self) -> Result<()> {
    let selected = self.figure_snapshot()?;
    self.select_region(selected.page, selected.bounds)
  }

  pub fn select_region(&mut self, page: PageId, bounds: Bounds) -> Result<()> {
    self.doc.validate(page)?;
    let bounds = vpdf_ui::layout::clip(bounds, self.doc.pages[page.0].bounds)
      .ok_or_else(|| anyhow::anyhow!("Empty capture region"))?;
    let snapshot = snapshot(page, bounds);
    self.overlays.close_preview();
    self.reader.cancel_scroll();
    self.disarm();
    self.overlays.open_region(Region::open(snapshot));
    self.reader.notice(BAR_HINT);
    Ok(())
  }

  pub(crate) fn region_input(&mut self, input: Input) -> Result<bool> {
    let Some(region) = self.overlays.region_mut() else { return Ok(false) };
    match region.input(input.action) {
      Transition::Stay => {}
      Transition::Run(operation) => self.region_operation(operation)?,
      Transition::Close => {
        self.dismiss_overlay();
        self.disarm();
      }
      Transition::Clear => {
        self.overlays.close_region();
        return Ok(false);
      }
      Transition::Command | Transition::Unhandled => return Ok(false),
      Transition::Transfer => {
        self.dismiss_overlay();
        return Ok(false);
      }
    }
    Ok(true)
  }

  pub fn region_operation(&mut self, operation: Operation) -> Result<()> {
    anyhow::ensure!(
      self.region_job.is_none() && self.region_send.is_none(),
      "Region operation running"
    );
    let selected = self
      .overlays
      .region_mut()
      .ok_or_else(|| anyhow::anyhow!("No selected region"))?
      .request(next_capture());
    if matches!(operation, Operation::Text) {
      let source = self.doc.text(selected.page)?;
      let text = source.in_region(selected.bounds);
      self.region_text(text)?;
      self.region_status(selected.id, "Text copied".into());
      return Ok(());
    }
    let request = capture::Request {
      path: self.path.clone(),
      password: self.password.clone(),
      page: selected.page,
      bounds: selected.bounds,
      rotation: self.layout.rotation(),
    };
    let (path, page, id) = (self.path.clone(), selected.page, selected.id);
    let work = capture::Job::start(request, &self.wake, move |shot, cancel| {
      finish(&path, page, shot.png, operation, cancel)
    });
    self.region_job = Some(Job { id, work });
    self.region_status(id, "Region operation running".into());
    Ok(())
  }

  pub(crate) fn region_updates(&mut self) -> Result<bool> {
    let Some(job) = &self.region_job else { return Ok(false) };
    let Some(result) = job.work.poll() else { return Ok(false) };
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
    self.reader.deliver_yank(Yank { text, register: '"' });
    Ok(())
  }

  pub(crate) fn region_status(&mut self, id: CaptureId, message: String) {
    if self
      .overlays
      .region_mut()
      .is_some_and(|region| !region.update(id, &message))
    {
      return;
    }
    self.reader.notice(message);
  }

  pub fn draw_region(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let snap_to_cells = graphics.crop_quirk();
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    self.scene_region(&mut painter, snap_to_cells);
    painter.finish()?;
    Ok(())
  }

  pub(crate) fn scene_region<'a>(
    &mut self,
    scene: &mut impl vpdf_model::scene::Draw<'a>,
    snap_to_cells: bool,
  ) {
    let sweep = self
      .gesture
      .sweep()
      .and_then(|sweep| sweep.bounds().map(|bounds| (sweep.page(), bounds)));
    vpdf_ui::region::Canvas {
      layout: &self.layout,
      scroll: self.reader.scroll,
      frame: self.frame(self.layout.screen()),
    }
    .scene(self.overlays.region_mut(), sweep, snap_to_cells, scene);
  }
}

fn snapshot(page: PageId, bounds: Bounds) -> Snapshot {
  Snapshot { id: next_capture(), page, bounds }
}

/// Delivers a captured region on the worker.
fn finish(
  path: &Path,
  page: PageId,
  png: Vec<u8>,
  operation: Operation,
  cancel: &AtomicBool,
) -> Result<Outcome> {
  match operation {
    Operation::Image if clipboard::remote() => Ok(Outcome::Remote(png)),
    Operation::Image => {
      clipboard::image_native(png)?;
      Ok(Outcome::Message("Image copied".into()))
    }
    Operation::Save => {
      let saved = save(path, page, &png)?;
      Ok(Outcome::Message(format!("Saved {}", saved.display())))
    }
    Operation::Ocr => {
      Ok(Outcome::Text(capture::recognize(png, "eng", cancel)?))
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

  #[test]
  fn results_wake_the_loop() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("region.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let screen = vpdf_ui::layout::Screen {
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
    let result = app.region_job.as_ref().unwrap().work.poll().unwrap();
    assert!(matches!(result, Ok(Outcome::Message(_))));
    let png = std::fs::read(dir.path().join("region-p1-region-1.png"))?;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(woken.try_iter().count(), 0);
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn region_job_survives_focus() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("paper.pdf");
    std::fs::copy(root.join("paper.pdf"), &path)?;
    let screen = vpdf_ui::layout::Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&path, "", false, screen)?;
    app.move_figure(true, 1)?;
    app.open_region()?;
    let page = app.overlays.region().unwrap().snapshot().page;
    app.region_operation(Operation::Save)?;
    let original = app.region_job.as_ref().unwrap().id;
    app.blur()?;
    assert!(app.overlays.region().is_none());
    assert_eq!(app.region_job.as_ref().unwrap().id, original);
    app.focused = false;
    {
      let seed_value = PageId(app.doc.pages.len() - 1);
      app.reader.seed_reader(|reader| reader.cursor.page = seed_value);
    };
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
