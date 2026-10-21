use crate::Result;
use crate::app::{App, Request, Tabline};
use crate::dnd::Target;
use crate::home::{Home, Outcome};
use crate::pace::Pacer;
use crate::state::{Mark, State};
use crate::trace::Drawable;
use crate::watch::Watcher;
use crossterm::event::{
  KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use vpdf_model::text::{PageId, Position};
use vpdf_pdf::document::PasswordRequired;
use vpdf_terminal::decode::{Event, MouseKind, Size};
use vpdf_terminal::dnd::{Dropped, Step};
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal::{self, At, Cause, Shape, Sight, View, sight};
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
pub struct Session {
  pub tabs: Vec<Tab>,
  pub active: usize,
  pub history: History,
  /// Images every pane of every tab shares.
  pub graphics: Graphics,
  pub quit: bool,
  pub state: State,
  frontend: crate::raster::Frontend,
  unlock: Option<unlock::Unlock>,
  native_opens: std::collections::VecDeque<PathBuf>,
  pending_keys: std::collections::VecDeque<KeyEvent>,
  secret: Option<String>,
  picker: Option<crate::programs::Picker>,
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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Rehover {
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
  Mouse(vpdf_terminal::decode::Mouse),
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
    let state = store.as_deref().map_or_default(State::load);
    Self {
      tabs: Vec::new(),
      active: 0,
      history: History::new(state.commands.clone(), state.searches.clone()),
      quit: false,
      graphics: Graphics::default(),
      state,
      frontend: crate::raster::Frontend::Terminal,
      unlock: None,
      native_opens: std::collections::VecDeque::new(),
      pending_keys: std::collections::VecDeque::new(),
      secret: None,
      picker: None,
      perf: crate::perf::Monitor::default(),
      home: None,
      screen: None,
      watcher: Watcher::default(),
      store,
      hangup: Arc::new(AtomicBool::new(false)),
      latest: crate::state::Latest::default(),
      unsaved: false,
      recorded: 0,
      shown: Vec::new(),
      pointer: Pointer::default(),
      unmeasured: None,
      sight: Sight::Focused,
      observer: None,
      observed: None,
      rehover: Rehover::Settled,
      skimmed: false,
      image_write: None,
      drops: vpdf_terminal::dnd::Drops::default(),
      resizing: None,
    }
  }

  pub fn save(&mut self) -> Result<()> {
    let Some(store) = &self.store else { return Ok(()) };
    let history = &self.history;
    self.state.save(store, history.commands(), history.searches())
  }

  fn start_at(&mut self, page: Option<usize>) -> Result<()> {
    self.activate(0)?;
    if let Some(page) = page {
      let app = self.app();
      let page = PageId(page.saturating_sub(1).min(app.doc.pages.len() - 1));
      app.reading_view().open_page(page)?;
    }
    Ok(())
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
mod interaction;
mod maintenance;
pub mod native;
mod pointer;
mod run;
#[cfg(test)]
mod tests;
mod unlock;
mod windows;

pub use run::view;
