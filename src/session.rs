use crate::Result;
use crate::app::{App, InputOwner, Request, TabMove, Tabline, Window};
use crate::decode::{Event, MouseKind, Reply, Size};
use crate::dnd::{Dropped, Step, Target};
use crate::document::PasswordRequired;
use crate::graphics::Graphics;
use crate::home::{Home, Outcome};
use crate::keys::PanelKind;
use crate::layout::Screen;
use crate::pace::Pacer;
use crate::pointer::{self, Hit, Hover, Pane, Pixel, Pointer, Region, Route};
use crate::prompt::History;
use crate::split::{Axis, Direction, Divider, Node, Place, Rect, neighbor};
use crate::state::{Mark, State};
use crate::terminal::{self, At, Shape, View};
use crate::text::{PageId, Position};
use crate::theme;
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

/// Panes of one tab, laid out by a split tree.
pub struct Tab {
  pub panes: Vec<App>,
  pub tree: Node,
  pub focus: usize,
  /// Pane focused before the current one, for `Ctrl-w p`.
  last: Option<u32>,
}

impl Tab {
  fn new(app: App) -> Self {
    Self { tree: Node::Leaf(app.id), panes: vec![app], focus: 0, last: None }
  }

  fn index(&self, id: u32) -> Option<usize> {
    self.panes.iter().position(|pane| pane.id == id)
  }

  /// Pane ids in screen order.
  pub fn order(&self) -> Vec<u32> {
    self.tree.leaves()
  }

  fn detach(&mut self, area: Rect) -> App {
    let id = self.panes[self.focus].id;
    let mut rects = Vec::new();
    self.tree.layout(area, &mut rects);
    let peer =
      self.last.filter(|peer| *peer != id && self.index(*peer).is_some());
    let next = peer.or_else(|| {
      [Direction::Right, Direction::Down, Direction::Left, Direction::Up]
        .into_iter()
        .find_map(|direction| neighbor(&rects, id, direction))
    });
    let app = self.panes.remove(self.focus);
    self.tree.remove(id);
    self.focus = next.and_then(|id| self.index(id)).unwrap_or(0);
    self.last = None;
    app
  }

  /// Runs `work` on pane `index` while it holds the shared graphics,
  /// which otherwise stay with the focused pane.
  fn lend<R>(&mut self, index: usize, work: impl FnOnce(&mut App) -> R) -> R {
    if index == self.focus {
      return work(&mut self.panes[index]);
    }
    let mut graphics = std::mem::take(&mut self.panes[self.focus].graphics);
    graphics.doc = self.panes[index].id;
    self.panes[index].graphics = graphics;
    let result = work(&mut self.panes[index]);
    let mut graphics = std::mem::take(&mut self.panes[index].graphics);
    graphics.doc = self.panes[self.focus].id;
    self.panes[self.focus].graphics = graphics;
    result
  }
}

/// Open documents as tabs of split panes. The focused pane of the active
/// tab holds the shared Kitty image cache and lends it while others draw.
pub struct Session {
  pub tabs: Vec<Tab>,
  pub active: usize,
  pub history: History,
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
  unmeasured: Option<Unmeasured>,
  sight: Sight,
  rehover: Rehover,
  image_write: Option<ImageWrite>,
  /// The drag over the window, OSC 72.
  drops: crate::dnd::Drops,
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
  capture: crate::region::CaptureId,
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

/// A resize without pixel metrics keeps the old layout, whose status
/// row a shrink can push off the terminal; the error goes on the last
/// row of the terminal's grid instead.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Unmeasured {
  rows: u16,
  columns: u16,
  error: String,
}

impl Unmeasured {
  /// Keeps the cursor where an open prompt put it.
  fn draw(&self, out: &mut impl Write) -> Result<()> {
    write!(out, "\x1b7{}", theme::ERROR)?;
    let row = self.rows.max(1) - 1;
    terminal::line(out, At::default(), row, &self.error, self.columns)?;
    write!(out, "{}\x1b8", theme::RESET)?;
    Ok(())
  }
}

impl Session {
  /// `store` holds recent documents and history between runs.
  pub fn new(store: Option<PathBuf>) -> Self {
    let state = store.as_deref().map_or_default(State::load);
    Self {
      tabs: Vec::new(),
      active: 0,
      history: state.history(),
      quit: false,
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
      rehover: Rehover::Settled,
      image_write: None,
      drops: crate::dnd::Drops::default(),
      resizing: None,
    }
  }

  fn content(&self, mut screen: Screen) -> Screen {
    if self.state.perf {
      let rows = crate::perf::rows(screen);
      screen.rows -= rows;
      screen.origin.1 += rows;
    }
    screen
  }

  fn area(&self) -> Rect {
    self.screen.map_or_default(|screen| {
      let content = self.content(screen);
      Rect { row: content.origin.1 - screen.origin.1, ..full(content) }
    })
  }

  fn toggle_perf(&mut self, on: Option<bool>) -> Result<()> {
    self.state.set_perf(on.unwrap_or(!self.state.perf));
    self.changed();
    self.perf.running(self.state.perf && self.sight != Sight::Hidden);
    if let Some(screen) = self.screen {
      self.resize(screen)?;
    }
    Ok(())
  }

  fn perf_tick(&mut self, out: &mut impl Write) -> Result<()> {
    let on = self.state.perf && self.sight != Sight::Hidden;
    self.perf.running(on);
    if !on {
      return Ok(());
    }
    let now = Instant::now();
    if !self.perf.wait(now).is_zero() {
      return Ok(());
    }
    self.perf.sample(now);
    if let Some(screen) = self.screen {
      let mut buffer = Vec::new();
      write!(buffer, "{}", terminal::frame_start())?;
      if self.perf.draw(&mut buffer, screen, false)? {
        write!(buffer, "{}", terminal::frame_end())?;
        out.write_all(&buffer)?;
        out.flush()?;
      }
    }
    Ok(())
  }

  pub fn save(&mut self) -> Result<()> {
    let Some(store) = &self.store else { return Ok(()) };
    self.state.save(store, &self.history)
  }

  /// Records the focused pane and new history in memory, where the
  /// hangup thread finds them too.
  fn record(&mut self) {
    if !self.running() || self.tabs.is_empty() {
      return;
    }
    let tab = &self.tabs[self.active];
    let moved = self.state.remember(&tab.panes[tab.focus]);
    if moved || self.history.revision != self.recorded {
      self.changed();
    }
  }

  /// Marks state unsaved and hands the hangup thread a copy.
  fn changed(&mut self) {
    self.unsaved = true;
    self.recorded = self.history.revision;
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
      self.state.remember(app);
    }
    self.save()
  }

  pub fn tab(&mut self) -> &mut Tab {
    &mut self.tabs[self.active]
  }

  pub fn app(&mut self) -> &mut App {
    let tab = self.tab();
    &mut tab.panes[tab.focus]
  }

  fn take_graphics(&mut self) -> Graphics {
    let Some(tab) = self.tabs.get_mut(self.active) else {
      return Graphics::default();
    };
    let focus = tab.focus;
    tab
      .panes
      .get_mut(focus)
      .map_or_default(|app| std::mem::take(&mut app.graphics))
  }

  /// Focuses a pane and blurs the old one.
  fn focus(&mut self, tab: usize, pane: usize) -> Result<()> {
    if self.active == tab && self.tabs[tab].focus == pane {
      return Ok(());
    }
    self.leave()?;
    let graphics = self.take_graphics();
    let target = &mut self.tabs[tab];
    if target.focus != pane {
      target.last = Some(target.panes[target.focus].id);
    }
    target.focus = pane;
    self.active = tab;
    self.give(graphics);
    // A rest from before focus is stale.
    self.app().calm_hover();
    self.arrange()
  }

  fn leave(&mut self) -> Result<()> {
    if let Some(tab) = self.tabs.get_mut(self.active)
      && let Some(app) = tab.panes.get_mut(tab.focus)
    {
      app.blur()?;
    }
    self.pointer.release();
    Ok(())
  }

  fn give(&mut self, mut graphics: Graphics) {
    let app = self.app();
    graphics.doc = app.id;
    app.graphics = graphics;
    app.requested.clear();
  }

  /// Sizes every pane of the active tab and marks the focused one.
  fn arrange(&mut self) -> Result<()> {
    let Some(screen) = self.screen else { return Ok(()) };
    let showing = self.tabs[self.active].order();
    for id in std::mem::replace(&mut self.shown, showing.clone()) {
      if showing.contains(&id) {
        continue;
      }
      let hidden = self.tabs.iter_mut().flat_map(|tab| &mut tab.panes);
      if let Some(app) = hidden.into_iter().find(|app| app.id == id) {
        app.trim();
      }
    }
    let names =
      self.tabs.iter().map(|tab| tab.panes[tab.focus].name()).collect();
    let active = self.active;
    let area = self.area();
    let tab = &mut self.tabs[active];
    let mut rects = Vec::new();
    tab.tree.layout(area, &mut rects);
    let mut tabline = Some(Tabline { names, active });
    for (id, rect) in rects {
      let Some(index) = tab.index(id) else { continue };
      let focused = index == tab.focus;
      let app = &mut tab.panes[index];
      // Another tab or pane may have drawn over these rows. After a
      // resize the clear also erases images `WezTerm` keeps in cells,
      // which it no longer deletes by placement id.
      app.blank = false;
      app.focused = focused;
      app.tabline = if focused {
        tabline.take().unwrap_or_default()
      } else {
        Tabline::default()
      };
      let pane = Screen {
        columns: rect.columns,
        rows: rect.rows,
        origin: (screen.origin.0 + rect.column, screen.origin.1 + rect.row),
        ..screen
      };
      let at = At { column: rect.column, row: rect.row };
      let before = (app.screen.columns, app.screen.rows, app.screen.origin);
      if before != (pane.columns, pane.rows, pane.origin) || app.at != at {
        // Hint labels and link previews sit at the old size.
        app.hints = None;
        app.peek = None;
        app.context_menu = None;
        app.screen = pane;
        app.at = at;
        app.requested.clear();
        app.relayout()?;
      }
    }
    Ok(())
  }

  fn existing(&self, path: &Path) -> Option<(usize, usize)> {
    if let Some(tab) = self.tabs.get(self.active)
      && tab.panes[tab.focus].path == path
    {
      return Some((self.active, tab.focus));
    }
    self.tabs.iter().enumerate().find_map(|(t, tab)| {
      tab.panes.iter().position(|app| app.path == path).map(|p| (t, p))
    })
  }

  /// Opens `path` in a new tab, or in place of the focused pane.
  pub fn open(&mut self, path: &Path, tab: bool, screen: Screen) -> Result<()> {
    self.screen = Some(screen);
    let path = std::fs::canonicalize(path)?;
    let found = self.existing(&path);
    if let Some((tab, pane)) = found {
      return self.focus(tab, pane);
    }
    let app = self.load(&path, "", screen)?;
    self.leave()?;
    let mut graphics = self.take_graphics();
    if tab || self.tabs.is_empty() {
      return self.insert_tab(app, graphics);
    }
    let tab = self.tab();
    let focus = tab.focus;
    let old = std::mem::replace(&mut tab.panes[focus], app);
    let id = tab.panes[focus].id;
    replace_leaf(&mut tab.tree, old.id, id);
    self.retire(&old, &mut graphics);
    self.give(graphics);
    self.arrange()
  }

  /// Loads `path` and restores its saved view.
  fn load(
    &mut self,
    path: &Path,
    password: &str,
    screen: Screen,
  ) -> Result<App> {
    let mut secret = password.to_owned();
    let mut app = loop {
      match App::with_screen(path, &secret, false, screen) {
        Err(error) if error.is::<PasswordRequired>() => {
          secret = crate::prompt::password()?;
        }
        result => break result?,
      }
    };
    self.watcher.add(path);
    if let Some(entry) = self.state.entry(path) {
      app.restore(entry)?;
    }
    self.state.remember(&app);
    self.changed();
    Ok(app)
  }

  /// Saves a closing pane's place, deletes its images, and stops
  /// watching its file.
  fn retire(&mut self, app: &App, graphics: &mut Graphics) {
    graphics.forget(app.id);
    self.watcher.remove(&app.path);
    self.state.remember(app);
  }

  pub fn activate(&mut self, index: usize) -> Result<()> {
    if index == self.active || index >= self.tabs.len() {
      return Ok(());
    }
    let focus = self.tabs[index].focus;
    self.focus(index, focus)
  }

  /// Closes the active tab; the session ends after the last.
  pub fn close(&mut self) -> Result<()> {
    self.close_tab(self.active)
  }

  /// Closes a tab; the last quits.
  fn close_tab(&mut self, index: usize) -> Result<()> {
    if index >= self.tabs.len() {
      return Ok(());
    }
    if index == self.active {
      self.leave()?;
    }
    let mut graphics = self.take_graphics();
    let tab = self.tabs.remove(index);
    for app in &tab.panes {
      self.retire(app, &mut graphics);
    }
    if self.tabs.is_empty() {
      return self.save();
    }
    if index < self.active {
      self.active -= 1;
    }
    self.active = self.active.min(self.tabs.len() - 1);
    self.give(graphics);
    self.arrange()
  }

  /// Closes the focused pane, then its tab when it was the last.
  fn close_pane(&mut self) -> Result<()> {
    if self.tab().panes.len() == 1 {
      return self.close();
    }
    self.leave()?;
    let area = self.area();
    let mut graphics = self.take_graphics();
    let app = self.tab().detach(area);
    self.retire(&app, &mut graphics);
    self.give(graphics);
    self.arrange()
  }

  /// Splits the focused pane; the new pane shows `file` or the same view.
  fn split(
    &mut self,
    axis: Axis,
    file: Option<&Path>,
    place: Place,
  ) -> Result<()> {
    // The same document reuses the password that unlocked it.
    let (path, password) = match file {
      Some(file) => (std::fs::canonicalize(file)?, String::new()),
      None => (self.app().path.clone(), self.app().password.clone()),
    };
    if file.is_some()
      && let Some((tab, pane)) = self.existing(&path)
    {
      return self.focus(tab, pane);
    }
    let screen = self.app().screen;
    anyhow::ensure!(Node::fits(axis, full(screen)), "Not enough room");
    let mut app = self.load(&path, &password, screen)?;
    if file.is_none() {
      app.mirror(self.app())?;
    }
    let tab = self.tab();
    let target = tab.panes[tab.focus].id;
    tab.tree.split(target, axis, app.id, place);
    tab.panes.push(app);
    let index = tab.panes.len() - 1;
    self.focus(self.active, index)
  }

  /// A split beside, placed by `land`; focus stays.
  fn split_beside(
    &mut self,
    land: impl FnOnce(&mut App) -> Result<()>,
  ) -> Result<()> {
    let (tab, pane) = (self.active, self.tab().focus);
    // A double click must not split twice.
    self.keeping_pointer(|session| {
      session.split(Axis::Columns, None, Place::After)?;
      // Focus returns even when landing fails.
      let landed = land(session.app());
      session.focus(tab, pane)?;
      landed
    })
  }

  /// Keeps the click run across `work`.
  fn keeping_pointer<R>(&mut self, work: impl FnOnce(&mut Self) -> R) -> R {
    let pointer = std::mem::take(&mut self.pointer);
    let result = work(self);
    self.pointer = pointer;
    result
  }

  fn window(
    &mut self,
    command: Window,
    (count, explicit): (usize, bool),
    file: Option<&Path>,
  ) -> Result<()> {
    let area = self.area();
    let tab = self.tab();
    let id = tab.panes[tab.focus].id;
    let order = tab.order();
    let at = order.iter().position(|pane| *pane == id).unwrap_or(0);
    let total = order.len();
    let target = match command {
      Window::Split(axis) => return self.split(axis, file, Place::Before),
      Window::Close => return self.close_pane(),
      Window::Only => return self.only(),
      Window::ToTab => return self.move_to_tab(),
      Window::Focus(direction) => {
        let mut rects = Vec::new();
        tab.tree.layout(area, &mut rects);
        neighbor(&rects, id, direction)
      }
      Window::Next if explicit => Some(order[count.clamp(1, total) - 1]),
      Window::Next => Some(order[(at + count) % total]),
      Window::Previous => Some(order[(at + total - count % total) % total]),
      Window::Last => tab.last,
      Window::Equalize => {
        tab.tree.equalize();
        None
      }
      Window::Resize(axis, step) => {
        let cells = step * i32::try_from(count).unwrap_or(1);
        tab.tree.resize(id, axis, cells, area);
        None
      }
      Window::Maximize(axis) => {
        tab.tree.resize(id, axis, i32::from(u16::MAX), area);
        None
      }
      Window::Exchange => {
        tab.tree.exchange(id);
        None
      }
      Window::Rotate(forward) => {
        anyhow::ensure!(
          tab.tree.rotate(id, forward),
          "Cannot rotate when another pane is split"
        );
        None
      }
      Window::New => {
        self.record();
        self.split(Axis::Rows, None, Place::Before)?;
        let recent = self.state.recent.clone();
        return self.app().show_recent(&recent);
      }
      Window::Move(direction) => {
        tab.tree.move_to_edge(id, direction);
        None
      }
    };
    match target.and_then(|id| self.tab().index(id)) {
      Some(index) => self.focus(self.active, index),
      None => self.arrange(),
    }
  }

  /// `Ctrl-w o`: closes every pane but the focused one.
  fn only(&mut self) -> Result<()> {
    if self.tab().panes.len() == 1 {
      return Ok(());
    }
    self.leave()?;
    let tab = self.tab();
    let keep = tab.panes[tab.focus].id;
    let closing: Vec<_> =
      tab.panes.extract_if(.., |app| app.id != keep).collect();
    tab.tree = Node::Leaf(keep);
    tab.focus = 0;
    tab.last = None;
    let mut graphics = self.take_graphics();
    for app in &closing {
      self.retire(app, &mut graphics);
    }
    self.give(graphics);
    self.arrange()
  }

  /// `Ctrl-w T`: moves the focused pane into a new tab.
  fn move_to_tab(&mut self) -> Result<()> {
    if self.tab().panes.len() == 1 {
      return Ok(());
    }
    self.leave()?;
    let area = self.area();
    let graphics = self.take_graphics();
    let app = self.tab().detach(area);
    self.insert_tab(app, graphics)
  }

  /// Puts `app` in a tab after the active one and focuses it.
  fn insert_tab(&mut self, app: App, graphics: Graphics) -> Result<()> {
    let index = if self.tabs.is_empty() { 0 } else { self.active + 1 };
    self.tabs.insert(index, Tab::new(app));
    self.active = index;
    self.give(graphics);
    self.arrange()
  }

  /// Copies the focused view into a new tab covered by recent documents.
  fn recent_tab(&mut self, screen: Screen) -> Result<()> {
    self.record();
    let path = self.app().path.clone();
    let password = self.app().password.clone();
    let mut app = self.load(&path, &password, screen)?;
    app.mirror(self.app())?;
    self.leave()?;
    let graphics = self.take_graphics();
    self.insert_tab(app, graphics)?;
    let recent = self.state.recent.clone();
    self.app().show_recent(&recent)
  }

  fn step(&self, step: TabMove) -> usize {
    let count = self.tabs.len();
    match step {
      TabMove::Next(n) => (self.active + n) % count,
      TabMove::Previous(n) => (self.active + count - n % count) % count,
      TabMove::Go(n) => n.clamp(1, count) - 1,
    }
  }

  /// After a frame reaches the terminal: starts work the shown panes
  /// deferred until their whole view was on screen.
  pub fn drawn(&mut self) {
    if let Some(tab) = self.tabs.get_mut(self.active) {
      tab.panes.iter_mut().for_each(App::start_deferred);
    }
  }

  /// Applies what the focused pane asked for after an input.
  pub fn settle(&mut self) -> Result<()> {
    if self.tabs.is_empty() {
      return Ok(());
    }
    let fallback = self.app().screen;
    if self.app().quit {
      return self.close_pane();
    }
    let screen = self.screen.unwrap_or(fallback);
    match self.app().request.take() {
      Some(Request::Open { path, tab }) => self.open(&path, tab, screen),
      Some(Request::RecentTab) => self.recent_tab(screen),
      Some(Request::PanelOpen { path, position, focus }) => {
        self.panel_open(&path, position, focus, screen)
      }
      Some(Request::PanelMark { name, focus }) => {
        let mark = self.global_mark(name)?;
        self.panel_open(&mark.path, Some(mark.cursor), focus, screen)
      }
      Some(Request::Tab(step)) => self.activate(self.step(step)),
      Some(Request::Window { command, count, explicit, file }) => {
        self.window(command, (count, explicit), file.as_deref())
      }
      Some(Request::Panel(kind)) => {
        self.pointer.release();
        if kind == PanelKind::Recent {
          self.record();
          let recent = self.state.recent.clone();
          return self.app().show_recent(&recent);
        }
        let global = self.state.marks.clone();
        let rows = self.app().mark_rows(&global);
        self.app().show_panel(kind, rows)
      }
      Some(Request::SetMark(name)) => {
        let app = self.app();
        let mark =
          Mark { path: app.path.clone(), cursor: app.reading_position()? };
        self.state.marks.insert(name, mark);
        self.changed();
        Ok(())
      }
      Some(Request::GoMark(name, exact)) => {
        let mark = self.global_mark(name)?;
        self.open(&mark.path, true, screen)?;
        self.app().go_to(mark.cursor, exact)
      }
      Some(Request::SplitAt(position)) => {
        self.split_beside(|app| app.land_at(position))
      }
      Some(Request::SplitTo(target)) => {
        self.split_beside(|app| app.follow(&target))
      }
      // Each key's request applies before the next.
      Some(Request::Keys(keys)) => keys.into_iter().try_for_each(|key| {
        self.type_key(key)?;
        self.settle()
      }),
      Some(Request::SplitMark(name)) => {
        let mark = self.global_mark(name)?;
        if mark.path == self.app().path {
          return self.split_beside(|app| app.land_at(mark.cursor));
        }
        self.panel_open(&mark.path, Some(mark.cursor), false, screen)
      }
      Some(Request::Pick) => match crate::fzf::pick(&std::env::current_dir()?)?
      {
        Some(path) => self.open(&path, false, screen),
        None => Ok(()),
      },
      Some(Request::TabClose(index)) => {
        self.close_tab(index.unwrap_or(self.active))
      }
      Some(Request::QuitAll) => {
        self.quit = true;
        Ok(())
      }
      Some(Request::Perf(on)) => self.toggle_perf(on),
      Some(Request::Mouse(on)) => {
        if !on {
          self.drop_pointer();
          self.pointer_left();
        }
        terminal::capture_mouse(on)
      }
      None => Ok(()),
    }
  }

  fn global_mark(&self, name: char) -> Result<Mark> {
    let mark = self.state.marks.get(&name).cloned();
    mark.ok_or_else(|| anyhow::anyhow!("Mark not set: {name}"))
  }

  fn panel_open(
    &mut self,
    path: &Path,
    position: Option<Position>,
    focus: bool,
    screen: Screen,
  ) -> Result<()> {
    let path = std::fs::canonicalize(path)?;
    let source = (self.active, self.tab().focus);
    // The recent view picks a document for the pane it covers.
    let here = focus && self.app().covered();
    let found = self.existing(&path);
    let target = if let Some(target) = found {
      target
    } else {
      if focus {
        self.open(&path, !here, screen)?;
      } else {
        // Beside the source, which keeps focus.
        self.keeping_pointer(|session| {
          session.split(Axis::Columns, Some(&path), Place::After)
        })?;
      }
      (self.active, self.tab().focus)
    };
    if here && target != source {
      let app = &mut self.tabs[source.0].panes[source.1];
      if let Some(panel) = &mut app.panel {
        panel.focused = false;
      }
      app.relayout()?;
    }
    let app = &mut self.tabs[target.0].panes[target.1];
    if let Some(position) = position {
      app.land_at(position)?;
    }
    app.close_overview();
    if focus {
      app.reader.normal();
      app.reader.cancel();
      if let Some(panel) = &mut app.panel {
        panel.focused = false;
      }
      app.relayout()?;
      self.focus(target.0, target.1)
    } else {
      // A second middle press must still find its run.
      self.keeping_pointer(|session| session.focus(source.0, source.1))
    }
  }

  fn running(&self) -> bool {
    !self.quit
      && !self.hangup.load(Ordering::Relaxed)
      && (!self.tabs.is_empty() || self.home.is_some())
  }

  /// One terminal mouse report; whether it needs a frame.
  pub fn mouse_report(&mut self, report: crate::decode::Mouse) -> Result<bool> {
    self.mouse_report_at(report, Instant::now())
  }

  /// One mouse report that arrived at `now`; whether it needs a frame.
  pub fn mouse_report_at(
    &mut self,
    report: crate::decode::Mouse,
    now: Instant,
  ) -> Result<bool> {
    // Reports in flight after `:mouse off`.
    if !terminal::mouse() {
      return Ok(false);
    }
    let Some(screen) = self.screen else { return Ok(false) };
    // Padding past the grid reports cells beyond it.
    let (column, row) = report.cell(screen.cell());
    let column = column.min(screen.columns.saturating_sub(1));
    let row = row.min(screen.rows.saturating_sub(1));
    if let Some(forward) = report.history() {
      return self.history_button(column, row, forward);
    }
    let Some(mut mouse) = report.cells(screen.cell()) else {
      return Ok(false);
    };
    (mouse.column, mouse.row) = (column, row);
    let pixel =
      report.pixel().map(|(x, y)| Pixel { x, y, cell: screen.cell() });
    self.pointed(mouse, pixel, now)
  }

  /// `Ctrl-o` or `Ctrl-i` where pointed.
  fn history_button(
    &mut self,
    column: u16,
    row: u16,
    forward: bool,
  ) -> Result<bool> {
    let modifiers = crossterm::event::KeyModifiers::NONE;
    let mouse =
      MouseEvent { kind: MouseEventKind::Moved, column, row, modifiers };
    // One button at a time.
    if self.pointer.holding() || self.on_completion(mouse) {
      return Ok(false);
    }
    self.app().calm_hover();
    if self.app().dismiss_click()? {
      self.pointer.release();
      return Ok(true);
    }
    let Some((index, _)) = self.under(mouse) else { return Ok(false) };
    self.app().interrupt();
    self.pointer.release();
    self.focus(self.active, index)?;
    let history =
      crate::keys::Input::plain(crate::keys::Action::History(forward));
    self.app().action(history)?;
    Ok(true)
  }

  /// A report in cells; whether it needs a frame.
  pub fn mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
    self.pointed(mouse, None, Instant::now())
  }

  fn pointed(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<bool> {
    if mouse.kind == MouseEventKind::Moved {
      let before = self.pointer.target();
      let hover = self.hit(mouse, pixel);
      let seen = before.map(Hover::shown) != hover.map(Hover::shown);
      let onto = |hover: Option<Hover>| hover.map(|h| (h.pane, h.hit));
      let arrived = onto(before) != onto(hover);
      let changed = self.pointer.hover(mouse, pixel, hover);
      if arrived {
        self.moved_onto();
      }
      if changed {
        self.show_hover();
      } else if let Some(app) = self.pointed_pane() {
        app.rest_again();
      }
      let left = self.leave_check();
      // Bare motion: a release got lost.
      let app = self.app();
      let dropped = app.drop_held();
      let resized = self.resizing.take().is_some();
      return Ok(dropped || resized || seen || left);
    }
    if let Some(changed) = self.edge_event(mouse, pixel, now)? {
      return Ok(changed);
    }
    let wheel = pointer::Wheel::of(mouse.kind).is_some();
    if wheel && self.app().prompt.is_some() {
      return Ok(false);
    }
    // Presses and releases end rests in every pane.
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
      for app in &mut self.tab().panes {
        app.calm_hover();
      }
    }
    // A double click splits once.
    if mouse.kind == MouseEventKind::Down(MouseButton::Middle)
      && self.pointer.repeats(mouse, pixel, now)
    {
      return Ok(false);
    }
    // A double click keeps a note open.
    if mouse.kind == MouseEventKind::Down(MouseButton::Left)
      && self.app().on_anchor(mouse, pixel)
      && self.pointer.repeats(mouse, pixel, now)
    {
      return Ok(false);
    }
    if matches!(mouse.kind, MouseEventKind::Down(_))
      && let Some(drawn) = self.press_first(mouse, pixel, now)?
    {
      return Ok(drawn);
    }
    let tab = &self.tabs[self.active];
    // Snapshot before disarm hides clue rows.
    let panes: Vec<_> = tab.panes.iter().map(Pane::of).collect();
    let focused = tab.panes[tab.focus].id;
    let route = self.pointer.route(&panes, focused, mouse, pixel, now);
    let gesture = wheel && self.pointer.turn(mouse, pixel, now);
    let interrupts = pointer::interrupts(mouse.kind)
      && !(wheel
        && route.as_ref().is_some_and(|route| route.region == Region::Overlay));
    let app = self.app();
    // Disarming changes the status line and hides clue rows.
    let armed = interrupts
      && (!app.keys.pending().is_empty()
        || app.clues.is_some()
        || app.reader.operator.is_some());
    if interrupts {
      app.disarm();
    }
    let Some(route) = route else { return Ok(armed) };
    let Some(index) = self.tab().index(route.pane) else { return Ok(armed) };
    if route.focus {
      // Focusing releases captures; keep this one.
      let active = self.active;
      self.keeping_pointer(|session| session.focus(active, index))?;
    }
    let app = &mut self.tab().panes[index];
    // Per-pixel drags; edges scroll.
    let reading = |app: &App| {
      let menu = app.context_menu.as_ref().map(|menu| menu.selected);
      let held = (app.grabbing(), app.scrub_row());
      let view = (app.view_key(), app.layout.pan.to_bits(), held);
      (app.reader.cursor, app.reader.selection, view, app.gesture, menu)
    };
    let before =
      matches!(mouse.kind, MouseEventKind::Drag(_)).then(|| reading(app));
    if !gesture {
      app.wheel = None;
    }
    // Turns past an end move nothing.
    let view = wheel.then(|| app.wheel_view());
    // Like keys, presses clear old notes.
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      app.clear_notes();
    }
    app.mouse(route)?;
    let moved = view.is_none_or(|view| view != app.wheel_view());
    let read = before.is_none_or(|before| before != reading(app));
    let changed = read && moved;
    Ok(armed || changed)
  }

  /// Presses taken before panes; whether drawn.
  fn press_first(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<Option<bool>> {
    // One button at a time.
    if self.pointer.busy(mouse) {
      return Ok(Some(false));
    }
    // A new left press: release lost.
    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
      self.resizing = None;
    }
    self.press_hover_pane(mouse)?;
    let app = self.app();
    if app.item_press(mouse)? || app.context_press(mouse) {
      self.pointer.release();
      return Ok(Some(true));
    }
    // Blank completion rows stay; clue rows close.
    if self.on_completion(mouse) {
      return Ok(Some(false));
    }
    if self.app().preview_press(mouse)? {
      if mouse.kind == MouseEventKind::Down(MouseButton::Middle) {
        // Its run drops a double click's repeat.
        let tab = self.tab();
        let panes: Vec<_> = tab.panes.iter().map(Pane::of).collect();
        let focused = tab.panes[tab.focus].id;
        self.pointer.route(&panes, focused, mouse, pixel, now);
      } else {
        self.pointer.release();
      }
      return Ok(Some(true));
    }
    if self.app().dismiss_click()? {
      self.pointer.release();
      return Ok(Some(true));
    }
    if self.status_press(mouse)? {
      return Ok(Some(true));
    }
    if let Some(edge) = self.edge_at(mouse.column, mouse.row) {
      // Keys and notes end, as on blanks.
      self.app().interrupt();
      // Boundaries never focus; left presses drag.
      if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return Ok(Some(true));
      }
      return self.edge_press(edge, mouse, pixel, now).map(Some);
    }
    Ok(None)
  }

  /// The boundary at a cell.
  fn edge_at(&self, column: u16, row: u16) -> Option<Edge> {
    let tab = self.tabs.get(self.active)?;
    let mut dividers = Vec::new();
    tab.tree.dividers(self.area(), &mut dividers);
    let on = |axis| {
      let mut found = dividers.iter().filter(|divider| divider.axis == axis);
      found.find(|divider| divider.rect.contains(column, row)).copied()
    };
    if let Some(divider) = on(Axis::Columns) {
      return Some(Edge::Split(divider));
    }
    // The panel's last column, beside the page.
    let border = tab.panes.iter().map(Pane::of).find(|pane| {
      let panel = pane.area(Region::Panel);
      let edge = Rect {
        column: pane.rect.column + panel.columns.saturating_sub(2),
        row: pane.rect.row,
        columns: panel.columns.min(2),
        rows: panel.rows,
      };
      (1..pane.rect.columns).contains(&pane.panel) && edge.contains(column, row)
    });
    if let Some(pane) = border {
      return Some(Edge::Panel(pane.id));
    }
    // Status items stay clickable.
    let item = || {
      tab.panes.iter().any(|app| {
        let pane = Pane::of(app).rect;
        pane.contains(column, row)
          && app.chrome_at(column - pane.column, row - pane.row).is_some()
      })
    };
    on(Axis::Rows).filter(|_| !item()).map(Edge::Split)
  }

  /// Boundary drags and releases; whether drawn.
  fn edge_event(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<Option<bool>> {
    let Some(edge) = self.resizing else { return Ok(None) };
    let release = match mouse.kind {
      MouseEventKind::Drag(MouseButton::Left) => false,
      MouseEventKind::Up(MouseButton::Left) => true,
      _ => return Ok(None),
    };
    // Ends the capture; keeps the spot.
    self.pointer.route(&[], 0, mouse, pixel, now);
    if release {
      self.resizing = None;
      return Ok(Some(true));
    }
    // Moves past the threshold, as pane drags.
    if !self.pointer.dragged() {
      return Ok(Some(false));
    }
    self.drag_edge(edge, mouse).map(Some)
  }

  /// A draggable boundary, as a hover.
  fn edge_hover(&mut self, mouse: MouseEvent) -> Option<Hover> {
    // Overlays and held presses take the press.
    let free = !self.tabs.is_empty()
      && !self.pointer.holding()
      && !self.app().overlaid();
    let edge = self.edge_at(mouse.column, mouse.row).filter(|_| free)?;
    let pane = match edge {
      Edge::Split(_) => 0,
      Edge::Panel(id) => id,
    };
    let region = Region::Edge;
    Some(Hover { pane, region, hit: edge.hit(), beneath: None })
  }

  /// Starts a drag; double clicks equalize.
  fn edge_press(
    &mut self,
    edge: Edge,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<bool> {
    let clicks = self.pointer.press_edge(mouse, pixel, now);
    self.resizing = Some(edge);
    if clicks < 2 {
      return Ok(true);
    }
    let area = self.area();
    match edge {
      Edge::Split(divider) => self.tab().tree.equalize_at(divider.index, area),
      Edge::Panel(id) => {
        if let Some(index) = self.tab().index(id) {
          let app = &mut self.tab().panes[index];
          app.panel_width = None;
          app.relayout()?;
        }
      }
    }
    self.arrange()?;
    Ok(true)
  }

  /// Moves the dragged boundary to the pointer.
  fn drag_edge(&mut self, edge: Edge, mouse: MouseEvent) -> Result<bool> {
    let area = self.area();
    match edge {
      Edge::Split(divider) => {
        let at = match divider.axis {
          Axis::Columns => mouse.column,
          Axis::Rows => mouse.row,
        };
        let tree = &mut self.tab().tree;
        let mut before = Vec::new();
        tree.layout(area, &mut before);
        tree.drag(divider.index, at, area);
        let mut after = Vec::new();
        tree.layout(area, &mut after);
        if before == after {
          return Ok(false);
        }
        self.arrange()?;
      }
      Edge::Panel(id) => {
        let Some(index) = self.tab().index(id) else { return Ok(false) };
        let app = &mut self.tab().panes[index];
        let left = app.left;
        app.panel_width = Some(mouse.column.saturating_sub(app.at.column) + 1);
        app.relayout()?;
        // Remembers the width shown, not asked.
        app.panel_width = Some(app.left);
        if app.left == left {
          return Ok(false);
        }
      }
    }
    Ok(true)
  }

  /// Whether the last frame moved the hover.
  pub fn redraw_due(&mut self) -> bool {
    let due = self.rehover == Rehover::Due;
    if due {
      self.rehover = Rehover::Following;
    }
    due
  }

  /// An OSC 72 drag and drop event; whether it needs a frame.
  pub fn drop_event(
    &mut self,
    body: &str,
    out: &mut impl Write,
  ) -> Result<bool> {
    let tab = self.tabs.get(self.active);
    let typing = tab.is_some_and(|tab| tab.panes[tab.focus].prompt.is_some());
    let Some(drop) = self.drop_step(body, typing, out)? else {
      return Ok(false);
    };
    let done =
      drop.target(typing).and_then(|target| self.dropped(&drop, target));
    answer(out, &drop.finish(done.is_ok()))?;
    done.map(|()| true)
  }

  /// Answers an OSC 72 event; a finished drop.
  fn drop_step(
    &mut self,
    body: &str,
    typing: bool,
    out: &mut impl Write,
  ) -> Result<Option<Dropped>> {
    match self.drops.event(body, typing) {
      Step::Wait => Ok(None),
      Step::Reply(reply) => answer(out, &reply).map(|()| None),
      Step::Dropped(drop) => Ok(Some(drop)),
      Step::Failed(error) => {
        answer(out, crate::dnd::CANCEL)?;
        anyhow::bail!("Drop failed: {error}")
      }
    }
  }

  /// Types a drop into the open prompt, or opens its PDF: in the view
  /// that shows it, or the pane under the drop.
  fn dropped(&mut self, drop: &Dropped, target: Target) -> Result<()> {
    let path = match target {
      Target::Text(text) => {
        let tab = &mut self.tabs[self.active];
        return tab.panes[tab.focus].paste(&text, &mut self.history);
      }
      Target::Pdf(path) => std::fs::canonicalize(path)?,
    };
    if let Some((tab, pane)) = self.existing(&path) {
      return self.focus(tab, pane);
    }
    let (column, row) = drop.cell;
    let under = |app: &App| Pane::of(app).rect.contains(column, row);
    // A load that fails says so in the pane the drop chose.
    if let Some(index) = self.tab().panes.iter().position(under) {
      self.focus(self.active, index)?;
    }
    let screen = self.screen.unwrap_or(self.app().screen);
    // While the drop lasts, so a file it only promises still exists.
    self.open(&path, false, screen)
  }

  /// A drop on the start screen: a prompt takes its text, else its PDF
  /// opens in a tab.
  fn home_drop(
    &mut self,
    home: &mut Home,
    body: &str,
    out: &mut impl Write,
  ) -> Result<Landing> {
    let typing = home.typing();
    let drop = match self.drop_step(body, typing, out) {
      Ok(Some(drop)) => drop,
      Ok(None) => return Ok(Landing::Wait),
      Err(error) => {
        home.error = Some(error.to_string());
        return Ok(Landing::Shown);
      }
    };
    let opened = match drop.target(typing) {
      Ok(Target::Text(text)) => {
        home.event(Event::Paste(text), &mut self.history).map(|_| false)
      }
      Ok(Target::Pdf(path)) => self
        .open(&path, true, self.screen.unwrap_or(home.screen))
        .map(|()| true),
      Err(error) => Err(error),
    };
    answer(out, &drop.finish(opened.is_ok()))?;
    match opened {
      Ok(true) => Ok(Landing::Opened),
      Ok(false) => Ok(Landing::Shown),
      Err(error) => {
        home.error = Some(error.to_string());
        Ok(Landing::Shown)
      }
    }
  }

  /// Opens `path` from the start screen in a new tab; whether it did. A
  /// failure shows on the start screen.
  fn leave_home(&mut self, home: &mut Home, path: &Path) -> bool {
    let opened = self.open(path, true, self.screen.unwrap_or(home.screen));
    if let Err(error) = &opened {
      home.error = Some(error.to_string());
    }
    opened.is_ok()
  }

  /// Whether a press hits completions.
  fn on_completion(&mut self, mouse: MouseEvent) -> bool {
    self.app().prompt.is_some()
      && self.focused_in(Region::Overlay, mouse).is_some()
  }

  /// `mouse` routed in the focused pane's `region`.
  fn focused_in(&mut self, region: Region, mouse: MouseEvent) -> Option<Route> {
    let pane = Pane::of(self.app());
    let local = pointer::route(&[pane], pane.id, None, mouse);
    local.filter(|route| route.region == region)
  }

  /// Types `key` into the focused pane.
  fn type_key(&mut self, key: KeyEvent) -> Result<()> {
    let tab = &mut self.tabs[self.active];
    // `App::key` closes the focused one's.
    let focus = tab.focus;
    for (index, app) in tab.panes.iter_mut().enumerate() {
      if index != focus {
        app.drop_hover_preview();
      }
    }
    let app = &mut tab.panes[tab.focus];
    let before = app.panel.as_ref().filter(|p| p.focused).map(|p| p.kind);
    app.clear_notes();
    // Keys may change the tree mid-drag.
    self.resizing = None;
    // Clues close even when the key fails.
    let result = app.key(key, &mut self.history);
    app.after_key();
    let after = app.panel.as_ref().filter(|p| p.focused).map(|p| p.kind);
    if before != after {
      self.pointer.release();
    }
    result
  }

  /// Acts on a pressed status item.
  fn status_press(&mut self, mouse: MouseEvent) -> Result<bool> {
    let MouseEventKind::Down(button) = mouse.kind else { return Ok(false) };
    let Some((index, route)) = self.under(mouse) else { return Ok(false) };
    if route.region != Region::Status {
      return Ok(false);
    }
    let app = &self.tabs[self.active].panes[index];
    let item = app.chrome_at(route.mouse.column, route.mouse.row);
    let Some(item) = item else { return Ok(false) };
    self.focus(self.active, index)?;
    let app = self.app();
    app.interrupt();
    app.chrome_press(item, button)?;
    self.pointer.release();
    Ok(true)
  }

  /// Pane index and route under `mouse`.
  fn under(&mut self, mouse: MouseEvent) -> Option<(usize, Route)> {
    let tab = self.tabs.get(self.active)?;
    let panes: Vec<_> = tab.panes.iter().map(Pane::of).collect();
    let route = pointer::route(&panes, tab.panes[tab.focus].id, None, mouse)?;
    Some((self.tab().index(route.pane)?, route))
  }

  /// The pointer left the window; whether a hover ended.
  pub fn pointer_left(&mut self) -> bool {
    let changed = self.pointer.leave();
    self.show_hover();
    self.leave_check() || changed
  }

  /// Closes hover previews the pointer left.
  fn leave_check(&mut self) -> bool {
    let spot = self.pointer.spot().map(|(mouse, _)| mouse);
    let Some(tab) = self.tabs.get_mut(self.active) else { return false };
    let mut left = false;
    for app in &mut tab.panes {
      if app.hover_peek().is_none() {
        continue;
      }
      let pane = Pane::of(app);
      let route =
        spot.and_then(|mouse| pointer::route(&[pane], pane.id, None, mouse));
      let at = route.map(|route| (route.mouse.column, route.mouse.row));
      left |= app.hover_leave(at);
    }
    left
  }

  /// Inside another pane's preview focuses it; outside closes it.
  fn press_hover_pane(&mut self, mouse: MouseEvent) -> Result<()> {
    let tab = self.tab();
    let shows = tab.panes.iter().position(|app| app.hover_peek().is_some());
    let Some(index) = shows.filter(|index| *index != tab.focus) else {
      return Ok(());
    };
    let app = &mut tab.panes[index];
    if app.in_preview(mouse) {
      return self.focus(self.active, index);
    }
    app.drop_hover_preview();
    Ok(())
  }

  /// Opens a due hover preview, if any.
  pub fn hover_preview(&mut self, now: Instant) -> bool {
    !self.pointer.holding()
      && self.resting().is_some_and(|app| app.hover_preview(now))
  }

  /// The pane under the pointer.
  fn pointed_pane(&mut self) -> Option<&mut App> {
    let pane = self.pointer.target()?.pane;
    let tab = self.tabs.get_mut(self.active)?;
    tab.panes.iter_mut().find(|app| app.id == pane)
  }

  /// The pointed pane, when free to preview.
  fn resting(&mut self) -> Option<&mut App> {
    let tab = self.tabs.get(self.active)?;
    // Keyboard overlays hold rests everywhere.
    let owner = tab.panes[tab.focus].input_owner();
    let free = matches!(
      owner,
      InputOwner::Content | InputOwner::Panel | InputOwner::Overview
    );
    // Previews and recent cards share one picture.
    let pictured = |app: &App| app.peek.is_some() || app.covered();
    if !free || tab.panes.iter().any(pictured) {
      return None;
    }
    self.pointed_pane()
  }

  /// Retests the last spot; the view changed.
  fn rehover(&mut self) {
    if let Some((mouse, pixel)) = self.pointer.spot() {
      let hover = self.hit(mouse, pixel);
      self.pointer.refresh(hover);
    }
    self.show_hover();
    self.leave_check();
  }

  /// Gives each pane its hover.
  fn show_hover(&mut self) {
    let hover = self.pointer.target();
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    for app in &mut tab.panes {
      app.point(hover.filter(|hover| hover.pane == app.id));
    }
  }

  /// Motion picks a menu row or item.
  fn moved_onto(&mut self) {
    let Some(hover) = self.pointer.target() else { return };
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    let app = tab.panes.iter_mut().find(|app| app.id == hover.pane);
    if let Some(app) = app {
      app.moved_onto(hover.hit);
    }
  }

  /// A held gesture's shape, else the hover's.
  fn shape(&self) -> Shape {
    if let Some(edge) = self.resizing {
      return edge.hit().shape();
    }
    if self.focused().grabbing() {
      return Shape::Grabbing;
    }
    if self.tabs[self.active].panes.iter().any(App::sweeping) {
      return Shape::Crosshair;
    }
    self.pointer.target().map_or_default(|hover| hover.hit).shape()
  }

  /// What the pointer rests on at `mouse`.
  fn hit(&mut self, mouse: MouseEvent, pixel: Option<Pixel>) -> Option<Hover> {
    if let Some(hover) = self.edge_hover(mouse) {
      return Some(hover);
    }
    let (index, route) = self.under(mouse)?;
    let Route { pane, region, mouse, .. } = route;
    let app = &mut self.tab().panes[index];
    let (column, row) = (mouse.column, mouse.row);
    // Hover is advisory; failures find nothing.
    let hit = app.hovered(region, column, row, pixel).unwrap_or_default();
    let below = if region.popup() {
      app.hovered(Region::Page, column, row, pixel)
    } else {
      Ok(hit)
    };
    let beneath = match below {
      Ok(Hit::Link(page, index)) => Some((page, index)),
      _ => None,
    };
    Some(Hover { pane, region, hit, beneath })
  }

  /// Fits the start screen or every pane to a new terminal size.
  pub fn resize(&mut self, screen: Screen) -> Result<()> {
    self.perf.invalidate();
    self.screen = Some(screen);
    let content = self.content(screen);
    if let Some(home) = &mut self.home {
      home.at.row = content.origin.1 - screen.origin.1;
      home.screen = content;
    }
    if self.tabs.is_empty() {
      return Ok(());
    }
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.at = At { column: u16::MAX, row: u16::MAX };
    }
    self.arrange()
  }

  /// Fits the start screen or panes to a `measured` screen; when its
  /// cells could not be measured, keeps the layout and says why on the
  /// last row of `grid`, the terminal's columns and rows.
  fn refit(
    &mut self,
    measured: Result<Option<Screen>>,
    grid: impl FnOnce() -> io::Result<(u16, u16)>,
  ) -> Result<()> {
    match measured {
      Ok(Some(screen)) => {
        self.unmeasured = None;
        self.resize(screen)
      }
      Ok(None) => Ok(()),
      Err(error) => {
        let (columns, rows) = grid()?;
        let error = error.to_string();
        self.unmeasured = Some(Unmeasured { rows, columns, error });
        Ok(())
      }
    }
  }

  /// Deletes every placement once tmux shows something else, which they
  /// would cover, and holds frames until tmux shows the pane again.
  pub fn hide(&mut self, out: &mut impl Write) -> Result<()> {
    self.sight = Sight::Hidden;
    self.release_pointer(out)?;
    if self.tabs.is_empty() {
      return self.home.as_mut().map_or(Ok(()), |home| home.release(out));
    }
    self.app().graphics.clear(out)
  }

  /// The pointer may rest elsewhere now.
  fn release_pointer(&mut self, out: &mut impl Write) -> Result<()> {
    self.drop_pointer();
    terminal::hand_over(out)?;
    // Out before the slow `tmux display`.
    out.flush()?;
    Ok(())
  }

  /// Forgets the pointer: hover, held presses, and gestures.
  fn drop_pointer(&mut self) {
    self.pointer.leave();
    self.resizing = None;
    // Its release may never come.
    self.pointer.release();
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.drop_held();
    }
  }

  fn image_message(&mut self, write: &ImageWrite, message: String) {
    if let Some(app) = self
      .tabs
      .iter_mut()
      .flat_map(|tab| &mut tab.panes)
      .find(|app| app.id == write.pane)
    {
      app.region_status(write.capture, message);
    }
  }

  fn image_status(&mut self, body: &str) -> bool {
    let Some(result) = crate::clipboard::image_status(body) else {
      return false;
    };
    let Some(write) = self.image_write.take() else { return false };
    if matches!(write.state, ImageState::Waiting(_)) {
      let message = result
        .map_or_else(|error| error.to_string(), |()| "Image copied".into());
      self.image_message(&write, message);
    }
    true
  }

  fn image_wait(&self) -> Option<Duration> {
    match &self.image_write {
      Some(ImageWrite { state: ImageState::Waiting(until), .. }) => {
        Some(until.saturating_duration_since(Instant::now()))
      }
      _ => None,
    }
  }

  fn image_timeout(&mut self, now: Instant) -> bool {
    let Some(mut write) = self.image_write.take() else { return false };
    let expired =
      matches!(write.state, ImageState::Waiting(until) if now >= until);
    if expired {
      write.state = ImageState::TimedOut;
      self.image_message(&write, "Image clipboard timed out".into());
    }
    self.image_write = Some(write);
    expired
  }

  fn image_updates(&mut self, out: &mut impl Write) -> bool {
    let mut dirty = false;
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      let result = app.region_updates().and_then(|changed| {
        if changed {
          app.clipboard(out)?;
        }
        Ok(changed)
      });
      match result {
        Ok(changed) => dirty |= changed,
        Err(error) => {
          app.error = Some(error.to_string());
          dirty = true;
        }
      }
    }
    let transfers: Vec<_> = self
      .tabs
      .iter_mut()
      .flat_map(|tab| &mut tab.panes)
      .filter_map(|app| app.region_send.take().map(|send| (app.id, send)))
      .collect();
    for (pane, transfer) in transfers {
      dirty = true;
      let write = ImageWrite {
        pane,
        capture: transfer.id,
        state: ImageState::Waiting(Instant::now() + Duration::from_secs(10)),
      };
      if self.image_write.is_some() {
        self.image_message(
          &write,
          "Image clipboard awaiting previous status".into(),
        );
        continue;
      }
      match crate::caps::get().and_then(|caps| caps.clipboard) {
        Some(crate::caps::Support::Answered) => {}
        Some(_) => {
          self.image_message(&write, "Image clipboard unsupported".into());
          continue;
        }
        None => {
          self
            .image_message(&write, "Image clipboard support unanswered".into());
          continue;
        }
      }
      self.image_write = Some(write);
      let result =
        crate::clipboard::image_remote(out, &transfer.png, terminal::tmux())
          .and_then(|()| out.flush().map_err(Into::into));
      if let Err(error) = result {
        let mut write = self.image_write.take().expect("reserved image write");
        write.state = ImageState::TimedOut;
        self.image_message(&write, error.to_string());
        self.image_write = Some(write);
      }
    }
    dirty
  }

  /// Shows the next tmux notice once the status line is free; whether
  /// it did.
  fn notify(&mut self) -> bool {
    let now = Instant::now();
    let mut changed = self.image_timeout(now);
    for app in &mut self.tab().panes {
      if app.landing.as_ref().is_some_and(|landing| now >= landing.until) {
        app.landing = None;
        changed = true;
      }
      changed |= app.bar_expired(now);
    }
    changed |= self.autoscroll(now);
    changed |= self.hover_preview(now);
    let reader = &mut self.app().reader;
    if !reader.message.is_empty() {
      return changed;
    }
    let Some(notice) = terminal::notice() else { return changed };
    reader.message = notice;
    true
  }

  /// Steps the held edge drag; whether drawn.
  pub fn autoscroll(&mut self, now: Instant) -> bool {
    let held = self.pointer.held_pane();
    let mut changed = false;
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      // Lost releases stop it too.
      if held != Some(app.id) {
        app.autoscroll = None;
        continue;
      }
      match app.autoscroll_step(now) {
        Ok(moved) => changed |= moved,
        Err(error) => {
          app.autoscroll = None;
          app.error = Some(error.to_string());
          changed = true;
        }
      }
    }
    changed
  }

  /// When the held edge drag next scrolls.
  pub fn autoscroll_due(&self) -> Option<Instant> {
    let panes = &self.tabs.get(self.active)?.panes;
    panes.iter().find_map(|app| app.autoscroll.map(|held| held.due))
  }

  /// Asks tmux how it shows the pane, for `cause`; whether the reader
  /// shows again and needs a frame.
  fn look(&mut self, cause: Cause, out: &mut impl Write) -> Result<bool> {
    match terminal::pane_view() {
      Some(view) => self.turn(sight(self.sight, cause, view), out),
      // The tick asks again.
      None if cause == Cause::Tick => Ok(false),
      None => self.turn(Sight::Unfocused, out),
    }
  }

  /// Moves to `sight`, hiding on the way in; whether the reader shows
  /// again and needs a frame.
  fn turn(&mut self, sight: Sight, out: &mut impl Write) -> Result<bool> {
    let was = self.sight;
    if sight == was {
      return Ok(false);
    }
    if sight == Sight::Hidden {
      self.hide(out)?;
      return Ok(false);
    }
    self.sight = sight;
    Ok(was == Sight::Hidden)
  }

  /// What `act` proves of tmux showing the pane: only a shown, active
  /// pane gets keys, and only a shown one the pointer. Covers focus
  /// events lost while a prompt, the start screen, or a child read input.
  /// Whether the reader shows again and needs a frame.
  fn shown_by(&mut self, act: &Act, out: &mut impl Write) -> Result<bool> {
    let sight = match act {
      Act::Key(_) => Sight::Focused,
      Act::Mouse(_) if self.sight == Sight::Hidden => Sight::Unfocused,
      _ => return Ok(false),
    };
    self.turn(sight, out)
  }

  /// Uploads image `id` again after the terminal evicted it.
  fn missing(&mut self, id: u32) {
    if !self.app().graphics.missing(id) {
      return;
    }
    // Tiles no longer kept render again.
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.requested.clear();
    }
  }

  /// Drops cached images after another program had the screen, which
  /// also took the focus events tmux sent meanwhile; asks tmux anew,
  /// since it sends no focus state when asked for focus events.
  fn images_lost(&mut self, out: &mut impl Write) -> Result<bool> {
    if !terminal::images_lost() {
      return Ok(false);
    }
    self.perf.invalidate();
    crate::graphics::release(out)?;
    self.app().graphics.lost();
    // The child program read the mouse.
    self.pointer.leave();
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.requested.clear();
      app.blank = false;
    }
    if terminal::focus_reports() {
      self.look(Cause::Start, out)?;
    }
    Ok(true)
  }

  fn draw_home(&mut self, home: &mut Home, out: &mut impl Write) -> Result<()> {
    // Its thumbnail would cover what tmux shows instead.
    if self.sight != Sight::Hidden {
      // Start-screen motion is passive.
      self.pointer.leave();
      terminal::point(out, Shape::Default)?;
      self.perf.running(self.state.perf);
      home.draw(out)?;
      self.perf.frame(Instant::now());
      if self.state.perf
        && let Some(screen) = self.screen
      {
        self.perf.draw(out, screen, false)?;
        out.flush()?;
      }
    }
    if let Some(unmeasured) = &self.unmeasured {
      unmeasured.draw(out)?;
      out.flush()?;
    }
    Ok(())
  }

  /// Draws the start screen and handles one input.
  fn home(&mut self, out: &mut crate::perf::Output<impl Write>) -> Result<()> {
    let Some(mut home) = self.home.take() else { return Ok(()) };
    self.draw_home(&mut home, out)?;
    let event = loop {
      // A wake may come from the hangup thread; the caller checks it.
      self.perf.bytes(Instant::now(), out.take());
      self.perf_tick(out)?;
      let wait = (self.state.perf && self.sight != Sight::Hidden)
        .then(|| self.perf.wait(Instant::now()));
      let Some(event) = crate::input::next(wait)? else {
        if !self.quit
          && !self.hangup.load(Ordering::Relaxed)
          && wait.is_some()
          && !home.updates()
        {
          continue;
        }
        self.home = Some(home);
        return Ok(());
      };
      if let Event::Reply(Reply::Osc { code: 72, body }) = &event {
        match self.home_drop(&mut home, body, out)? {
          Landing::Wait => continue,
          Landing::Opened => return home.release(out),
          Landing::Shown => {
            self.home = Some(home);
            return Ok(());
          }
        }
      }
      if !passive(&event) {
        break event;
      }
    };
    if let Event::Focus(gained) = event {
      self.home = Some(home);
      return if gained {
        self.turn(Sight::Focused, out).map(drop)
      } else {
        self.look(Cause::FocusOut, out).map(drop)
      };
    }
    // Only a shown pane gets keys, and the pointer.
    if matches!(event, Event::Key(_)) {
      self.sight = Sight::Focused;
    }
    if matches!(event, Event::Mouse(_)) && self.sight == Sight::Hidden {
      self.sight = Sight::Unfocused;
    }
    if let Event::Resize(report) = event {
      let measured = terminal::resized(report, self.screen);
      self.home = Some(home);
      let refit = self.refit(measured, crossterm::terminal::size);
      if let (Err(error), Some(home)) = (refit, &mut self.home) {
        home.error = Some(error.to_string());
      }
      return Ok(());
    }
    let outcome = home.event(event, &mut self.history);
    match outcome {
      Ok(Outcome::Open(path)) => {
        if self.leave_home(&mut home, &path) {
          return home.release(out);
        }
      }
      Ok(Outcome::Quit) => return Ok(()),
      Ok(Outcome::Pick) => {
        home.release(out)?;
        let picked = std::env::current_dir()
          .map_err(anyhow::Error::from)
          .and_then(|root| crate::fzf::pick(&root));
        match picked {
          Ok(Some(path)) => {
            if self.leave_home(&mut home, &path) {
              return Ok(());
            }
          }
          Ok(None) => {}
          Err(error) => home.error = Some(error.to_string()),
        }
      }
      Ok(Outcome::Perf(on)) => {
        self.home = Some(home);
        return self.toggle_perf(on);
      }
      Ok(Outcome::Stay) => {}
      Err(error) => home.error = Some(error.to_string()),
    }
    self.home = Some(home);
    Ok(())
  }
}

impl Drawable for Session {
  /// Unfocused panes first, so the focused pane's prompt places the
  /// terminal cursor last.
  fn draw_frame(&mut self, out: &mut Vec<u8>) -> Result<()> {
    for app in &mut self.tab().panes {
      app.drop_stray_preview();
    }
    self.rehover();
    // Without a spot the pointer is elsewhere.
    if self.pointer.spot().is_some() {
      terminal::point(out, self.shape())?;
    }
    write!(out, "{}\x1b[0m", terminal::frame_start())?;
    self.app().graphics.begin(out)?;
    let area = self.area();
    let unmeasured = self.unmeasured.clone();
    let tab = self.tab();
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        tab.lend(index, |app| app.draw_pane(out))?;
      }
    }
    let mut dividers = Vec::new();
    tab.tree.dividers(area, &mut dividers);
    let columns =
      dividers.iter().filter(|divider| divider.axis == Axis::Columns);
    for rect in columns.map(|divider| divider.rect) {
      let at = At { column: rect.column, row: rect.row };
      for row in 0..rect.rows {
        let goto = at.goto(row, 0u16);
        write!(out, "{goto}{}│{}", theme::MUTED, theme::RESET)?;
      }
    }
    let focus = tab.focus;
    tab.panes[focus].draw_pane(out)?;
    if let Some(unmeasured) = unmeasured {
      unmeasured.draw(out)?;
    }
    tab.panes[focus].graphics.end(out)?;
    if self.state.perf
      && let Some(screen) = self.screen
    {
      self.perf.draw(out, screen, false)?;
    }
    write!(out, "{}", terminal::frame_end())?;
    // Drawing can scroll thumbnails or rewrap rows.
    let drawn = self.pointer.target().map(Hover::shown);
    self.rehover();
    let moved = self.pointer.target().map(Hover::shown) != drawn;
    // One follow-up, so hover-drawn regions cannot loop.
    let following = self.rehover == Rehover::Following;
    self.rehover =
      if moved && !following { Rehover::Due } else { Rehover::Settled };
    Ok(())
  }

  fn flushed(&mut self, at: Instant) {
    self.perf.frame(at);
  }

  fn focused(&self) -> &App {
    let tab = &self.tabs[self.active];
    &tab.panes[tab.focus]
  }
}

fn full(screen: Screen) -> Rect {
  Rect { column: 0, row: 0, columns: screen.columns, rows: screen.rows }
}

fn replace_leaf(node: &mut Node, from: u32, to: u32) {
  match node {
    Node::Leaf(id) if *id == from => *id = to,
    Node::Leaf(_) => {}
    Node::Split { first, second, .. } => {
      replace_leaf(first, from, to);
      replace_leaf(second, from, to);
    }
  }
}

/// Opens `paths` at `page`, or the start screen without any.
pub fn view(paths: &[PathBuf], page: Option<usize>) -> Result<()> {
  anyhow::ensure!(io::stdout().is_terminal(), "TUI requires a terminal");
  let mut session = Session::new(crate::state::location());
  crate::hangup::watch(
    session.store.clone(),
    Arc::clone(&session.hangup),
    Arc::clone(&session.latest),
  )?;
  terminal::restore_on_panic();
  let (_terminal, screen) = crate::gate::enter()?;
  // Before any pane sizes its image budget or reads input; a spool that
  // fails to start leaves tiles encoded.
  let _ = crate::spool::start(crate::graphics::session());
  if paths.is_empty() {
    session.screen = Some(screen);
    session.home =
      Some(Home::new(&session.state.recent, session.content(screen)));
    session.resize(screen)?;
    return run_saving(&mut session);
  }
  for path in paths {
    session.open(path, true, screen)?;
  }
  session.activate(0)?;
  let app = session.app();
  if let Some(page) = page {
    app.reader.edge(
      PageId(page.saturating_sub(1).min(app.doc.pages.len() - 1)),
      false,
      &mut app.doc,
    )?;
    app.show_page(app.reader.cursor.page)?;
    app.ensure_cursor()?;
  }
  run_saving(&mut session)
}

/// Runs the session and saves its state even when the loop fails, as
/// after the terminal hangs up.
fn run_saving(session: &mut Session) -> Result<()> {
  let result = run(session);
  let saved = session.finish();
  result.and(saved)
}

/// Background results for every pane of the active tab.
fn updates(session: &mut Session, out: &mut impl Write) -> Result<bool> {
  let mut dirty = session.images_lost(out)?;
  dirty |= session.image_updates(out);
  let tab = session.tab();
  for index in 0..tab.panes.len() {
    dirty |= tab.lend(index, App::raster_updates);
    let app = &mut tab.panes[index];
    let shelf = app.panel.as_mut().and_then(|panel| panel.shelf.as_mut());
    dirty |= shelf.is_some_and(crate::recent::Shelf::updates);
    // A failed update shows in its pane instead of ending the session.
    let updated = app.search_updates().and_then(|search| {
      Ok(search | app.ocr_updates()? | app.outline_updates()?)
    });
    dirty |= updated.unwrap_or_else(|error| {
      app.error = Some(error.to_string());
      true
    });
  }
  Ok(dirty)
}

/// Reloads the active tab's panes after their files change; each holds
/// the shared graphics while it reloads, so its old images go.
fn watch(session: &mut Session, force: bool) -> bool {
  let touched = session.watcher.touched();
  let tab = &mut session.tabs[session.active];
  let mut dirty = false;
  for index in 0..tab.panes.len() {
    let app = &tab.panes[index];
    if !(force || app.settling.is_some() || touched.contains(&app.path)) {
      continue;
    }
    dirty |= tab.lend(index, |app| {
      app.watch().unwrap_or_else(|error| {
        app.error = Some(format!("Reload failed: {error}"));
        true
      })
    });
  }
  dirty
}

/// Pointer input the start screen ignores: motion, drags, leaving.
fn passive(event: &Event) -> bool {
  match event {
    Event::Mouse(mouse) => {
      matches!(mouse.kind, MouseKind::Moved | MouseKind::Drag(_))
    }
    Event::Leave => true,
    _ => false,
  }
}

/// Input the session acts on.
#[derive(Debug, PartialEq)]
enum Act {
  Key(KeyEvent),
  Paste(String),
  /// With the mode 2048 report, when the terminal sent one.
  Resize(Option<Size>),
  Mouse(crate::decode::Mouse),
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
fn act(event: Event) -> Option<Act> {
  match event {
    Event::Key(key) if key.kind != KeyEventKind::Release => Some(Act::Key(key)),
    Event::Paste(text) => Some(Act::Paste(text)),
    Event::Resize(report) => Some(Act::Resize(report)),
    Event::Mouse(mouse) => (mouse.action().is_some()
      || mouse.history().is_some())
    .then_some(Act::Mouse(mouse)),
    Event::Leave => Some(Act::Leave),
    Event::Reply(Reply::Graphics { id: Some(id), message, .. })
      if message.starts_with("ENOENT") =>
    {
      Some(Act::Missing(id))
    }
    Event::Reply(Reply::Osc { code: 5522, body }) => {
      Some(Act::ImageStatus(body))
    }
    Event::Reply(Reply::Osc { code: 72, body }) => Some(Act::Drop(body)),
    Event::Focus(gained) => Some(Act::Focus(gained)),
    Event::Key(_) | Event::Reply(_) => None,
  }
}

/// Handles one input; whether it needs a frame.
fn input(
  session: &mut Session,
  act: Act,
  out: &mut impl Write,
) -> Result<bool> {
  let shown = session.shown_by(&act, out)?;
  let done = match act {
    Act::Key(key) => session.type_key(key),
    Act::Paste(text) => {
      let tab = &mut session.tabs[session.active];
      tab.panes[tab.focus].paste(&text, &mut session.history)
    }
    Act::Resize(report) => {
      let measured = terminal::resized(report, session.screen);
      session.refit(measured, crossterm::terminal::size)
    }
    Act::Mouse(mouse) => return Ok(session.mouse_report(mouse)? || shown),
    Act::Leave => return Ok(session.pointer_left()),
    Act::Drop(body) => return session.drop_event(&body, out),
    Act::ImageStatus(body) => {
      session.image_status(&body);
      Ok(())
    }
    Act::Missing(id) => {
      session.missing(id);
      Ok(())
    }
    Act::Focus(true) => session.turn(Sight::Focused, out).map(drop),
    Act::Focus(false) => session
      .release_pointer(out)
      .and_then(|()| session.look(Cause::FocusOut, out).map(drop)),
  };
  done.map(|()| true)
}

/// Handles and settles one input; whether it needs a frame.
fn apply(session: &mut Session, act: Act, out: &mut impl Write) -> bool {
  let result = input(session, act, out)
    .and_then(|changed| session.settle().map(|()| changed));
  result.unwrap_or_else(|error| {
    report(session, Err(error));
    true
  })
}

/// Positions reach memory this often after a change.
const CHECK: Duration = Duration::from_millis(250);

/// Files are checked this often, in case the OS reports no change.
const WATCH: Duration = Duration::from_secs(2);

fn run(session: &mut Session) -> Result<()> {
  let mut out = crate::perf::Output::new(io::BufWriter::new(io::stdout()));
  let mut trace = crate::trace::Trace::new()?;
  let mut buffer = Vec::with_capacity(64 * 1024);
  let mut dirty = true;
  let mut pacer = Pacer::new(Instant::now());
  let mut chores = Chores::new(Instant::now());
  let mut drawn = Instant::now();
  let mut title = String::new();
  while session.running() {
    if terminal::resize_deferred() {
      let measured = terminal::remeasure().map(Some);
      let resized = session.refit(measured, crossterm::terminal::size);
      report(session, resized);
      dirty = true;
    }
    if session.tabs.is_empty() {
      session.home(&mut out)?;
      dirty = true;
      continue;
    }
    session.perf.bytes(Instant::now(), out.take());
    session.perf_tick(&mut out)?;
    dirty |= updates(session, &mut out)?;
    session.app().clipboard(&mut out)?;
    dirty |= session.notify();
    let now = Instant::now();
    let shown = session.sight != Sight::Hidden;
    if pacer.ready(now) && shown {
      for app in &mut session.tab().panes {
        dirty |= app.animate();
      }
      if dirty {
        let current = session.app().title();
        if current != title {
          set_title(&mut out, &current)?;
          title = current;
        }
        trace.draw(session, &mut out, &mut buffer)?;
        session.drawn();
        dirty = session.redraw_due();
        pacer.drawn(now);
        drawn = now;
      }
    }
    let animating =
      session.tab().panes.iter().any(|app| app.animation.is_some());
    // A hidden reader draws nothing until tmux shows it again.
    let pending = (dirty || animating) && shown;
    let now = Instant::now();
    let changed = chores.touched.max(drawn) > chores.checked;
    let mut wait = if pending {
      pacer.wait(now)
    } else {
      idle(session, chores.checked, chores.watched, changed)
    };
    if session.state.perf && shown {
      wait = wait.min(session.perf.wait(now));
    }
    let event = crate::input::next(Some(wait))?;
    // Streams of pointer reports must not starve them.
    if event.is_none() || now.elapsed() >= wait {
      dirty |= chores.run(session, &mut out)?;
    }
    let Some(act) = event.and_then(act) else { continue };
    let (typed, arrived) = (act.typed(), Instant::now());
    // Motion and per-pixel drags often change nothing.
    if apply(session, act, &mut out) {
      if typed {
        chores.touched = arrived;
        trace.input(arrived);
      }
      dirty = true;
    }
  }
  Ok(())
}

/// When the timed chores last ran, and the last typing.
struct Chores {
  checked: Instant,
  written: Instant,
  touched: Instant,
  watched: Instant,
}

impl Chores {
  fn new(now: Instant) -> Self {
    Self { checked: now, written: now, touched: now, watched: now }
  }

  /// Clues, file checks, tmux looks, and saved positions; whether a
  /// frame is due.
  fn run(
    &mut self,
    session: &mut Session,
    out: &mut impl Write,
  ) -> Result<bool> {
    let mut dirty = session.app().clue_due();
    let force = self.watched.elapsed() >= WATCH;
    if force {
      self.watched = Instant::now();
    }
    dirty |= watch(session, force);
    if force && session.sight != Sight::Focused {
      dirty |= session.look(Cause::Tick, out)?;
    }
    // Positions reach memory 250 ms after a change and disk after a
    // 5 s pause or once a minute; exits and hangups write the rest.
    if self.checked.elapsed() >= CHECK {
      self.checked = Instant::now();
      session.record();
      let due = self.touched.elapsed() > Duration::from_secs(5)
        || self.written.elapsed() > Duration::from_secs(60);
      match if due { session.flush() } else { Ok(false) } {
        Ok(true) => self.written = self.checked,
        Ok(false) => {}
        Err(error) => {
          session.app().error = Some(format!("State not saved: {error}"));
        }
      }
    }
    Ok(dirty)
  }
}

/// Sets the window title, except where the terminal ignores the title
/// stack and would keep it after exit.
fn set_title(out: &mut impl Write, title: &str) -> Result<()> {
  if crate::caps::get().is_none_or(crate::caps::Caps::title_stack) {
    write!(out, "\x1b]2;{}\x07", terminal::clean(title, 200))?;
  }
  Ok(())
}

/// How long the idle loop may sleep: until the next file check, state
/// check after a change, clue, or settled file change.
fn idle(
  session: &mut Session,
  checked: Instant,
  watched: Instant,
  changed: bool,
) -> Duration {
  let mut wait = WATCH.saturating_sub(watched.elapsed());
  if changed || session.unsaved {
    wait = wait.min(CHECK.saturating_sub(checked.elapsed()));
  }
  if let Some(image) = session.image_wait() {
    wait = wait.min(image);
  }
  if let Some(clue) = session.app().clue_wait() {
    wait = wait.min(clue);
  }
  let held = session.pointer.holding();
  let due = session.resting().and_then(App::hover_due);
  if let Some(due) = due.filter(|_| !held) {
    wait = wait.min(due.saturating_duration_since(Instant::now()));
  }
  if let Some(due) = session.autoscroll_due() {
    wait = wait.min(due.saturating_duration_since(Instant::now()));
  }
  for app in &session.tab().panes {
    if let Some(landing) = &app.landing {
      wait = wait.min(landing.until.saturating_duration_since(Instant::now()));
    }
    if let Some(until) = app.bar.until {
      wait = wait.min(until.saturating_duration_since(Instant::now()));
    }
  }
  if session.tab().panes.iter().any(|app| app.settling.is_some()) {
    wait = wait.min(crate::watch::SETTLE);
  }
  wait
}

/// Where a drop on the start screen leaves it.
enum Landing {
  Wait,
  /// A document opened in a tab.
  Opened,
  /// The start screen redraws: its prompt took text, or an error shows.
  Shown,
}

/// Sends a reply the terminal waits for.
fn answer(out: &mut impl Write, reply: &str) -> Result<()> {
  out.write_all(reply.as_bytes())?;
  out.flush()?;
  Ok(())
}

/// Shows a failure in the focused pane instead of ending the session.
fn report(session: &mut Session, result: Result<()>) {
  if let Err(error) = result
    && !session.tabs.is_empty()
  {
    session.app().error = Some(error.to_string());
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::decode::{Button, Mouse, Spot};
  use crate::text::Bounds;
  use crossterm::event::{KeyCode, KeyModifiers};

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn perf_is_global_and_persistent() -> Result<()> {
    let root = tempfile::tempdir()?;
    let store = root.path().join("state.json");
    let mut session = paper()?;
    session.store = Some(store.clone());
    let screen = session.screen.expect("screen");
    session.app().command("perf on")?;
    session.settle()?;
    assert!(session.state.perf);
    let content = screen.rows - crate::perf::rows(screen);
    assert_eq!(session.app().screen.rows, content);
    assert_eq!(session.app().at.row, crate::perf::rows(screen));
    session.app().command("vsplit")?;
    session.settle()?;
    assert!(session.tab().panes.iter().all(|a| a.screen.rows == content));
    let mut out = Vec::new();
    session.draw_frame(&mut out)?;
    let text = String::from_utf8(out)?;
    assert_eq!(text.matches("FPS").count(), 1);
    session.save()?;
    assert!(Session::new(Some(store)).state.perf);
    session.app().command("perf")?;
    session.settle()?;
    assert!(!session.state.perf);
    assert!(session.tab().panes.iter().all(|a| a.screen.rows == screen.rows));
    assert!(session.app().command("perf maybe").is_err());
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn perf_split_respects_focused_pane_size() -> Result<()> {
    let mut session = paper()?;
    session.toggle_perf(Some(true))?;
    for _ in 0..16 {
      let fits = Node::fits(Axis::Columns, full(session.app().screen));
      let count = session.tab().panes.len();
      let result = session.split(Axis::Columns, None, Place::After);
      if !fits {
        assert!(result.is_err());
        assert_eq!(session.tab().panes.len(), count);
        return Ok(());
      }
      result?;
    }
    panic!("splits never reached the minimum pane width");
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn perf_start_screen_reserves_once() -> Result<()> {
    let mut session = paper()?;
    let screen = session.screen.expect("screen");
    session.tabs.clear();
    session.home = Some(Home::new(&[], screen));
    session.toggle_perf(Some(true))?;
    let content = screen.rows - crate::perf::rows(screen);
    assert_eq!(session.home.as_ref().expect("home").screen.rows, content);
    session.toggle_perf(Some(true))?;
    assert_eq!(session.home.as_ref().expect("home").screen.rows, content);
    session.toggle_perf(Some(false))?;
    assert_eq!(session.home.as_ref().expect("home").screen.rows, screen.rows);
    Ok(())
  }

  #[test]
  fn image_timeout_retains_transfer() {
    let mut session = Session::new(None);
    let capture = crate::region::CaptureId::next();
    let now = Instant::now();
    session.image_write = Some(ImageWrite {
      pane: 999,
      capture,
      state: ImageState::Waiting(now + Duration::from_secs(10)),
    });
    assert!(!session.image_timeout(now));
    assert!(session.image_timeout(now + Duration::from_secs(10)));
    assert!(!session.image_timeout(now + Duration::from_secs(20)));
    let original = session.image_write.as_ref().unwrap();
    assert_eq!(original.capture, capture);
    assert!(matches!(original.state, ImageState::TimedOut));
    assert!(!session.image_status("type=read:status=DONE"));
    assert!(session.image_write.is_some());
    assert!(session.image_status("type=write:status=DONE"));
    assert!(session.image_write.is_none());
    assert!(!session.image_status("type=write:status=DONE"));
  }

  /// Shape tests share the global shape state.
  static SHAPES: std::sync::Mutex<()> = std::sync::Mutex::new(());

  /// The paper at page 2, 120x40.
  fn paper() -> Result<Session> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&root.join("paper.pdf"), true, screen)?;
    session.app().command("2")?;
    Ok(session)
  }

  /// The first page cell `wanted` accepts.
  fn find(
    session: &mut Session,
    wanted: fn(Hit) -> bool,
  ) -> Result<(u16, u16)> {
    let page = Region::Page;
    for (row, column) in (0..39).flat_map(|row| (0..120).map(move |c| (row, c)))
    {
      if wanted(session.app().hovered(page, column, row, None)?) {
        return Ok((column, row));
      }
    }
    anyhow::bail!("no such cell")
  }

  /// Shapes a frame writes after `kind`.
  fn shape_after(
    session: &mut Session,
    kind: MouseEventKind,
    (column, row): (u16, u16),
  ) -> Result<Vec<String>> {
    let modifiers = KeyModifiers::NONE;
    session.mouse(MouseEvent { kind, column, row, modifiers })?;
    session.settle()?;
    let mut out = Vec::new();
    session.draw_frame(&mut out)?;
    Ok(shapes(&out))
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn frames_write_changed_shapes() -> Result<()> {
    let _shapes = SHAPES.lock();
    let mut session = paper()?;
    let link = find(&mut session, |hit| matches!(hit, Hit::Link(..)))?;
    let blank = find(&mut session, |hit| hit == Hit::Nothing)?;
    terminal::arm_shapes();
    let frame = |session: &mut Session,
                 cell: Option<(u16, u16)>|
     -> Result<Vec<String>> {
      match cell {
        Some(cell) => shape_after(session, MouseEventKind::Moved, cell),
        None => {
          let mut out = Vec::new();
          session.draw_frame(&mut out)?;
          Ok(shapes(&out))
        }
      }
    };
    // (move to, written)
    let steps = [
      (Some(link), vec!["pointer"]),
      (None, vec![]),
      (Some(blank), vec!["default"]),
      (Some(link), vec!["pointer"]),
    ];
    for (step, (cell, want)) in steps.into_iter().enumerate() {
      assert_eq!(frame(&mut session, cell)?, want, "step {step}");
    }
    // A focus-out, then hiding, hands over once.
    let mut out = Vec::new();
    input(&mut session, Act::Focus(false), &mut out)?;
    assert_eq!(shapes(&out), ["default"]);
    out.clear();
    session.hide(&mut out)?;
    assert_eq!(shapes(&out), Vec::<String>::new());
    session.sight = Sight::Focused;
    // (move to, written)
    let steps =
      [(None, vec![]), (Some(blank), vec!["default"]), (Some(blank), vec![])];
    for (step, (cell, want)) in steps.into_iter().enumerate() {
      assert_eq!(frame(&mut session, cell)?, want, "shown again, step {step}");
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn scrollbars_wake_and_hide() -> Result<()> {
    let mut session = paper()?;
    session.draw_frame(&mut Vec::new())?;
    session.app().reader.scroll += 64.0;
    session.draw_frame(&mut Vec::new())?;
    let now = Instant::now();
    let wait = idle(&mut session, now, now, false);
    assert!(wait <= crate::scrollbar::SHOWN, "{wait:?}");
    assert!(!session.notify(), "shown until the deadline");
    let past = now.checked_sub(Duration::from_secs(1));
    session.app().bar.until = past;
    assert!(session.notify(), "the deadline draws a frame");
    assert_eq!(session.app().bar_until(), None);
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn held_gestures_write_shapes() -> Result<()> {
    use MouseEventKind::{Down, Drag, Moved, Up};
    use crossterm::event::MouseButton::{Left, Middle};
    let _shapes = SHAPES.lock();
    let mut session = paper()?;
    let blank = find(&mut session, |hit| hit == Hit::Nothing)?;
    let near = (blank.0 + 3, blank.1);
    terminal::arm_shapes();
    // (command first, event, cell) -> written
    let steps = [
      ("", Moved, blank, vec![]),
      ("", Down(Middle), blank, vec![]),
      ("", Drag(Middle), near, vec!["grabbing"]),
      ("", Up(Middle), near, vec!["default"]),
      ("vsplit", Moved, (60, 5), vec!["col-resize"]),
      ("", Down(Left), (60, 5), vec![]),
      ("", Drag(Left), (40, 5), vec![]),
      ("", Up(Left), (40, 5), vec![]),
      ("", Moved, (10, 39), vec!["default"]),
      ("split", Moved, (5, 19), vec!["row-resize"]),
    ];
    for (command, kind, cell, want) in steps {
      if !command.is_empty() {
        session.app().command(command)?;
        session.settle()?;
      }
      let got = shape_after(&mut session, kind, cell)?;
      assert_eq!(got, want, "{kind:?} at {cell:?}");
    }
    session.app().open_panel(PanelKind::Outline)?;
    let edge = session.app().left - 1;
    // (cell) -> written
    let steps = [
      ((edge - 1, 3), vec!["col-resize"]),
      ((edge, 3), vec![]),
      ((edge + 2, 3), vec!["default"]),
    ];
    for (cell, want) in steps {
      assert_eq!(shape_after(&mut session, Moved, cell)?, want, "{cell:?}");
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn image_status_preserves_bar() -> Result<()> {
    use crate::region::{Region, Transfer};
    use std::collections::VecDeque;
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&root.join("paper.pdf"), true, screen)?;
    session.app().move_figure(true, 1)?;
    session.app().open_region()?;
    let old = session.app().region.as_ref().unwrap().snapshot().clone();
    let pane = session.app().id;
    session.image_write = Some(ImageWrite {
      pane,
      capture: old.id,
      state: ImageState::Waiting(Instant::now() + Duration::from_millis(500)),
    });
    let now = Instant::now();
    assert!(idle(&mut session, now, now, false) <= Duration::from_millis(500));
    session.app().select_region(old.page, old.bounds)?;
    let new = session.app().region.as_ref().unwrap().snapshot().clone();
    assert_ne!(old.id, new.id);
    session.app().region_status(new.id, "New operation".into());
    let mut events = VecDeque::new();
    crate::decode::Parser::default()
      .feed(b"\x1b]5522;type=write:status=DONE\x1b\\", &mut events);
    let event = act(events.pop_front().unwrap()).unwrap();
    assert!(!event.typed());
    input(&mut session, event, &mut Vec::new())?;
    assert!(session.image_write.is_none());
    let Some(Region::Bar { status, .. }) = &session.app().region else {
      panic!("new bar")
    };
    assert_eq!(status, "New operation");
    assert_eq!(session.app().reader.message, "New operation");
    session.image_write =
      Some(ImageWrite { pane, capture: old.id, state: ImageState::TimedOut });
    session.app().region_send = Some(Transfer { id: new.id, png: vec![1] });
    let mut output = Vec::new();
    assert!(session.image_updates(&mut output));
    assert_eq!(output, [] as [u8; 0]);
    assert_eq!(session.image_write.as_ref().unwrap().capture, old.id);
    assert_eq!(
      session.app().reader.message,
      "Image clipboard awaiting previous status"
    );
    assert!(session.image_status("type=write:status=EPERM"));
    assert!(session.image_write.is_none());
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn landing_deadlines_expire_across_panes() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&root.join("paper.pdf"), true, screen)?;
    session.app().request = Some(Request::Window {
      command: Window::Split(Axis::Columns),
      count: 1,
      explicit: false,
      file: None,
    });
    session.settle()?;
    let now = Instant::now();
    for app in &mut session.tab().panes {
      app.flash_landing();
      app.reader.message = "Occupied".into();
      let remaining = app.landing.as_ref().unwrap().until.duration_since(now);
      assert!(remaining >= crate::marks::Landing::DURATION);
      assert!(remaining < Duration::from_millis(650));
    }
    let wait = idle(&mut session, now, now, false);
    assert!(wait <= crate::marks::Landing::DURATION);
    assert!(!session.notify());
    for app in &mut session.tab().panes {
      app.landing.as_mut().unwrap().until = now;
    }
    assert_eq!(idle(&mut session, now, now, false), Duration::ZERO);
    assert!(session.notify());
    assert!(session.tab().panes.iter().all(|app| app.landing.is_none()));
    assert!(!session.notify());
    Ok(())
  }

  #[test]
  fn unmeasured_resize_shows_on_the_last_row() -> Result<()> {
    let error = terminal::NO_CELL.to_owned();
    let unmeasured = Unmeasured { rows: 10, columns: 12, error };
    let mut out = Vec::new();
    unmeasured.draw(&mut out)?;
    let want = format!(
      "\x1b7{}\x1b[10;1HTerminal rep{}\x1b8",
      theme::ERROR,
      theme::RESET
    );
    assert_eq!(String::from_utf8(out)?, want);
    Ok(())
  }

  #[test]
  fn refit_keeps_the_layout_without_metrics() {
    let screen = Screen {
      columns: 80,
      rows: 24,
      origin: (0, 0),
      cell_width: 8,
      cell_height: 16,
    };
    let lost =
      Unmeasured { rows: 20, columns: 70, error: terminal::NO_CELL.into() };
    let mut session = Session::new(None);
    session.home = Some(Home::new(&[], screen));
    let failed = || Err(anyhow::anyhow!(terminal::NO_CELL));
    let grid = || Ok((70, 20));
    // (measured, want unmeasured, want start screen rows)
    let cases = [
      (failed(), Some(lost.clone()), 24),
      (Ok(None), Some(lost), 24),
      (Ok(Some(Screen { rows: 30, ..screen })), None, 30),
    ];
    for (index, (measured, unmeasured, rows)) in cases.into_iter().enumerate() {
      assert!(session.refit(measured, grid).is_ok(), "case {index}");
      assert_eq!(session.unmeasured, unmeasured, "case {index}");
      let home = session.home.as_ref().map(|home| home.screen.rows);
      assert_eq!(home, Some(rows), "case {index}");
    }
  }

  /// tmux tells only its active pane about focus; the rest is asked.
  #[test]
  fn sight_follows_tmux() {
    use Cause::{FocusOut, Start, Tick};
    use Sight::{Focused, Hidden, Unfocused};
    let active = View::Active { focused: true };
    let away = View::Active { focused: false };
    // (sight, cause, view, want)
    let cases = [
      (Focused, Start, active, Focused),
      (Focused, Start, View::Inactive, Unfocused),
      (Focused, Start, View::Hidden, Hidden),
      (Focused, FocusOut, View::Inactive, Unfocused),
      (Focused, FocusOut, View::Hidden, Hidden),
      // A popup or menu over the active pane.
      (Focused, FocusOut, active, Hidden),
      // The client lost the OS focus.
      (Focused, FocusOut, away, Unfocused),
      (Focused, Start, away, Unfocused),
      (Unfocused, Tick, View::Hidden, Hidden),
      (Unfocused, Tick, View::Inactive, Unfocused),
      (Hidden, Tick, View::Inactive, Unfocused),
      (Hidden, Tick, View::Hidden, Hidden),
      // The popup is still open; focus-in ends it.
      (Hidden, Tick, active, Hidden),
      (Unfocused, Tick, away, Unfocused),
    ];
    for (now, cause, view, want) in cases {
      assert_eq!(sight(now, cause, view), want, "{now:?} {cause:?} {view:?}");
    }
  }

  /// Entering `Hidden` hides; leaving it asks for a frame.
  #[test]
  fn turns_report_reveals() -> Result<()> {
    use Sight::{Focused, Hidden, Unfocused};
    let mut session = Session::new(None);
    // (to, frame due, sight after)
    let steps = [
      (Unfocused, false, Unfocused),
      (Hidden, false, Hidden),
      (Hidden, false, Hidden),
      (Unfocused, true, Unfocused),
      (Hidden, false, Hidden),
      (Focused, true, Focused),
      (Focused, false, Focused),
    ];
    for (to, due, after) in steps {
      let mut out = Vec::new();
      assert_eq!(session.turn(to, &mut out)?, due, "{to:?}");
      assert_eq!(session.sight, after, "{to:?}");
    }
    Ok(())
  }

  /// Keys and the pointer reveal a pane whose focus-in got lost.
  #[test]
  fn input_proves_the_pane_shown() -> Result<()> {
    use Sight::{Focused, Hidden, Unfocused};
    let key = Act::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    let mouse = Act::Mouse(Mouse {
      kind: MouseKind::Press(Button::LEFT),
      at: Spot::Cell { column: 1, row: 1 },
      modifiers: KeyModifiers::NONE,
    });
    // (sight, act, sight after)
    let cases = [
      (Hidden, &key, Focused),
      (Unfocused, &key, Focused),
      (Hidden, &mouse, Unfocused),
      (Unfocused, &mouse, Unfocused),
      (Hidden, &Act::Missing(1), Hidden),
      (Hidden, &Act::Focus(false), Hidden),
    ];
    for (now, act, want) in cases {
      let mut session = Session::new(None);
      session.sight = now;
      session.shown_by(act, &mut Vec::new())?;
      assert_eq!(session.sight, want, "{now:?} {act:?}");
    }
    Ok(())
  }

  /// Pointer input draws only when it reveals or changes something.
  #[test]
  fn pointer_input_reports_frames() -> Result<()> {
    let at = Spot::Cell { column: 1, row: 1 };
    let motion = || {
      Act::Mouse(Mouse {
        kind: MouseKind::Moved,
        at,
        modifiers: KeyModifiers::NONE,
      })
    };
    // (sight, act) -> frame
    let cases = [
      (Sight::Hidden, motion(), true),
      (Sight::Unfocused, motion(), false),
      (Sight::Focused, Act::Leave, false),
    ];
    for (sight, act, want) in cases {
      let mut session = Session::new(None);
      session.sight = sight;
      let at = format!("{sight:?} {act:?}");
      assert_eq!(input(&mut session, act, &mut Vec::new())?, want, "{at}");
    }
    Ok(())
  }

  /// Pointer shapes a frame writes.
  fn shapes(out: &[u8]) -> Vec<String> {
    let out = String::from_utf8_lossy(out);
    out
      .split("\x1b]22;")
      .skip(1)
      .map(|rest| rest.split('\x1b').next().unwrap_or("").to_owned())
      .collect()
  }

  /// One text line; `annots` join the page.
  fn line_pdf(directory: &Path, annots: &str) -> Result<PathBuf> {
    let pdf = directory.join("a.pdf");
    crate::fixtures::pdf(&pdf, &["alpha beta gamma"], annots)?;
    Ok(pdf)
  }

  #[test]
  fn sweeps_show_a_crosshair() -> Result<()> {
    use crate::pointer::{Hit, Region};
    use crossterm::event::MouseButton::Left;
    use crossterm::event::MouseEventKind::{Down, Drag, Moved, Up};
    let directory = tempfile::tempdir()?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), "")?, true, screen)?;
    let app = session.app();
    let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
    let text = cells.find(|&(column, row)| {
      app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
    });
    let (column, row) = text.expect("a text cell");
    terminal::arm_shapes();
    let (alt, plain) = (KeyModifiers::ALT, KeyModifiers::NONE);
    // (event, modifiers, column) -> shapes written
    let steps = [
      (Moved, plain, column, vec!["text"]),
      (Down(Left), alt, column, vec!["crosshair"]),
      (Drag(Left), alt, column + 6, vec![]),
      // Lost release: motion drops the sweep.
      (Moved, plain, column, vec!["text"]),
      (Down(Left), alt, column, vec!["crosshair"]),
      (Drag(Left), alt, column + 6, vec![]),
      (Up(Left), alt, column + 6, vec!["text"]),
    ];
    for (step, (kind, modifiers, column, want)) in steps.into_iter().enumerate()
    {
      session.mouse(MouseEvent { kind, column, row, modifiers })?;
      let mut out = Vec::new();
      session.draw_frame(&mut out)?;
      assert_eq!(shapes(&out), want, "step {step}: {kind:?}");
    }
    Ok(())
  }

  /// `:mouse off` drops hover and later reports; `:mouse` toggles.
  #[test]
  fn mouse_command_toggles_capture() -> Result<()> {
    use crate::decode::{Mouse, MouseKind, Spot};
    use crate::pointer::{Hit, Region};
    let directory = tempfile::tempdir()?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), "")?, true, screen)?;
    session.screen = Some(screen);
    let app = session.app();
    let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
    let text = cells.find(|&(column, row)| {
      app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
    });
    let (column, row) = text.expect("a text cell");
    let at = Spot::Cell { column, row };
    let moved =
      Mouse { kind: MouseKind::Moved, at, modifiers: KeyModifiers::NONE };
    let hovered = |session: &Session| session.pointer.target().map(|h| h.hit);
    assert!(session.mouse_report(moved)?);
    assert_eq!(hovered(&session), Some(Hit::Text));
    // (command, capture after, hover after a report)
    let steps = [
      ("mouse off", false, None),
      ("mouse", true, Some(Hit::Text)),
      ("mouse", false, None),
      ("mouse on", true, Some(Hit::Text)),
    ];
    for (command, on, hover) in steps {
      // A held pan gets no release once capture ends.
      let grab = crate::gestures::Grab { last: (0.0, 0.0), moved: true };
      session.app().grab = Some(grab);
      session.app().command(command)?;
      session.settle()?;
      assert_eq!(terminal::mouse(), on, "{command}");
      assert_eq!(session.app().grabbing(), on, "{command}: pan");
      assert_eq!(hovered(&session), None, "{command}: dropped");
      let info = session.app().info();
      assert_eq!(info.contains("Mouse: on"), on, "{command}: info");
      session.mouse_report(moved)?;
      assert_eq!(hovered(&session), hover, "{command}: report");
      session.pointer.leave();
    }
    assert!(session.app().command("mouse maybe").is_err());
    Ok(())
  }

  /// Menu backdrops cover only page cells.
  #[test]
  fn menu_backdrops_skip_the_panel() -> Result<()> {
    use crate::keys::PanelKind;
    use crossterm::event::MouseButton::Right;
    use crossterm::event::MouseEventKind::{Down, Up};
    let directory = tempfile::tempdir()?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), "")?, true, screen)?;
    session.app().open_panel(PanelKind::Help)?;
    let left = session.app().left;
    // (pressed column) -> backdrop's page-area x range
    let cases = [(1, None), (left - 3, Some(0.0))];
    for (column, want) in cases {
      for kind in [Down(Right), Up(Right)] {
        let modifiers = KeyModifiers::NONE;
        session.mouse(MouseEvent { kind, column, row: 3, modifiers })?;
      }
      let app = session.app();
      let area = app.context_menu.as_ref().expect("a menu").area();
      let rect = app.context_rect();
      assert_eq!(rect.map(|rect| rect.x0), want, "{column}");
      if let Some(rect) = rect {
        let end = f32::from(area.column + area.columns - left) * 8.0;
        assert_eq!(rect.x1, end, "{column}");
      } else {
        assert!(area.column + area.columns <= left, "{column}");
      }
      app.context_menu = None;
    }
    Ok(())
  }

  /// Page-area pixels each `z=1` placement in `out` covers.
  fn highlights(out: &[u8], left: u16) -> Vec<Bounds> {
    let text = String::from_utf8_lossy(out);
    let parts = text.split("\x1b7\x1b[").skip(1);
    let placed = parts.filter_map(|part| {
      let (at, rest) = part.split_once('H')?;
      let (row, column) = at.split_once(';')?;
      let control = rest.strip_prefix("\x1b_G")?.split(';').next()?;
      let value = |key: &str| {
        let mut pairs = control.split(',').filter_map(|p| p.split_once('='));
        pairs.find(|(name, _)| *name == key).map(|(_, value)| value)
      };
      let number = |key| value(key).and_then(|v| v.parse::<f32>().ok());
      if value("z") != Some("1") {
        return None;
      }
      let column = column.parse::<f32>().ok()? - 1.0 - f32::from(left);
      let row = row.parse::<f32>().ok()? - 1.0;
      let x0 = column * 8.0 + number("X")?;
      let y0 = row * 16.0 + number("Y")?;
      let (x1, y1) = (x0 + number("w")?, y0 + number("h")?);
      Some(Bounds { x0, y0, x1, y1 })
    });
    placed.collect()
  }

  /// The focused panel's page veil leaves menus over the page clear.
  #[test]
  fn veils_skip_menus() -> Result<()> {
    use crate::keys::PanelKind;
    use crossterm::event::MouseButton::Right;
    use crossterm::event::MouseEventKind::{Down, Up};
    let directory = tempfile::tempdir()?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), "")?, true, screen)?;
    session.app().open_panel(PanelKind::Help)?;
    let left = session.app().left;
    // A stretched pixel: `c=` columns wide.
    let veiled = |out: &[u8]| {
      let text = String::from_utf8_lossy(out);
      text.split("\x1b_G").any(|g| g.contains("z=1,c="))
    };
    // (pressed column) -> veiled
    let cases = [(None, true), (Some(1), true), (Some(left - 3), false)];
    for (column, want) in cases {
      session.app().context_menu = None;
      for kind in column.map_or_default(|_| vec![Down(Right), Up(Right)]) {
        let column = column.unwrap_or_default();
        let modifiers = KeyModifiers::NONE;
        session.mouse(MouseEvent { kind, column, row: 3, modifiers })?;
      }
      let app = session.app();
      assert!(app.panel_focused(), "{column:?}");
      let mut out = Vec::new();
      app.draw(&mut out)?;
      assert_eq!(veiled(&out), want, "{column:?}");
    }
    Ok(())
  }

  /// Thumbnail rings leave the menu over them clear.
  #[test]
  fn overview_rings_skip_the_menu() -> Result<()> {
    use crossterm::event::MouseButton::Right;
    use crossterm::event::MouseEventKind::{Down, Up};
    let directory = tempfile::tempdir()?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), "")?, true, screen)?;
    let app = session.app();
    app.open_overview()?;
    app.draw(&mut Vec::new())?;
    let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
    let thumbnail =
      cells.find(|&(column, row)| app.thumbnail_at(column, row).is_some());
    let (column, row) = thumbnail.expect("a thumbnail");
    for kind in [Down(Right), Up(Right)] {
      let modifiers = KeyModifiers::NONE;
      session.mouse(MouseEvent { kind, column, row, modifiers })?;
    }
    let app = session.app();
    let menu = app.context_rect().expect("a menu over the page");
    let mut out = Vec::new();
    app.draw(&mut out)?;
    let rings = highlights(&out, app.left);
    assert!(!rings.is_empty(), "the selected ring shows");
    let over = |b: &Bounds| {
      b.x0 < menu.x1 && menu.x0 < b.x1 && b.y0 < menu.y1 && menu.y0 < b.y1
    };
    assert!(!rings.iter().any(over), "{rings:?} over {menu:?}");
    Ok(())
  }

  /// Focus loss ends held drags.
  #[test]
  fn edge_drags_scroll_until_focus_loss() -> Result<()> {
    use crate::gestures::AUTOSCROLL;
    use crate::pointer::{Hit, Region};
    use crossterm::event::MouseButton::Left;
    use crossterm::event::MouseEventKind::{Down, Drag};
    let directory = tempfile::tempdir()?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), "")?, true, screen)?;
    let app = session.app();
    let mut cells = (0..23).flat_map(|row| (0..80).map(move |c| (c, row)));
    let text = cells.find(|&(column, row)| {
      app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
    });
    let (column, row) = text.expect("a text cell");
    let now = Instant::now();
    assert!(idle(&mut session, now, now, false) > AUTOSCROLL);
    let (alt, plain) = (KeyModifiers::ALT, KeyModifiers::NONE);
    let corner = |app: &App| match app.gesture {
      crate::gestures::Gesture::Sweep(sweep) => sweep.to,
      _ => None,
    };
    for modifiers in [plain, alt] {
      let event = |kind, row| MouseEvent { kind, column, row, modifiers };
      session.app().reader.scroll = 0.0;
      session.mouse(event(Down(Left), row))?;
      // The status row is past it.
      session.mouse(event(Drag(Left), 23))?;
      let due = session.autoscroll_due().expect("a held edge drag");
      assert!(idle(&mut session, now, now, false) <= AUTOSCROLL);
      let before = corner(session.app());
      assert!(session.autoscroll(due), "{modifiers:?}");
      // Sweeps follow the scroll too.
      let moved = corner(session.app()) != before;
      assert_eq!(moved, modifiers == alt, "{modifiers:?}");
      input(&mut session, Act::Focus(false), &mut Vec::new())?;
      let later = Instant::now() + AUTOSCROLL;
      assert!(!session.autoscroll(later), "{modifiers:?}");
      assert_eq!(session.autoscroll_due(), None);
      // Drags after it hold nothing.
      session.mouse(event(Drag(Left), 23))?;
      assert_eq!(session.autoscroll_due(), None, "{modifiers:?}");
      assert!(!session.app().sweeping(), "{modifiers:?}");
      assert_eq!(session.app().sweep_hint(), None);
    }
    Ok(())
  }

  #[test]
  fn start_screen_drops_open_or_type() -> Result<()> {
    use base64::Engine;
    let directory = tempfile::tempdir()?;
    let pdf = line_pdf(directory.path(), "")?;
    let notes = directory.path().join("notes.txt");
    std::fs::write(&notes, "text")?;
    let screen = Screen {
      columns: 80,
      rows: 24,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let list = |path: &Path| {
      let uri = format!("file://{}\r\n", path.display());
      base64::engine::general_purpose::STANDARD.encode(uri)
    };
    let drop = |session: &mut Session, home: &mut Home, path: &Path| {
      let mut out = Vec::new();
      let mut landed = Landing::Wait;
      for body in [
        "t=M:x=1:y=1:o=1;text/uri-list".to_owned(),
        format!("t=r:x=1;{}", list(path)),
        "t=r:x=1".to_owned(),
      ] {
        landed = session.home_drop(home, &body, &mut out)?;
      }
      Ok::<_, anyhow::Error>((landed, String::from_utf8_lossy(&out).into()))
    };
    let ended =
      |out: &String, how: &str| out.ends_with(&format!("t=r:o={how}\x1b\\"));
    // A prompt takes the path as text.
    let mut session = Session::new(None);
    let mut home = Home::new(&[], screen);
    let colon = KeyEvent::new(KeyCode::Char(':'), KeyModifiers::NONE);
    home.event(Event::Key(colon), &mut session.history)?;
    let (landed, out) = drop(&mut session, &mut home, &pdf)?;
    assert!(matches!(landed, Landing::Shown) && ended(&out, "1"));
    assert!(session.tabs.is_empty());
    // Without a prompt the PDF opens; a list with none is cancelled.
    let mut home = Home::new(&[], screen);
    let (landed, out) = drop(&mut session, &mut home, &notes)?;
    assert!(matches!(landed, Landing::Shown) && ended(&out, "0"));
    assert_eq!(home.error.as_deref(), Some("No PDF in the drop"));
    let (landed, out) = drop(&mut session, &mut home, &pdf)?;
    assert!(matches!(landed, Landing::Opened) && ended(&out, "1"));
    assert_eq!(session.tabs.len(), 1);
    // The terminal's error shows and ends the drop.
    let mut home = Home::new(&[], screen);
    let mut out = Vec::new();
    session.home_drop(&mut home, "t=M:x=1:y=1:o=1;text/uri-list", &mut out)?;
    let landed = session.home_drop(&mut home, "t=R:x=1;EIO", &mut out)?;
    assert!(matches!(landed, Landing::Shown));
    assert!(ended(&String::from_utf8_lossy(&out).into(), "0"));
    assert_eq!(home.error.as_deref(), Some("Drop failed: EIO"));
    Ok(())
  }

  #[test]
  fn only_actionable_input_counts() {
    let none = KeyModifiers::NONE;
    let press = KeyEvent::new(KeyCode::Char('j'), none);
    let release =
      KeyEvent::new_with_kind(KeyCode::Char('j'), none, KeyEventKind::Release);
    let mouse = |kind, at| Event::Mouse(Mouse { kind, at, modifiers: none });
    let cell = Spot::Cell { column: 1, row: 2 };
    let size = Size { rows: 24, columns: 80, height: 384, width: 640 };
    let graphics = |id, message: &str| {
      Event::Reply(Reply::Graphics {
        id,
        placement: Some(1),
        message: message.into(),
      })
    };
    let click =
      |at| Mouse { kind: MouseKind::Press(Button::LEFT), at, modifiers: none };
    let pixel = Spot::Pixel { x: 1, y: 2 };
    let cases = [
      (Event::Key(press), Some(Act::Key(press))),
      (Event::Key(release), None),
      (Event::Paste("a".into()), Some(Act::Paste("a".into()))),
      (Event::Resize(None), Some(Act::Resize(None))),
      (Event::Resize(Some(size)), Some(Act::Resize(Some(size)))),
      (
        mouse(MouseKind::Press(Button::LEFT), cell),
        Some(Act::Mouse(click(cell))),
      ),
      (
        mouse(MouseKind::Press(Button::BACK), cell),
        Some(Act::Mouse(Mouse {
          kind: MouseKind::Press(Button::BACK),
          at: cell,
          modifiers: none,
        })),
      ),
      (mouse(MouseKind::Release(Button::BACK), cell), None),
      (
        mouse(MouseKind::Press(Button::LEFT), pixel),
        Some(Act::Mouse(click(pixel))),
      ),
      (Event::Leave, Some(Act::Leave)),
      (Event::Focus(true), Some(Act::Focus(true))),
      (Event::Reply(Reply::Keyboard(5)), None),
      (graphics(Some(9), "ENOENT:no such image"), Some(Act::Missing(9))),
      (graphics(Some(9), "OK"), None),
      (graphics(None, "ENOENT:no such image"), None),
      (
        Event::Reply(Reply::Osc { code: 72, body: "t=m".into() }),
        Some(Act::Drop("t=m".into())),
      ),
    ];
    for (event, want) in cases {
      let at = format!("{event:?}");
      assert_eq!(act(event), want, "{at}");
    }
    // None of these moves `touched`.
    let moved = act(mouse(MouseKind::Moved, cell)).expect("motion");
    assert!(!moved.typed() && !Act::Leave.typed());
    assert!(!Act::Drop("t=m".into()).typed());
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn local_keys_clear_capture() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 121,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&root.join("book.pdf"), true, screen)?;
    let mut events = std::collections::VecDeque::new();
    let mut parser = crate::decode::Parser::default();
    for bytes in [
      &b":30\r\x1b[<0;30;10M\x1b[<32;35;12M"[..],
      &b"\t"[..],
      &b"\x1b[<32;100;32M\x1b[<0;100;32m"[..],
    ] {
      if bytes == b"\t" {
        session.app().open_panel(PanelKind::Help)?;
      }
      let cursor = session.app().reader.cursor;
      parser.feed(bytes, &mut events);
      for event in events.drain(..) {
        let act = act(event).expect("actionable bytes");
        input(&mut session, act, &mut Vec::new())?;
        session.settle()?;
      }
      if bytes.starts_with(b"\x1b[<32;100") {
        assert_eq!(session.app().reader.cursor, cursor);
      }
    }
    assert!(session.app().reader.selection.is_some());
    assert!(!session.app().panel_focused());
    Ok(())
  }

  /// Keys close previews in every pane (U15).
  #[test]
  fn keys_close_other_previews() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let link = "/Annots[<</Type/Annot/Subtype/Link/Rect[15 90 200 125]\
                /Dest[3 0 R/XYZ 0 50 0]>>]";
    let screen = Screen {
      columns: 121,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new(None);
    session.open(&line_pdf(directory.path(), link)?, true, screen)?;
    session.app().command("vsplit")?;
    session.settle()?;
    let tab = session.tab();
    let index = 1 - tab.focus;
    let app = &mut tab.panes[index];
    let bounds = app.doc.links(PageId(0))?[0].first();
    let rect = app.layout.project(PageId(0), bounds, app.reader.scroll);
    let (x, y) = rect.centre();
    let column = app.at.column + app.left + (x / 8.0) as u16;
    let row = app.at.row + (y / 16.0) as u16;
    let (kind, modifiers) = (MouseEventKind::Moved, KeyModifiers::NONE);
    session.mouse(MouseEvent { kind, column, row, modifiers })?;
    assert!(session.hover_preview(Instant::now() + crate::links::REST));
    assert!(session.tab().panes[index].peek.is_some());
    let mut events = std::collections::VecDeque::new();
    crate::decode::Parser::default().feed(b"j", &mut events);
    for event in events {
      input(&mut session, act(event).expect("a key"), &mut Vec::new())?;
    }
    assert!(session.tab().panes.iter().all(|app| app.peek.is_none()));
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn deferred_panels_clear_capture() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 121,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    for command in [b":recent\r".as_slice(), b":marks\r".as_slice()] {
      let mut session = Session::new(None);
      session.open(&root.join("book.pdf"), true, screen)?;
      let mut events = std::collections::VecDeque::new();
      let mut parser = crate::decode::Parser::default();
      for bytes in [
        b":30\r\x1b[<0;30;10M\x1b[<32;35;12M".as_slice(),
        command,
        b"\x1b[<32;40;15M\x1b[<0;40;15m".as_slice(),
      ] {
        let cursor = session.app().reader.cursor;
        parser.feed(bytes, &mut events);
        for event in events.drain(..) {
          input(
            &mut session,
            act(event).expect("input bytes"),
            &mut Vec::new(),
          )?;
          session.settle()?;
        }
        if bytes.starts_with(b"\x1b[<32;40") {
          assert_eq!(session.app().reader.cursor, cursor);
        }
      }
      assert!(session.app().panel_focused());
      assert!(session.app().reader.selection.is_some());
    }
    Ok(())
  }
}
