//! Background document jobs a pane polls: OCR, outline, margin crops.

use crate::Result;
use crate::app::App;
use std::sync::{Arc, mpsc};
use vpdf_pdf::jobs::crop::{Request, Update};
use vpdf_pdf::jobs::ocr::OcrJob;
use vpdf_pdf::jobs::outline::{OutlineJob, spawn};
use vpdf_ui::keys::PanelKind;

impl App {
  pub fn start_ocr(&mut self, language: &str) -> Result<()> {
    if let Some(job) = self.ocr.take() {
      job.cancel();
    }
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
    let Some(job) = &self.ocr else { return Ok(false) };
    let Ok(result) = job.result.try_recv() else { return Ok(false) };
    let page = job.page;
    self.ocr = None;
    match result {
      Ok(text) => {
        if let Some(worker) = &self.search_worker {
          worker.seed(page, Arc::clone(&text));
        }
        self.doc.seed(page, text);
        self.reader.notice("OCR ready");
        self.reading_view().reclamp_text()?;
      }
      Err(error) => self.error = Some(error),
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
    if self.complete {
      self.resume_outline();
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

  /// `zc`: trims white margins, or restores them.
  pub fn toggle_crop(&mut self) -> Result<()> {
    if self.crop_scan.clear() || !self.layout.crop().is_none() {
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
