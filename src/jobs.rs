//! Background document jobs and their results.

use crate::Result;
use crate::app::App;
use std::sync::{Arc, mpsc};
use vpdf_pdf::jobs::crop::{Request, Update};
use vpdf_pdf::jobs::ocr::OcrJob;
use vpdf_pdf::jobs::outline::{OutlineJob, spawn};
use vpdf_ui::keys::PanelKind;

#[derive(Default)]
pub(crate) struct AutoOcr {
  seen: std::collections::HashSet<vpdf_model::text::PageId>,
  stopped: bool,
}

impl App {
  pub(crate) fn job_updates(&mut self) -> bool {
    let panel =
      self.panel.as_mut().is_some_and(crate::panel::PanelSession::updates);
    let updated = self.search_updates().and_then(|search| {
      Ok(
        search
          | self.ocr_updates()?
          | self.outline_updates()?
          | self.figure_updates()?
          | self.crop_updates()?,
      )
    });
    panel
      | updated.unwrap_or_else(|error| {
        self.error = Some(error.to_string());
        true
      })
  }

  pub fn start_ocr(&mut self, language: &str) -> Result<()> {
    if let Some(job) = self.ocr.take() {
      job.cancel();
    }
    self.auto_ocr.seen.insert(self.navigation_position().page);
    self.ocr = Some(OcrJob::start(
      &self.path,
      &self.password,
      self.navigation_position().page,
      language,
      &self.wake,
    ));
    self.reader.notice("OCR running; :cancel stops");
    Ok(())
  }

  pub fn ocr_updates(&mut self) -> Result<bool> {
    self.defer_ocr();
    let Some(job) = &self.ocr else { return Ok(false) };
    let result = match job.result.try_recv() {
      Ok(result) => result,
      Err(mpsc::TryRecvError::Empty) => return Ok(false),
      Err(mpsc::TryRecvError::Disconnected) => Err("OCR worker stopped".into()),
    };
    let page = job.page;
    let automatic = job.is_automatic();
    self.ocr = None;
    match result {
      Ok(Some(text)) => {
        if let Some(worker) = &self.search_worker {
          worker.seed(page, Arc::clone(&text));
        }
        self.doc.seed(page, text);
        self.refresh_figures()?;
        if self.doc.has_ocr(page) {
          self.auto_ocr.seen.remove(&page);
        }
        if !automatic {
          self.reader.notice("OCR ready");
        }
        self.reading_view().reclamp_text()?;
      }
      Err(error) if !automatic => self.error = Some(error),
      Ok(None) | Err(_) => {}
    }
    Ok(true)
  }
}

/// Where a pane's outline stands. Automatic loading waits for the first
/// frame that shows the whole view, so it cannot slow the first page.
pub enum Loading {
  Deferred,
  Running(OutlineJob),
  Loaded,
  /// `:cancel` stopped it; only an explicit request starts it again.
  Canceled,
}

impl App {
  /// Starts loading the outline, which `Document::open` skips.
  pub fn load_outline(&mut self) {
    self.outline_load =
      Loading::Running(spawn(&self.path, &self.password, &self.wake));
  }

  /// Starts a deferred or canceled load for a command that needs the
  /// outline.
  pub fn want_outline(&mut self) {
    if matches!(self.outline_load, Loading::Deferred | Loading::Canceled) {
      self.load_outline();
    }
  }

  /// Starts a deferred load once the pane's whole view reached the
  /// terminal.
  pub fn start_deferred(&mut self) {
    self.defer_ocr();
    if self.complete {
      self.resume_outline();
      self.start_auto_ocr();
    }
  }

  fn start_auto_ocr(&mut self) {
    if self.auto_ocr.stopped
      || self.ocr.is_some()
      || self.covered()
      || self.overlays.overview().is_some()
      || self.reader.animation().active()
    {
      return;
    }
    let page = self.navigation_position().page;
    if !self.doc.has_ocr(page) && self.auto_ocr.seen.insert(page) {
      self.ocr =
        Some(OcrJob::automatic(&self.path, &self.password, page, &self.wake));
    }
  }

  pub(crate) fn defer_ocr(&mut self) {
    let page = self.navigation_position().page;
    if self.ocr.as_ref().is_some_and(|job| {
      job.is_automatic()
        && (!self.complete
          || self.covered()
          || self.overlays.overview().is_some()
          || self.reader.animation().active()
          || job.page != page)
    }) {
      let job = self.ocr.take().unwrap();
      job.cancel();
      self.auto_ocr.seen.remove(&job.page);
    }
  }

  pub(crate) fn cancel_ocr(&mut self) {
    self.auto_ocr.stopped = true;
    if let Some(job) = self.ocr.take() {
      job.cancel();
    }
  }

  pub(crate) fn resume_outline(&mut self) {
    if matches!(self.outline_load, Loading::Deferred) {
      self.load_outline();
    }
  }

  pub fn cancel_outline(&mut self) {
    if self.outline_pending() {
      self.outline_load = Loading::Canceled;
    }
  }

  /// Whether outline rows are still to come.
  pub fn outline_pending(&self) -> bool {
    matches!(self.outline_load, Loading::Deferred | Loading::Running(_))
  }

  pub fn outline_updates(&mut self) -> Result<bool> {
    let Loading::Running(job) = &self.outline_load else { return Ok(false) };
    let (outline, inferred) = match job.result.try_recv() {
      Ok(found) => found,
      // The worker ended without a result.
      Err(mpsc::TryRecvError::Disconnected) => (Vec::new(), false),
      Err(mpsc::TryRecvError::Empty) => return Ok(false),
    };
    self.outline_load = Loading::Loaded;
    self.doc.outline = outline;
    self.doc.inferred = inferred;
    self.refill_panel(PanelKind::Outline)?;
    self.track_outline();
    Ok(true)
  }
}

impl App {
  pub(crate) fn start_crop(&mut self) {
    self.crop_scan.start(
      Request {
        path: &self.path,
        password: &self.password,
        doc: &self.doc,
        wake: &self.wake,
      },
      self.nearby_pages(),
    );
    self.layout.request_crop();
    self.reader.notice("Detecting margins");
  }

  fn apply_crop(
    &mut self,
    crops: Arc<vpdf_model::geometry::Crops>,
  ) -> Result<()> {
    let animation = self.reading_view().suspend_scroll();
    self.layout.resolve_crops(crops);
    self.relayout()?;
    if let Some(point) = animation {
      self.reading_view().resume_scroll(point);
    }
    self.reader.notice("Margins cropped");
    Ok(())
  }

  pub(crate) fn request_crop(&mut self) {
    if self.layout.crop().is_none() {
      return;
    }
    let initial = self.crop_scan.request(
      Request {
        path: &self.path,
        password: &self.password,
        doc: &self.doc,
        wake: &self.wake,
      },
      self.nearby_pages(),
    );
    if initial {
      self.layout.request_crop();
      self.reader.notice("Detecting margins");
    }
  }

  pub fn crop_updates(&mut self) -> Result<bool> {
    self.request_crop();
    let Some(update) = self.crop_scan.poll(&mut self.doc) else {
      return Ok(false);
    };
    let result = match update {
      Update::Canceled => {
        self.request_crop();
        return Ok(true);
      }
      Update::Ready(result) => result,
    };
    let crops = match result {
      Ok(crops) => crops,
      Err(error) => {
        self.layout.show_margins();
        self.relayout()?;
        return Err(anyhow::Error::msg(error));
      }
    };
    self.apply_crop(crops.resolved())?;
    Ok(true)
  }

  pub(crate) fn cancel_crop(&mut self) -> Result<()> {
    if self.crop_scan.clear() {
      self.layout.show_margins();
      self.relayout()?;
    }
    Ok(())
  }

  pub(crate) fn crop_enabled(&self) -> bool {
    self.crop_scan.active() || !self.layout.crop().is_none()
  }

  pub(crate) fn set_crop(&mut self, enabled: bool) -> Result<()> {
    if enabled != self.crop_enabled() {
      self.toggle_crop()?;
    }
    Ok(())
  }

  /// `zc`: trims white margins, or restores them.
  pub fn toggle_crop(&mut self) -> Result<()> {
    if self.crop_enabled() {
      self.crop_scan.clear();
      self.layout.show_margins();
      self.relayout()?;
      self.reader.notice("Margins shown");
    } else if let Some(crops) = self.doc.margins() {
      self.apply_crop(crops.resolved())?;
    } else {
      self.start_crop();
    }
    Ok(())
  }
}

#[derive(Default)]
pub enum Figures {
  /// Waits for rendering and idle input.
  #[default]
  Deferred,
  /// Only explicit requests resume this state.
  Idle,
  Running(vpdf_pdf::jobs::figures::Job),
  Ready(vpdf_pdf::jobs::figures::Entries),
}

impl App {
  pub fn want_figures(&mut self) {
    if matches!(self.figures, Figures::Deferred | Figures::Idle) {
      self.figures = Figures::Running(vpdf_pdf::jobs::figures::Job::start(
        &self.path,
        &self.password,
        self.doc.pages.len(),
        self.doc.seeds(),
        &self.wake,
      ));
    }
  }

  pub(crate) fn refresh_figures(&mut self) -> Result<()> {
    self.figures = Figures::Deferred;
    if self.panel.as_ref().is_some_and(|p| p.view.kind() == PanelKind::Figures)
    {
      self.want_figures();
      self.refill_panel(PanelKind::Figures)?;
    }
    Ok(())
  }

  pub fn cancel_figures(&mut self) {
    if matches!(self.figures, Figures::Deferred | Figures::Running(_)) {
      self.figures = Figures::Idle;
    }
  }

  pub fn figure_updates(&mut self) -> Result<bool> {
    let Figures::Running(job) = &self.figures else { return Ok(false) };
    let found = match job.result.try_recv() {
      Ok(found) => found,
      Err(mpsc::TryRecvError::Empty) => return Ok(false),
      Err(mpsc::TryRecvError::Disconnected) => {
        Err("Figure worker stopped".into())
      }
    };
    self.figures = Figures::Idle;
    match found {
      Ok(entries) => self.figures = Figures::Ready(entries),
      Err(error) => {
        if self
          .panel
          .as_ref()
          .is_some_and(|p| p.view.kind() == PanelKind::Figures)
        {
          self.error = Some(error);
        }
      }
    }
    self.refill_panel(PanelKind::Figures)?;
    Ok(true)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;
  use vpdf_model::text::{Bounds, Glyph, Line, PageId, Text};

  fn figures_app() -> Result<(tempfile::TempDir, App, mpsc::Receiver<()>)> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("figures.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let screen = vpdf_ui::layout::Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&path, "", true, screen)?;
    let (wake, ready) = crate::fixtures::wake();
    app.wake = wake;
    Ok((directory, app, ready))
  }

  #[test]
  fn figure_refresh_uses_ocr() -> Result<()> {
    let (_directory, mut app, ready) = figures_app()?;
    app.open_panel(PanelKind::Figures)?;
    ready.recv_timeout(Duration::from_secs(5))?;
    assert!(app.figure_updates()?);
    assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
    let glyphs = "Figure 1: OCR"
      .chars()
      .enumerate()
      .map(|(index, ch)| Glyph {
        ch,
        quad: None,
        bounds: Bounds {
          x0: 20.0 + index as f32 * 7.0,
          x1: 27.0 + index as f32 * 7.0,
          y0: 70.0,
          y1: 82.0,
        },
      })
      .collect();
    let text = Text {
      lines: vec![Line { ocr: None, paragraph: true, glyphs }],
      truncated: false,
    };
    app.doc.seed(PageId(0), Arc::new(text));
    app.refresh_figures()?;
    assert!(matches!(app.figures, Figures::Running(_)));
    ready.recv_timeout(Duration::from_secs(5))?;
    assert!(app.figure_updates()?);
    let rows = app.panel.as_ref().unwrap().view.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "Figure 1: OCR");
    app.panel = None;
    app.doc.seed(PageId(0), Arc::new(Text::default()));
    app.refresh_figures()?;
    assert!(matches!(app.figures, Figures::Deferred));
    app.open_panel(PanelKind::Figures)?;
    ready.recv_timeout(Duration::from_secs(5))?;
    assert!(app.figure_updates()?);
    assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
    Ok(())
  }

  #[test]
  fn changed_pdf_rejects_figures() -> Result<()> {
    let (directory, mut app, ready) = figures_app()?;
    let replacement = directory.path().join("replacement.pdf");
    crate::fixtures::pdf(&replacement, &["body", "Figure 2: new page"], "")?;
    std::fs::rename(replacement, &app.path)?;
    app.open_panel(PanelKind::Figures)?;
    ready.recv_timeout(Duration::from_secs(5))?;
    assert!(app.figure_updates()?);
    assert!(matches!(app.figures, Figures::Idle));
    assert_eq!(app.error.as_deref(), Some("PDF changed; reopen figures"));
    assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
    Ok(())
  }
}
