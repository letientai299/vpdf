//! The native window's tabs and splits, saved apart from reading state.
//!
//! One native instance claims the session file through a lock held for
//! its lifetime; concurrent instances neither restore nor overwrite it.
//! Terminal sessions never touch it.
use super::super::{Session, Tab};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vpdf_model::geometry::Crop;
use vpdf_model::text::Position;
use vpdf_pdf::document::PasswordRequired;
use vpdf_ui::layout::Screen;
use vpdf_ui::split::{Node, Shape};

const VERSION: u32 = 1;
/// Coalesces writes after input.
const DELAY: Duration = Duration::from_secs(2);
/// Launches a tab may fail before it is dropped.
const ATTEMPTS: u8 = 3;

/// Open tabs and window state, as last saved.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Layout {
  version: u32,
  tabs: Vec<Saved>,
  active: usize,
  #[serde(default)]
  pub fullscreen: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Saved {
  panes: Vec<Pane>,
  shape: Shape,
  focus: usize,
  /// Launches that could not reopen any pane.
  #[serde(default)]
  missed: u8,
}

impl Saved {
  /// Each pane appears in the shape exactly once.
  fn valid(&self) -> bool {
    let mut seen = vec![false; self.panes.len()];
    let mut ok = true;
    let mut visit = |shape: &Shape| {
      let mut stack = vec![shape];
      while let Some(shape) = stack.pop() {
        match shape {
          Shape::Pane(index) => match seen.get_mut(*index) {
            Some(seen) if !*seen => *seen = true,
            _ => ok = false,
          },
          Shape::Split { first, second, .. } => {
            stack.extend([first.as_ref(), second.as_ref()]);
          }
        }
      }
    };
    visit(&self.shape);
    ok && !seen.is_empty() && seen.iter().all(|seen| *seen)
  }
}

/// A pane's view; reading state keeps the rest of the document's place.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Pane {
  path: PathBuf,
  cursor: Position,
  zoom: f32,
  rotation: u16,
  #[serde(default)]
  crop: Crop,
  /// The reading-state visit this view belongs to.
  #[serde(default)]
  opened: crate::state::Visit,
}

/// The claimed session file.
pub struct Store {
  path: PathBuf,
  _lock: File,
  written: Option<Layout>,
  due: Option<Instant>,
}

impl Store {
  /// Claims the session beside `state`; None while another instance
  /// holds it.
  pub fn claim(state: &Path) -> Option<Self> {
    let path = state.with_file_name("native-session.json");
    std::fs::create_dir_all(path.parent()?).ok()?;
    let lock = File::options()
      .create(true)
      .truncate(false)
      .write(true)
      .open(path.with_extension("lock"))
      .ok()?;
    lock.try_lock().ok()?;
    Some(Self { path, _lock: lock, written: None, due: None })
  }

  /// The saved layout; None when missing, unreadable, or from another
  /// version.
  pub fn load(&mut self) -> Option<Layout> {
    let bytes = std::fs::read(&self.path).ok()?;
    let mut layout: Layout = serde_json::from_slice(&bytes).ok()?;
    let active = layout.active;
    let mut index = 0;
    let mut before = 0;
    layout.tabs.retain(|tab| {
      let valid = tab.valid();
      before += usize::from(valid && index < active);
      index += 1;
      valid
    });
    layout.active = before;
    (layout.version == VERSION).then(|| {
      self.written = Some(layout.clone());
      layout
    })
  }

  /// Writes `layout` unless already saved.
  fn save(&mut self, layout: Layout) -> Result<()> {
    if self.written.as_ref() == Some(&layout) {
      return Ok(());
    }
    let directory = self.path.parent().unwrap_or(Path::new("."));
    let mut staged =
      tempfile::Builder::new().suffix(".tmp").tempfile_in(directory)?;
    staged.write_all(&serde_json::to_vec(&layout)?)?;
    staged.as_file().sync_all()?;
    staged.persist(&self.path).map_err(|error| error.error)?;
    self.written = Some(layout);
    Ok(())
  }

  /// Schedules a save; continuous input still saves every `DELAY`.
  pub fn touch(&mut self, now: Instant) {
    self.due.get_or_insert(now + DELAY);
  }

  /// Saves when due; returns the wait until the next save.
  pub fn poll(
    &mut self,
    session: &mut Session,
    now: Instant,
  ) -> Option<Duration> {
    let due = self.due?;
    if now < due {
      return Some(due - now);
    }
    self.due = None;
    self.flush(session);
    None
  }

  pub fn flush(&mut self, session: &mut Session) {
    if let Err(error) = self.save(session.layout()) {
      super::report(session, Err(error.context("Session not saved")));
    }
  }
}

/// Native window state the session file keeps.
#[derive(Default)]
pub struct Windows {
  pub restoring: Option<Restoring>,
  /// Tabs whose files may return, saved after the open ones.
  unopened: Vec<Saved>,
  /// The window fills its screen.
  pub fullscreen: bool,
}

impl Windows {
  pub fn new(layout: Option<Layout>) -> Self {
    let Some(layout) = layout else { return Self::default() };
    Self {
      fullscreen: layout.fullscreen,
      restoring: Some(Restoring::new(layout)),
      unopened: Vec::new(),
    }
  }

  /// The window reported its size, so restored panes fit it.
  pub fn sized(&mut self) {
    if let Some(restoring) = &mut self.restoring {
      restoring.sized = true;
    }
  }

  /// Records full-screen state; no frame needed.
  pub fn fullscreen(&mut self, full: bool) -> bool {
    self.fullscreen = full;
    false
  }
}

/// Saved tabs still to open, the saved active tab first.
#[derive(Default)]
pub struct Restoring {
  pending: VecDeque<(usize, Saved)>,
  /// Saved index and first pane id of each restored tab.
  placed: Vec<(usize, u32)>,
  /// Saved tab index and description of each file not reopened.
  skipped: Vec<(usize, String)>,
  active: usize,
  /// Whether the window has reported its size.
  pub sized: bool,
}

impl Restoring {
  pub fn new(layout: Layout) -> Self {
    let active = layout.active.min(layout.tabs.len().saturating_sub(1));
    let mut tabs: Vec<_> = layout.tabs.into_iter().enumerate().collect();
    let before: Vec<_> = tabs.drain(..active).rev().collect();
    Self {
      pending: tabs.into_iter().chain(before).collect(),
      active,
      ..Self::default()
    }
  }

  pub fn waiting(&self) -> bool {
    self.sized && !self.pending.is_empty()
  }

  /// Where saved tab `index` goes among the open `tabs`.
  fn place(&self, index: usize, tabs: &[Tab]) -> usize {
    let at = |id: u32| tabs.iter().position(|tab| tab.panes[0].id == id);
    let after = self
      .placed
      .iter()
      .filter(|(saved, _)| *saved < index)
      .max_by_key(|(saved, _)| *saved)
      .and_then(|(_, id)| at(*id));
    let before = || {
      self
        .placed
        .iter()
        .filter(|(saved, _)| *saved > index)
        .min_by_key(|(saved, _)| *saved)
        .and_then(|(_, id)| at(*id))
    };
    after.map(|tab| tab + 1).or_else(before).unwrap_or(0)
  }
}

impl Session {
  /// Open tabs, then ones not yet restored, at their saved places.
  pub(in crate::session) fn layout(&self) -> Layout {
    let live = self.tabs.iter().enumerate().filter_map(|(index, tab)| {
      let saved = saved_tab(tab)?;
      Some((index == self.active, saved))
    });
    let (mut before, mut after) = (Vec::new(), Vec::new());
    if let Some(restoring) = &self.windows.restoring {
      for (index, saved) in &restoring.pending {
        let list =
          if *index < restoring.active { &mut before } else { &mut after };
        list.push((*index, saved.clone()));
      }
    }
    before.sort_by_key(|(index, _)| *index);
    after.sort_by_key(|(index, _)| *index);
    let mut tabs = Vec::new();
    let mut active = 0;
    // Until a tab opens, the saved active tab keeps focus.
    let waiting = self.windows.restoring.as_ref().map(|r| r.active);
    let pending = |list: Vec<(usize, Saved)>| {
      list.into_iter().map(move |(index, saved)| {
        (self.tabs.is_empty() && Some(index) == waiting, saved)
      })
    };
    for (focused, saved) in pending(before).chain(live).chain(pending(after)) {
      if focused {
        active = tabs.len();
      }
      tabs.push(saved);
    }
    tabs.extend(self.windows.unopened.iter().cloned());
    Layout {
      version: VERSION,
      tabs,
      active,
      fullscreen: self.windows.fullscreen,
    }
  }

  /// Opens the next saved tab; true when one was tried.
  pub(in crate::session) fn restore_next(&mut self, screen: Screen) -> bool {
    let Some(restoring) = &mut self.windows.restoring else { return false };
    if !restoring.sized {
      return false;
    }
    self.claim_open();
    let restoring = restoring_mut(self);
    if let Some((index, saved)) = restoring.pending.pop_front() {
      self.restore_tab(index, &saved, screen);
    }
    self.finish_restore();
    true
  }

  /// Saved single-pane tabs that explicit opens already show hold their
  /// places instead of opening twice.
  fn claim_open(&mut self) {
    let pending = std::mem::take(&mut restoring_mut(self).pending);
    let mut kept = VecDeque::new();
    let mut claimed = Vec::new();
    for (index, saved) in pending {
      let open = (saved.panes.len() == 1)
        .then(|| self.existing(&saved.panes[0].path))
        .flatten();
      match open {
        Some((tab, _)) => claimed.push((index, self.tabs[tab].panes[0].id)),
        None => kept.push_back((index, saved)),
      }
    }
    let restoring = restoring_mut(self);
    restoring.pending = kept;
    restoring.placed.extend(claimed);
  }

  fn restore_tab(&mut self, index: usize, saved: &Saved, screen: Screen) {
    let content = self.content(screen);
    let mut panes: Vec<crate::app::App> = Vec::new();
    let mut ids = Vec::new();
    let mut locked = false;
    for pane in &saved.panes {
      match self.restore_pane(pane, &panes, content) {
        Ok(app) => {
          ids.push(Some(app.id));
          panes.push(app);
        }
        Err(error) => {
          ids.push(None);
          let name = pane.path.file_name().unwrap_or(pane.path.as_os_str());
          locked |= error.is::<PasswordRequired>();
          let reason = if error.is::<PasswordRequired>() {
            "locked"
          } else if !pane.path.exists() {
            "missing"
          } else {
            "unreadable"
          };
          let note = format!("{} ({reason})", name.display());
          let skipped = &mut restoring_mut(self).skipped;
          if !skipped.iter().any(|(_, seen)| *seen == note) {
            skipped.push((index, note));
          }
        }
      }
    }
    let tree = Node::from_shape(&saved.shape, &|index| {
      ids.get(index)?.as_ref().copied()
    });
    if let Some(tree) = tree {
      let focused = ids.get(saved.focus).copied().flatten();
      let focus = focused
        .and_then(|id| panes.iter().position(|app| app.id == id))
        .unwrap_or(0);
      self.insert_restored(
        index,
        Tab {
          panes: panes.into_iter().map(Into::into).collect(),
          tree,
          focus,
          last: None,
        },
      );
    } else if !locked && saved.missed.saturating_add(1) < ATTEMPTS {
      // Files on absent volumes may return.
      let missed = saved.missed + 1;
      self.windows.unopened.push(Saved { missed, ..saved.clone() });
    }
  }

  /// Panes keep their saved views unless another reader opened the
  /// document since; then reading state places its first pane.
  fn restore_pane(
    &mut self,
    pane: &Pane,
    tab: &[crate::app::App],
    screen: Screen,
  ) -> Result<crate::app::App> {
    let path = std::fs::canonicalize(&pane.path)?;
    let reopened = self.state.entry(&path).is_some()
      && self.state.open_revision(&path) != pane.opened;
    let shown = self.panes().chain(tab).any(|app| app.path == path);
    let mut app = self.load(&path, "", screen)?;
    if shown || !reopened {
      let mut entry = app.reading_entry();
      entry.cursor = pane.cursor;
      entry.zoom = pane.zoom;
      entry.rotation = pane.rotation;
      entry.crop = pane.crop;
      app.restore(&entry)?;
    }
    self.reopen_sidebar(&mut app);
    Ok(app)
  }

  fn insert_restored(&mut self, index: usize, tab: Tab) {
    if self.tabs.len() == 1
      && self.tabs[0].panes.len() == 1
      && self.tabs[0].panes[0].is_recent()
      && self.tabs[0].panes[0].alternate.is_none()
      && self.tabs[0].panes[0].prompt().is_none()
    {
      self.tabs.clear();
    }
    let restoring = self.windows.restoring.as_mut().expect("Restore queue");
    let at = restoring.place(index, &self.tabs);
    restoring.placed.push((index, tab.panes[0].id));
    let first = self.tabs.is_empty();
    self.tabs.insert(at, tab);
    if first {
      self.active = 0;
      self.refocus();
      let arranged = self.arrange();
      super::report(self, arranged);
    } else if at <= self.active {
      self.active += 1;
    }
  }

  /// Reports skipped files once the queue empties.
  fn finish_restore(&mut self) {
    let Some(restoring) = &self.windows.restoring else { return };
    if !restoring.pending.is_empty() {
      return;
    }
    let mut skipped = std::mem::take(&mut self.windows.restoring)
      .expect("Restore queue")
      .skipped;
    if !skipped.is_empty() {
      skipped.sort_by_key(|(index, _)| *index);
      let names: Vec<_> = skipped.into_iter().map(|(_, name)| name).collect();
      let error = anyhow::anyhow!("Not reopened: {}", names.join(", "));
      super::report(self, Err(error));
    }
  }
}

fn restoring_mut(session: &mut Session) -> &mut Restoring {
  session.windows.restoring.as_mut().expect("Restore queue")
}

fn saved_tab(tab: &Tab) -> Option<Saved> {
  let kept: Vec<_> = tab
    .panes
    .iter()
    .filter_map(crate::session::Pane::document)
    .filter(|app| app.manual.is_none())
    .collect();
  let index = |id: u32| kept.iter().position(|app| app.id == id);
  let shape = tab.tree.shape(&index)?;
  let focused = tab.panes[tab.focus].id;
  Some(Saved {
    focus: index(focused).unwrap_or(0),
    panes: kept
      .iter()
      .map(|app| {
        let entry = app.reading_entry();
        Pane {
          path: entry.path,
          cursor: entry.cursor,
          zoom: entry.zoom,
          rotation: entry.rotation,
          crop: entry.crop,
          opened: app.opened,
        }
      })
      .collect(),
    shape,
    missed: 0,
  })
}

#[cfg(test)]
#[path = "restore_tests.rs"]
mod tests;
