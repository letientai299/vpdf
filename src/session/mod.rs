use crate::Result;
use crate::app::{App, Request, Tabline};
use crate::dnd::Target;
use crate::home::{Home, Outcome};
use crate::pace::Pacer;
use crate::state::{Mark, State};
use crate::trace::Drawable;
use crate::watch::Watcher;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use vpdf_model::geometry::At;
use vpdf_model::input::MouseKind;
use vpdf_model::input::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
#[cfg(test)]
use vpdf_model::text::PageId;
use vpdf_model::text::Position;
use vpdf_pdf::document::PasswordRequired;
use vpdf_terminal::decode::{Event, Size};
use vpdf_terminal::dnd::{Dropped, Step};
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal::{self, Cause, Shape, Sight, View, sight};
use vpdf_ui::chrome::Notice;
use vpdf_ui::keys::PanelKind;
use vpdf_ui::layout::Screen;
use vpdf_ui::pointer::{Hit, Hover, Pixel, Pointer, Region, Route};
use vpdf_ui::prompt::History;
use vpdf_ui::split::{Axis, Direction, Divider, Node, Place, Rect, neighbor};
use vpdf_ui::window::{TabMove, Window};

/// Panes of one tab, laid out by a split tree.
pub struct Tab {
  pub panes: Vec<App>,
  pub tree: Node,
  pub focus: usize,
  /// Pane focused before the current one, for `Ctrl-w p`.
  last: Option<u32>,
}

/// Open documents as tabs of split panes, which draw with one shared
/// Kitty image cache.
#[derive(Default)]
pub struct Session {
  native: native::commands::State,
  help: help::Help,
  settings: crate::config::Settings,
  system_palette: vpdf_model::theme::Palette,
  config_source: crate::config::Source,
  startup_warning: Option<String>,
  config_warning: Option<String>,
  pub tabs: Vec<Tab>,
  pub active: usize,
  pub history: History,
  /// Images every pane of every tab shares.
  pub graphics: Graphics,
  pub quit: bool,
  pub state: State,
  frontend: crate::raster::Frontend,
  observing: Option<()>,
  unlock: Option<unlock::Unlock>,
  native_opens: std::collections::VecDeque<vpdf_model::reference::Reference>,
  pending_keys: std::collections::VecDeque<KeyEvent>,
  secret: Option<String>,
  picker: Option<crate::programs::Picker>,
  printing: Option<printing::Job>,
  perf: crate::perf::Monitor,
  /// Start screen while no document is open.
  pub home: Option<Home>,
  /// The whole terminal; panes divide it.
  screen: Option<Screen>,
  watcher: Watcher,
  store: Option<PathBuf>,
  /// Set by SIGHUP or SIGTERM, so the session ends and saves.
  hangup: Arc<AtomicBool>,
  latest: crate::state::Latest,
  cleanup: crate::state::Cleaner,
  /// Recorded positions not yet written.
  unsaved: bool,
  /// History revision last handed to the hangup thread.
  recorded: u64,
  /// Panes the last arrangement showed; ones hidden since are trimmed.
  shown: Vec<u32>,
  pointer: Pointer,
  /// Set while a resize could not measure cells.
  unmeasured: Option<Notice>,
  sight: Sight,
  observer: Option<vpdf_terminal::tmux_observer::Observer>,
  observed: Option<vpdf_terminal::tmux_observer::Snapshot>,
  rehover: Rehover,
  /// A wheel gesture deferred the page hover until it ends.
  skimmed: bool,
  image_write: Option<clipboard::ImageWrite>,
  /// The drag over the window, OSC 72.
  drops: vpdf_terminal::dnd::Drops,
  /// The boundary a press drags.
  resizing: Option<Edge>,
}

/// A boundary between panes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Edge {
  Split(Divider),
  /// The side panel's border in pane `id`.
  Panel(u32),
}

impl Edge {
  fn hit(self) -> Hit {
    let rows = matches!(self, Self::Split(Divider { axis: Axis::Rows, .. }));
    Hit::Edge { rows }
  }
}

/// Whether a frame moved the hover under a resting pointer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Rehover {
  #[default]
  Settled,
  /// The next frame shows the new hover.
  Due,
  /// That frame is drawing; it asks no more.
  Following,
}

/// Input the session acts on.
#[derive(Debug, PartialEq)]
enum Act {
  Key(KeyEvent),
  Paste(String),
  /// With the mode 2048 report, when the terminal sent one.
  Resize(Option<Size>),
  Mouse(vpdf_model::input::Mouse),
  /// The pointer left the window; not typing.
  Leave,
  /// The terminal no longer holds this image; not typing.
  Missing(u32),
  ImageStatus(String),
  /// OSC 72: a drag over the window or a drop; not typing.
  Drop(String),
  /// Mode 1004, which only tmux is asked for; true when the pane gains
  /// focus. Not typing.
  Focus(bool),
}

impl Act {
  /// Typing: user input but motion and leaving.
  fn typed(&self) -> bool {
    match self {
      Self::Mouse(mouse) => mouse.kind != MouseKind::Moved,
      Self::Missing(_)
      | Self::Focus(_)
      | Self::ImageStatus(_)
      | Self::Drop(_)
      | Self::Leave => false,
      Self::Key(_) | Self::Paste(_) | Self::Resize(_) => true,
    }
  }
}

/// What `event` asks of the session; None for input it ignores yet,
/// which must not count as typing: other replies, buttons past 9,
/// back and forward only on press, and key releases.
use run::{Landing, answer};

impl Session {
  /// `store` holds recent documents and history between runs.
  pub fn new(store: Option<PathBuf>) -> Self {
    Self::configured(store, crate::config::Loaded::default())
  }

  pub fn configured(
    store: Option<PathBuf>,
    config: crate::config::Loaded,
  ) -> Self {
    let state = store.as_deref().map_or_default(State::load);
    vpdf_ui::panel::set_ascii_glyphs(config.settings.ascii_glyphs());
    Self {
      settings: config.settings,
      config_source: config.source,
      config_warning: config.warning.clone(),
      startup_warning: config.warning,
      history: History::new(state.commands.clone(), state.searches.clone()),
      state,
      store,
      ..Self::default()
    }
  }

  fn set_system_palette(
    &mut self,
    palette: vpdf_model::theme::Palette,
  ) -> bool {
    let changed = self.system_palette != palette;
    self.system_palette = palette;
    let system = matches!(
      self.settings.theme(),
      crate::config::ThemeChoice::Builtin(vpdf_model::theme::Theme::System)
    );
    if changed && system {
      self.refresh_help();
    }
    changed && system
  }

  /// Password prompt or file picker holds input.
  fn blocked(&self) -> bool {
    self.unlock.is_some() || self.picker.is_some()
  }

  fn panes(&self) -> impl Iterator<Item = &App> {
    self.tabs.iter().flat_map(|tab| &tab.panes)
  }

  fn panes_mut(&mut self) -> impl Iterator<Item = &mut App> {
    self.tabs.iter_mut().flat_map(|tab| &mut tab.panes)
  }

  fn pane_mut(&mut self, id: u32) -> Option<&mut App> {
    self.panes_mut().find(|app| app.id == id)
  }

  fn present_startup(&mut self) {
    if self.unlock.is_some()
      || (!self.tabs.is_empty() && self.app().error.is_some())
      || self.home.as_ref().is_some_and(|home| home.view.error().is_some())
    {
      return;
    }
    let Some(warning) = self.startup_warning.take() else { return };
    if self.tabs.is_empty() {
      if let Some(home) = &mut self.home {
        home.view.fail(warning);
      } else {
        self.startup_warning = Some(warning);
      }
    } else {
      self.app().reader.notice(warning);
    }
  }

  fn remember_recent_layout(
    &mut self,
    preferences: vpdf_ui::panel::recent::Preferences,
  ) {
    if self.state.set_recent_layout(preferences) {
      self.changed();
    }
  }

  fn record_recent_layout(&mut self) {
    let preferences = self
      .tabs
      .get(self.active)
      .and_then(|tab| tab.panes.get(tab.focus))
      .and_then(|app| app.panel.as_ref())
      .and_then(|panel| panel.view.shelf())
      .map(vpdf_ui::panel::recent::Shelf::preferences);
    if let Some(preferences) = preferences {
      self.remember_recent_layout(preferences);
    }
  }

  fn show_recent(&mut self) -> Result<()> {
    let recent = self.state.recent.clone();
    let preferences = self.state.recent_layout;
    self.app().show_recent(&recent)?;
    self
      .app()
      .panel
      .as_mut()
      .unwrap()
      .view
      .shelf_mut()
      .unwrap()
      .set_preferences(preferences);
    Ok(())
  }

  pub fn save(&mut self) -> Result<()> {
    if let Some(store) = &self.store {
      let history = &self.history;
      self.state.save(store, history.commands(), history.searches())?;
    }
    self.refresh_recent();
    Ok(())
  }

  fn remember_open(&mut self, tab: usize, pane: usize) {
    self.state.refresh_history(self.store.as_deref());
    let app = &mut self.tabs[tab].panes[pane];
    if app.manual.is_some() {
      return;
    }
    if self.state.entry(&app.path).is_none()
      || app.opened != self.state.open_revision(&app.path)
    {
      app.marks.clear();
    }
    if self.state.entry(&app.path).is_none()
      || !app.opened.same_document(self.state.open_revision(&app.path))
    {
      let saved =
        self.state.entry(&app.path).and_then(|entry| entry.outline.clone());
      app.outline.restore_preferences(saved);
      if let Some(panel) = &mut app.panel
        && panel.view.kind() == PanelKind::Outline
      {
        app.outline.project(&mut panel.view);
      }
    }
    app.opened = self.state.opened(app.reading_entry());
    self.changed();
  }

  pub fn forget_recent(&mut self, paths: &[PathBuf]) -> Result<()> {
    self.state.refresh_history(self.store.as_deref());
    if !self.state.forget(paths) {
      return Ok(());
    }
    for app in self.panes_mut() {
      if paths.contains(&app.path) {
        app.marks.clear();
      }
    }
    self.refresh_recent();
    self.changed();
    self
      .flush()
      .map(|_| ())
      .map_err(|error| anyhow::anyhow!("Recent removal not saved: {error}"))
  }

  /// Removes the focused document's local mark, or a global mark for
  /// every document.
  pub fn remove_mark(&mut self, name: char) -> Result<()> {
    if name.is_ascii_lowercase() {
      self.app().marks.remove(&name);
      // The hangup save reads the recorded pane.
      self.record();
    } else {
      self.state.remove_mark(name);
    }
    self.refill_marks();
    self.changed();
    Ok(())
  }

  /// Refills every open Marks panel.
  pub(crate) fn refill_marks(&mut self) {
    let global = self.state.marks.clone();
    for app in self.panes_mut() {
      app.refill_marks(&global);
    }
  }

  fn refresh_recent(&mut self) {
    let entries = &self.state.recent;
    let rows = crate::panel::saved_rows(entries);
    let cards = crate::recent::entries(entries);
    if let Some(home) = &mut self.home {
      home.view.refresh_recent(rows.clone(), &cards);
    }
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      if let Some(panel) = &mut app.panel {
        panel.view.refresh_recent(rows.clone(), &cards);
      }
    }
  }

  fn clean_state(&mut self) -> bool {
    if self.store.is_none()
      || !self.cleanup.poll(&mut self.state, &self.frontend.wake())
    {
      return false;
    }
    self.refresh_recent();
    self.changed();
    if self.tabs.is_empty()
      && let Err(error) = self.flush()
    {
      self.startup_warning = Some(format!("State not saved: {error}"));
    }
    true
  }

  #[cfg(test)]
  fn start_at(&mut self, page: usize) -> Result<()> {
    self.activate(0)?;
    self.app().reading_view().open_page(PageId(page - 1))
  }

  /// Records the focused pane and new history in memory, where the
  /// hangup thread finds them too.
  fn record(&mut self) {
    if !self.running() || self.tabs.is_empty() {
      return;
    }
    let tab = &self.tabs[self.active];
    let moved = self.state.remember_pane(&tab.panes[tab.focus]);
    if moved || self.history.revision() != self.recorded {
      self.changed();
    }
  }

  /// Marks state unsaved and hands the hangup thread a copy.
  fn changed(&mut self) {
    self.unsaved = true;
    self.recorded = self.history.revision();
    if let Ok(mut latest) = self.latest.lock() {
      *latest = Some((self.state.clone(), self.history.clone()));
    }
  }

  /// Writes recorded positions; returns whether it wrote.
  fn flush(&mut self) -> Result<bool> {
    if !self.unsaved {
      return Ok(false);
    }
    self.save()?;
    self.unsaved = false;
    Ok(true)
  }

  /// Records every pane, the focused one last, and saves before exit.
  fn finish(&mut self) -> Result<()> {
    self.clean_state();
    self.printing = None;
    let focused = self.tabs.get(self.active).map(|tab| tab.panes[tab.focus].id);
    let panes = self.tabs.iter().flat_map(|tab| &tab.panes);
    let (last, others): (Vec<_>, Vec<_>) =
      panes.partition(|app| Some(app.id) == focused);
    for app in others.into_iter().chain(last) {
      self.state.remember_pane(app);
    }
    self.save()
  }
}

mod clipboard;
mod dispatch;
mod edges;
mod frame;
mod help;
mod interaction;
mod maintenance;
pub mod native;
mod pointer;
mod printing;
#[cfg(test)]
mod recent_tests;
#[cfg(test)]
mod reference_tests;
mod run;
#[cfg(test)]
mod sidebar_tests;
#[cfg(test)]
mod tests;
mod unlock;
mod windows;

pub use run::view;
