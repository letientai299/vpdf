//! Prepared sidebar rows, owned by background workers.
use crate::{Result, app::App};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use vpdf_model::text::PageId;
use vpdf_pdf::content_cache::{self, Kind};
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
}

#[derive(Default)]
enum Load {
  #[default]
  Deferred,
  Idle,
  Running(Job),
  Ready(Vec<Row>),
}

struct Job {
  result: mpsc::Receiver<std::result::Result<Vec<Row>, String>>,
  stop: Arc<AtomicBool>,
}

impl Drop for Job {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
  }
}

#[derive(Default)]
pub(crate) struct Sidebar {
  annotations: Load,
  links: Load,
  page: Option<PageId>,
}

impl App {
  fn sidebar_request(&self, kind: PanelKind) -> Option<Request> {
    match kind {
      PanelKind::Annotations => Some(Request::Annotations),
      PanelKind::Links => Some(Request::Links(self.navigation_position().page)),
      _ => None,
    }
  }
  fn sidebar_load(&mut self, request: Request) -> &mut Load {
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
    if !matches!(self.sidebar_load(request), Load::Deferred | Load::Idle) {
      return;
    }
    let (send, result) = mpsc::channel();
    let send = self.wake.sender(send);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let (path, password, pages, seeds) = (
      self.path.clone(),
      self.password.clone(),
      self.doc.pages.len(),
      self.doc.seeds(),
    );
    let root = content_cache::root();
    std::thread::spawn(move || {
      let found =
        worker::scan(&path, &password, pages, request, seeds, &stopped, root)
          .map_err(|error| error.to_string());
      if !stopped.load(Ordering::Relaxed) {
        let _ = send.send(found);
      }
    });
    *self.sidebar_load(request) = Load::Running(Job { result, stop });
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
    match kind {
      PanelKind::Annotations => {
        matches!(self.sidebar.annotations, Load::Deferred | Load::Running(_))
      }
      PanelKind::Links => {
        self.sidebar.page != Some(self.navigation_position().page)
          && !matches!(self.sidebar.links, Load::Idle)
          || matches!(self.sidebar.links, Load::Deferred | Load::Running(_))
      }
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
  pub(crate) fn sidebars_deferred(&self) -> bool {
    matches!(self.outline_load, crate::jobs::Loading::Deferred)
      || matches!(self.figures, crate::jobs::Figures::Deferred)
      || matches!(self.sidebar.annotations, Load::Deferred)
      || self.sidebar.page != Some(self.navigation_position().page)
        && !matches!(self.sidebar.links, Load::Idle)
      || matches!(self.sidebar.links, Load::Deferred)
  }
  pub(crate) fn cancel_sidebars(&mut self) {
    for load in [&mut self.sidebar.annotations, &mut self.sidebar.links] {
      if matches!(load, Load::Deferred | Load::Running(_)) {
        *load = Load::Idle;
      }
    }
  }
  pub(crate) fn refresh_sidebars(&mut self) -> Result<()> {
    self.sidebar = Sidebar::default();
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
      if visible && matches!(self.sidebar_load(request), Load::Deferred) {
        self.want_sidebar(kind);
        self.refill_panel(kind)?;
        changed = true;
      }
      let Load::Running(job) = self.sidebar_load(request) else {
        continue;
      };
      let found = match job.result.try_recv() {
        Ok(found) => found,
        Err(mpsc::TryRecvError::Empty) => continue,
        Err(mpsc::TryRecvError::Disconnected) => {
          Err("Sidebar worker stopped".into())
        }
      };
      *self.sidebar_load(request) = match found {
        Ok(rows) => Load::Ready(rows),
        Err(error) => {
          if visible {
            self.error = Some(error);
          }
          Load::Idle
        }
      };
      self.refill_panel(kind)?;
      changed |= visible;
    }
    Ok(changed)
  }
}
