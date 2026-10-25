//! Prepared sidebar rows, owned by background workers.
use crate::jobs::Load;
use crate::{Result, app::App};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use vpdf_model::text::PageId;
use vpdf_pdf::content_cache::{self, Kind};
use vpdf_pdf::jobs::{Config, OneShot, Wake};
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::Row;

#[cfg(test)]
mod tests;
mod worker;

#[derive(Clone, Copy, PartialEq)]
enum Request {
  Annotations,
  Links(PageId),
}

impl Request {
  fn code(self) -> String {
    format!(
      "{}{}{}{}{}{}{:?}",
      include_str!("sidebar.rs"),
      include_str!("sidebar/worker.rs"),
      include_str!("../crates/ui/src/annotations.rs"),
      include_str!("../crates/ui/src/panel/links.rs"),
      include_str!("../crates/ui/src/panel/content.rs"),
      include_str!("pdf_text.rs"),
      match self {
        Self::Links(page) => Some(page),
        Self::Annotations => None,
      }
    )
  }
  fn kind(self) -> Kind {
    match self {
      Self::Annotations => Kind::Annotations,
      Self::Links(_) => Kind::Links,
    }
  }
  fn slot(self) -> usize {
    match self {
      Self::Annotations => ANNOTATIONS,
      Self::Links(_) => LINKS,
    }
  }
}

/// Loads per pane, each with one worker slot.
const SLOTS: usize = 2;
const ANNOTATIONS: usize = 0;
const LINKS: usize = 1;

/// Sidebar workers alive across panes, canceled or not.
static ALIVE: AtomicUsize = AtomicUsize::new(0);

/// Most workers alive at once; one pane's loads never wait on each
/// other.
pub(crate) fn budget() -> usize {
  Config::default().helpers.get().max(SLOTS)
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

/// Holds the links slot as a running worker would.
#[cfg(test)]
pub(crate) fn hold_links(app: &App) -> impl Sized + use<> {
  Flight::take(&app.sidebar.slots[LINKS].alive, &app.wake)
    .expect("links slot free")
}

/// One load's worker; a request made while it runs waits, and starts
/// for the then-current page once it ends.
#[derive(Default)]
struct Slot {
  alive: Arc<AtomicBool>,
  queued: bool,
}

#[derive(Default)]
pub(crate) struct Sidebar {
  annotations: Load<Vec<Row>>,
  links: Load<Vec<Row>>,
  page: Option<PageId>,
  slots: [Slot; SLOTS],
}

impl App {
  fn sidebar_request(&self, kind: PanelKind) -> Option<Request> {
    match kind {
      PanelKind::Annotations => Some(Request::Annotations),
      PanelKind::Links => Some(Request::Links(self.navigation_position().page)),
      _ => None,
    }
  }
  fn sidebar_load(&mut self, request: Request) -> &mut Load<Vec<Row>> {
    match request {
      Request::Annotations => &mut self.sidebar.annotations,
      Request::Links(page) => {
        if self.sidebar.page != Some(page) {
          self.sidebar.page = Some(page);
          if !matches!(self.sidebar.links, Load::Idle) {
            self.sidebar.links = Load::Deferred;
          }
        }
        &mut self.sidebar.links
      }
    }
  }
  pub(crate) fn want_sidebar(&mut self, kind: PanelKind) {
    let Some(request) = self.sidebar_request(kind) else {
      return;
    };
    let startable = self.sidebar_load(request).startable();
    let slot = &mut self.sidebar.slots[request.slot()];
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
      let found =
        worker::scan(&path, &password, bounds, request, seeds, stopped, root);
      (!stopped.load(Ordering::Relaxed)).then_some(found)
    });
    *self.sidebar_load(request) = Load::Running(job);
  }
  pub(crate) fn sidebar_rows(&mut self, kind: PanelKind) -> Vec<Row> {
    let Some(request) = self.sidebar_request(kind) else {
      return Vec::new();
    };
    match self.sidebar_load(request) {
      Load::Ready(rows) => rows.clone(),
      _ => Vec::new(),
    }
  }
  pub fn sidebar_pending(&self, kind: PanelKind) -> bool {
    self.sidebar_queued(kind)
      || match kind {
        PanelKind::Annotations => self.sidebar.annotations.pending(),
        PanelKind::Links => self.links_stale() || self.sidebar.links.pending(),
        _ => false,
      }
  }
  /// The load waits for its slot's worker to end.
  pub(crate) fn sidebar_queued(&self, kind: PanelKind) -> bool {
    match kind {
      PanelKind::Annotations => self.sidebar.slots[ANNOTATIONS].queued,
      PanelKind::Links => self.sidebar.slots[LINKS].queued,
      _ => false,
    }
  }
  pub(crate) fn sidebar_loading(&self, kind: PanelKind) -> bool {
    matches!(
      (kind, &self.sidebar.annotations, &self.sidebar.links),
      (PanelKind::Annotations, Load::Running(_), _)
        | (PanelKind::Links, _, Load::Running(_))
    )
  }
  pub(crate) fn prepare_sidebars(&mut self) {
    self.resume_outline();
    if matches!(self.figures, crate::jobs::Figures::Deferred) {
      self.want_figures();
    }
    for kind in [PanelKind::Annotations, PanelKind::Links] {
      let Some(request) = self.sidebar_request(kind) else {
        continue;
      };
      if matches!(self.sidebar_load(request), Load::Deferred) {
        self.want_sidebar(kind);
      }
    }
  }
  /// Loads a pass could start; one waiting on its slot starts when the
  /// slot's worker ends and wakes the loop.
  pub(crate) fn sidebars_deferred(&self) -> bool {
    let free = |kind| !self.sidebar_queued(kind);
    matches!(self.outline_load, crate::jobs::Loading::Deferred)
      || matches!(self.figures, crate::jobs::Figures::Deferred)
      || free(PanelKind::Annotations)
        && matches!(self.sidebar.annotations, Load::Deferred)
      || free(PanelKind::Links)
        && (self.links_stale() || matches!(self.sidebar.links, Load::Deferred))
  }
  /// Links loaded or loading for a page other than the current one.
  fn links_stale(&self) -> bool {
    self.sidebar.page != Some(self.navigation_position().page)
      && !matches!(self.sidebar.links, Load::Idle)
  }
  pub(crate) fn cancel_sidebars(&mut self) {
    self.sidebar.annotations.cancel();
    self.sidebar.links.cancel();
    for slot in &mut self.sidebar.slots {
      slot.queued = false;
    }
  }
  pub(crate) fn refresh_sidebars(&mut self) -> Result<()> {
    let slots = self
      .sidebar
      .slots
      .each_ref()
      .map(|slot| Slot { alive: Arc::clone(&slot.alive), queued: false });
    self.sidebar = Sidebar { slots, ..Sidebar::default() };
    if let Some(kind) = self.panel.as_ref().map(|p| p.view.kind()) {
      self.want_sidebar(kind);
      if matches!(kind, PanelKind::Links | PanelKind::Annotations) {
        self.refill_panel(kind)?;
      }
    }
    Ok(())
  }
  pub fn sidebar_updates(&mut self) -> Result<bool> {
    let mut changed = false;
    for kind in [PanelKind::Annotations, PanelKind::Links] {
      let Some(request) = self.sidebar_request(kind) else {
        continue;
      };
      let visible = self.panel.as_ref().is_some_and(|p| p.view.kind() == kind);
      let deferred = matches!(self.sidebar_load(request), Load::Deferred);
      if self.sidebar.slots[request.slot()].queued || visible && deferred {
        self.want_sidebar(kind);
        if visible && matches!(self.sidebar_load(request), Load::Running(_)) {
          self.refill_panel(kind)?;
          changed = true;
        }
      }
      let Some(found) =
        self.sidebar_load(request).poll("Sidebar worker stopped")
      else {
        continue;
      };
      if let Err(error) = found
        && visible
      {
        self.error = Some(error);
      }
      self.refill_panel(kind)?;
      changed |= visible;
    }
    Ok(changed)
  }
}
