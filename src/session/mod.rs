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
use vpdf_terminal::decode::{Event, MouseKind, Reply, Size};
use vpdf_terminal::dnd::{Dropped, Step};
use vpdf_terminal::graphics::Graphics;
use vpdf_terminal::terminal::{self, At, Shape, View};
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
  perf: crate::perf::Monitor,
  /// Start screen while no document is open.
  pub home: Option<Home>,
  /// The whole terminal; panes divide it.
  screen: Option<Screen>,
  watcher: Watcher,
  store: Option<PathBuf>,
  /// Set by SIGHUP or SIGTERM, so the session ends and saves.
  hangup: Arc<AtomicBool>,
  latest: crate::hangup::Latest,
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
  image_write: Option<ImageWrite>,
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

struct ImageWrite {
  pane: u32,
  capture: vpdf_ui::region::CaptureId,
  state: ImageState,
}

enum ImageState {
  Waiting(Instant),
  TimedOut,
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

/// What the reader knows of tmux showing its pane. tmux sends focus
/// events only to a window's active pane, so an unfocused pane asks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Sight {
  /// Takes keys, or tmux is not asked: a focus event says what changes.
  #[default]
  Focused,
  /// Shown without keys; the idle tick asks tmux whether it still is.
  Unfocused,
  /// tmux shows something else over it, such as another window or a
  /// popup: frames wait, since tmux leaves placements over it.
  Hidden,
}

/// Why the reader asks tmux how it shows the pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cause {
  Start,
  FocusOut,
  Tick,
}

/// The sight after `cause`, given tmux's `view` of the pane. A focus-out
/// that leaves the pane active with a focused client is a popup or
/// menu. Without the OS focus the client tells nothing of a window
/// switch another client or a script makes, so the pane asks.
fn sight(now: Sight, cause: Cause, view: View) -> Sight {
  match (cause, view) {
    (_, View::Hidden) | (Cause::FocusOut, View::Active { focused: true }) => {
      Sight::Hidden
    }
    (_, View::Inactive)
    | (Cause::Start | Cause::FocusOut, View::Active { focused: false }) => {
      Sight::Unfocused
    }
    (Cause::Start, View::Active { focused: true }) => Sight::Focused,
    (Cause::Tick, View::Active { .. }) => now,
  }
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
      perf: crate::perf::Monitor::default(),
      home: None,
      screen: None,
      watcher: Watcher::default(),
      store,
      hangup: Arc::new(AtomicBool::new(false)),
      latest: crate::hangup::Latest::default(),
      unsaved: false,
      recorded: 0,
      shown: Vec::new(),
      pointer: Pointer::default(),
      unmeasured: None,
      sight: Sight::Focused,
      observer: None,
      observed: None,
      rehover: Rehover::Settled,
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

mod dispatch;
mod edges;
mod frame;
mod pointer;
mod run;
mod tabs;
#[cfg(test)]
mod tests;
mod windows;

pub use run::view;
