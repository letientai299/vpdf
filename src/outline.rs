use crate::app::App;
use crate::jobs::outline::{OutlineJob, spawn};
use crate::keys::PanelKind;
use std::sync::mpsc;

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

  pub fn outline_updates(&mut self) -> crate::Result<bool> {
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
