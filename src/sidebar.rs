//! Prepared sidebar rows, owned by background workers.
use crate::jobs::Load;
use crate::{Result, app::App};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use vpdf_pdf::content_cache;
use vpdf_pdf::jobs::{Config, OneShot, Wake};
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::Row;

#[cfg(test)]
mod tests;
mod worker;

/// Cache key for the annotation rows' format.
fn code() -> String {
  format!(
    "{}{}{}",
    include_str!("sidebar.rs"),
    include_str!("sidebar/worker.rs"),
    include_str!("../crates/ui/src/annotations.rs"),
  )
}

/// Sidebar workers alive across panes, canceled or not.
static ALIVE: AtomicUsize = AtomicUsize::new(0);

/// Most workers alive at once.
pub(crate) fn budget() -> usize {
  Config::default().helpers.get()
}

/// A worker's claim on its load and the shared budget, released when
/// its thread ends: native page-tree walks ignore stop flags, so a
/// canceled worker still contends for `MuPDF`'s locks.
struct Flight {
  alive: Arc<AtomicBool>,
  wake: Wake,
}

impl Flight {
  fn take(alive: &Arc<AtomicBool>, wake: &Wake) -> Option<Self> {
    if alive.load(Ordering::Acquire) {
      return None;
    }
    let limit = budget();
    ALIVE
      .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        (count < limit).then_some(count + 1)
      })
      .ok()?;
    alive.store(true, Ordering::Release);
    Some(Self { alive: Arc::clone(alive), wake: wake.clone() })
  }
}

impl Drop for Flight {
  fn drop(&mut self) {
    self.alive.store(false, Ordering::Release);
    ALIVE.fetch_sub(1, Ordering::AcqRel);
    self.wake.wake();
  }
}

/// Holds the annotations slot as a running worker would.
#[cfg(test)]
pub(crate) fn hold_annotations(app: &App) -> impl Sized + use<> {
  Flight::take(&app.sidebar.slot.alive, &app.wake)
    .expect("annotations slot free")
}

/// One load's worker; a request made while it runs waits, and starts
/// once it ends.
#[derive(Default)]
struct Slot {
  alive: Arc<AtomicBool>,
  queued: bool,
}

#[derive(Default)]
pub(crate) struct Sidebar {
  annotations: Load<Vec<Row>>,
  /// The last loaded rows, shown while a reload runs.
  previous: Vec<Row>,
  slot: Slot,
}

impl App {
  pub(crate) fn want_annotations(&mut self) {
    let startable = self.sidebar.annotations.startable();
    let slot = &mut self.sidebar.slot;
    slot.queued = false;
    if !startable {
      return;
    }
    let Some(flight) = Flight::take(&slot.alive, &self.wake) else {
      slot.queued = true;
      return;
    };
    let (path, password, seeds) =
      (self.path.clone(), self.password.clone(), self.doc.seeds());
    let bounds = self.doc.pages.iter().map(|page| page.bounds).collect();
    let root = content_cache::root();
    let job = OneShot::try_spawn(&self.wake, move |stopped| {
      let _flight = flight;
      let found = worker::scan(&path, &password, bounds, seeds, stopped, root);
      (!stopped.load(Ordering::Relaxed)).then_some(found)
    });
    self.sidebar.annotations = Load::Running(job);
  }
  /// Loaded rows; a reload shows the previous ones until it ends.
  pub(crate) fn annotation_rows(&self) -> Vec<Row> {
    match &self.sidebar.annotations {
      Load::Ready(rows) => rows.clone(),
      _ => self.sidebar.previous.clone(),
    }
  }
  pub fn annotations_pending(&self) -> bool {
    self.annotations_queued() || self.sidebar.annotations.pending()
  }
  /// The load waits for its slot's worker to end.
  pub(crate) fn annotations_queued(&self) -> bool {
    self.sidebar.slot.queued
  }
  pub(crate) fn annotations_loading(&self) -> bool {
    matches!(self.sidebar.annotations, Load::Running(_))
  }
  pub(crate) fn prepare_sidebars(&mut self) {
    self.resume_outline();
    if matches!(self.figures, crate::jobs::Figures::Deferred) {
      self.want_figures();
    }
    if matches!(self.sidebar.annotations, Load::Deferred) {
      self.want_annotations();
    }
  }
  /// Loads a pass could start; one waiting on its slot starts when the
  /// slot's worker ends and wakes the loop.
  pub(crate) fn sidebars_deferred(&self) -> bool {
    matches!(self.outline_load, crate::jobs::Loading::Deferred)
      || matches!(self.figures, crate::jobs::Figures::Deferred)
      || !self.sidebar.slot.queued
        && matches!(self.sidebar.annotations, Load::Deferred)
  }
  pub(crate) fn cancel_sidebars(&mut self) {
    self.sidebar.annotations.cancel();
    self.sidebar.slot.queued = false;
  }
  pub(crate) fn refresh_sidebars(&mut self) -> Result<()> {
    let alive = Arc::clone(&self.sidebar.slot.alive);
    let previous = self.annotation_rows();
    self.sidebar = Sidebar {
      annotations: Load::default(),
      previous,
      slot: Slot { alive, queued: false },
    };
    let kind = PanelKind::Annotations;
    if self.panel.as_ref().is_some_and(|p| p.view.kind() == kind) {
      self.want_annotations();
      self.refill_panel(kind)?;
    }
    Ok(())
  }
  pub fn sidebar_updates(&mut self) -> Result<bool> {
    let kind = PanelKind::Annotations;
    let mut changed = false;
    let visible = self.panel.as_ref().is_some_and(|p| p.view.kind() == kind);
    let deferred = matches!(self.sidebar.annotations, Load::Deferred);
    if self.sidebar.slot.queued || visible && deferred {
      self.want_annotations();
      if visible && matches!(self.sidebar.annotations, Load::Running(_)) {
        self.refill_panel(kind)?;
        changed = true;
      }
    }
    let Some(found) = self.sidebar.annotations.poll("Sidebar worker stopped")
    else {
      return Ok(changed);
    };
    if let Err(error) = found
      && visible
    {
      self.error = Some(error);
    }
    self.refill_panel(kind)?;
    Ok(changed | visible)
  }
}
