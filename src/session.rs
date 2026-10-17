use crate::Result;
use crate::app::{App, Request, TabMove, Tabline, Window};
use crate::decode::{Event, MouseKind, Reply, Size};
use crate::dnd::{Dropped, Step, Target};
use crate::document::PasswordRequired;
use crate::graphics::Graphics;
use crate::home::{Home, Outcome};
use crate::keys::PanelKind;
use crate::layout::Screen;
use crate::pace::Pacer;
use crate::pointer::{self, Hit, Hover, Pane, Pixel, Pointer, Route};
use crate::prompt::History;
use crate::split::{Axis, Direction, Node, Rect, neighbor};
use crate::state::{Mark, State};
use crate::terminal::{self, At, View};
use crate::text::PageId;
use crate::theme;
use crate::trace::Drawable;
use crate::watch::Watcher;
use crossterm::event::{KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
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
    }
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
    let tab = &mut self.tabs[active];
    let mut rects = Vec::new();
    tab.tree.layout(full(screen), &mut rects);
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
    self.leave()?;
    let mut graphics = self.take_graphics();
    let tab = self.tabs.remove(self.active);
    for app in &tab.panes {
      self.retire(app, &mut graphics);
    }
    if self.tabs.is_empty() {
      return self.save();
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
    let area = self.screen.map_or_default(full);
    let mut graphics = self.take_graphics();
    let app = self.tab().detach(area);
    self.retire(&app, &mut graphics);
    self.give(graphics);
    self.arrange()
  }

  /// Splits the focused pane; the new pane shows `file` or the same view.
  fn split(&mut self, axis: Axis, file: Option<&Path>) -> Result<()> {
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
    tab.tree.split(target, axis, app.id);
    tab.panes.push(app);
    let index = tab.panes.len() - 1;
    self.focus(self.active, index)
  }

  fn window(
    &mut self,
    command: Window,
    (count, explicit): (usize, bool),
    file: Option<&Path>,
  ) -> Result<()> {
    let area = self.screen.map_or_default(full);
    let tab = self.tab();
    let id = tab.panes[tab.focus].id;
    let order = tab.order();
    let at = order.iter().position(|pane| *pane == id).unwrap_or(0);
    let total = order.len();
    let target = match command {
      Window::Split(axis) => return self.split(axis, file),
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
        self.split(Axis::Rows, None)?;
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
    let area = self.screen.map_or_default(full);
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
        let mark = self.state.marks.get(&name).cloned();
        let mark =
          mark.ok_or_else(|| anyhow::anyhow!("Mark not set: {name}"))?;
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
        let mark = self.state.marks.get(&name).cloned();
        let mark =
          mark.ok_or_else(|| anyhow::anyhow!("Mark not set: {name}"))?;
        self.open(&mark.path, true, screen)?;
        self.app().go_to(mark.cursor, exact)
      }
      Some(Request::Pick) => match crate::fzf::pick(&std::env::current_dir()?)?
      {
        Some(path) => self.open(&path, false, screen),
        None => Ok(()),
      },
      Some(Request::TabClose) => self.close(),
      Some(Request::QuitAll) => {
        self.quit = true;
        Ok(())
      }
      None => Ok(()),
    }
  }

  fn panel_open(
    &mut self,
    path: &Path,
    position: Option<crate::text::Position>,
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
        self.split(Axis::Columns, Some(&path))?;
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
      app.go_to(position, true)?;
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
      self.focus(source.0, source.1)
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
    if self.app().dismiss_click()? {
      self.pointer.release();
      return Ok(true);
    }
    let Some((index, _)) = self.under(mouse) else { return Ok(false) };
    self.app().disarm();
    let app = self.app();
    app.error = None;
    app.reader.message.clear();
    self.pointer.release();
    self.focus(self.active, index)?;
    let history = crate::keys::Input {
      action: crate::keys::Action::History(forward),
      count: 1,
      explicit: false,
      register: '"',
    };
    self.app().action(history)?;
    Ok(true)
  }

  /// A report in cells; whether it needs a frame.
  pub fn mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
    self.pointed(mouse, None, Instant::now())
  }

  /// Presses focus; the wheel scrolls in place; bare motion hovers.
  fn pointed(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<bool> {
    if mouse.kind == MouseEventKind::Moved {
      let hover = self.hit(mouse, pixel);
      let changed = self.pointer.hover(mouse, pixel, hover);
      if changed {
        self.show_hover();
      }
      return Ok(changed);
    }
    let wheel = pointer::Wheel::of(mouse.kind).is_some();
    if wheel && self.app().prompt.is_some() {
      return Ok(false);
    }
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      if self.on_completion(mouse) {
        return Ok(false);
      }
      if self.app().dismiss_click()? {
        self.pointer.release();
        return Ok(true);
      }
    }
    let tab = &self.tabs[self.active];
    // Snapshot before disarm hides clue rows.
    let panes: Vec<_> = tab.panes.iter().map(Pane::of).collect();
    let focused = tab.panes[tab.focus].id;
    let route = self.pointer.route(&panes, focused, mouse, pixel, now);
    let gesture = wheel && self.pointer.turn(mouse, pixel, now);
    let interrupts = pointer::interrupts(mouse.kind);
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
      let pointer = std::mem::take(&mut self.pointer);
      let focused = self.focus(self.active, index);
      self.pointer = pointer;
      focused?;
    }
    let app = &mut self.tab().panes[index];
    // Per-pixel drags; edges scroll.
    let reading = |app: &App| {
      let view = (app.reader.scroll.to_bits(), app.text_top);
      (app.reader.cursor, app.reader.selection, view)
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
      app.error = None;
      app.reader.message.clear();
    }
    app.mouse(route)?;
    let moved = view.is_none_or(|view| view != app.wheel_view());
    let read = before.is_none_or(|before| before != reading(app));
    let changed = read && moved;
    Ok(armed || changed)
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
      Ok(Target::Pdf(path)) => {
        self.open(&path, true, home.screen).map(|()| true)
      }
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
    let opened = self.open(path, true, home.screen);
    if let Err(error) = &opened {
      home.error = Some(error.to_string());
    }
    opened.is_ok()
  }

  /// Whether a press hits completions.
  fn on_completion(&mut self, mouse: MouseEvent) -> bool {
    let pane = Pane::of(self.app());
    let local = pointer::route(&[pane], pane.id, None, mouse);
    self.app().prompt.is_some()
      && local.is_some_and(|route| route.region == pointer::Region::Overlay)
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
    changed
  }

  /// Retests the last spot; the view changed.
  fn rehover(&mut self) {
    if let Some((mouse, pixel)) = self.pointer.spot() {
      let hover = self.hit(mouse, pixel);
      self.pointer.refresh(hover);
    }
    self.show_hover();
  }

  /// Gives each pane its hovered link.
  fn show_hover(&mut self) {
    let hover = self.pointer.target();
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    for app in &mut tab.panes {
      let link = hover.and_then(|hover| match hover.hit {
        Hit::Link(page, index)
          if hover.pane == app.id && hover.region == pointer::Region::Page =>
        {
          Some((page, index))
        }
        _ => None,
      });
      app.point_link(link);
    }
  }

  /// What the pointer rests on at `mouse`.
  fn hit(&mut self, mouse: MouseEvent, pixel: Option<Pixel>) -> Option<Hover> {
    let (index, route) = self.under(mouse)?;
    let Route { pane, region, mouse, .. } = route;
    let app = &mut self.tab().panes[index];
    // Hover is advisory; failures find nothing.
    let hit = app.hovered(region, mouse.column, mouse.row, pixel);
    let hit = hit.unwrap_or_default();
    Some(Hover { pane, region, hit })
  }

  /// Fits the start screen or every pane to a new terminal size.
  pub fn resize(&mut self, screen: Screen) -> Result<()> {
    if let Some(home) = &mut self.home {
      home.screen = screen;
    }
    if self.tabs.is_empty() {
      return Ok(());
    }
    self.screen = Some(screen);
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
    self.pointer.leave();
    terminal::hand_over(out)?;
    // Out before the slow `tmux display`.
    out.flush()?;
    Ok(())
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
    }
    let reader = &mut self.app().reader;
    if !reader.message.is_empty() {
      return changed;
    }
    let Some(notice) = terminal::notice() else { return changed };
    reader.message = notice;
    true
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

  /// Draws the start screen and handles one input.
  fn home(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(mut home) = self.home.take() else { return Ok(()) };
    // Its thumbnail would cover what tmux shows instead.
    if self.sight != Sight::Hidden {
      // Start-screen motion is passive.
      self.pointer.leave();
      terminal::point(out, terminal::Shape::Default)?;
      home.draw(out)?;
    }
    if let Some(unmeasured) = &self.unmeasured {
      unmeasured.draw(out)?;
      out.flush()?;
    }
    let event = loop {
      // A wake may come from the hangup thread; the caller checks it.
      let Some(event) = crate::input::next(None)? else {
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
      let measured = terminal::resized(report, Some(home.screen));
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
    self.rehover();
    let hover = self.pointer.target();
    // Without a spot the pointer is elsewhere.
    if self.pointer.spot().is_some() {
      terminal::point(out, hover.map_or_default(|hover| hover.hit.shape()))?;
    }
    for app in &mut self.tab().panes {
      let here = hover.filter(|hover| hover.pane == app.id);
      app.hover = here.map_or_default(|hover| hover.hit);
    }
    write!(out, "{}\x1b[0m", terminal::frame_start())?;
    self.app().graphics.begin(out)?;
    let area = self.screen.map_or_default(full);
    let unmeasured = self.unmeasured.clone();
    let tab = self.tab();
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        tab.lend(index, |app| app.draw_pane(out))?;
      }
    }
    let mut separators = Vec::new();
    tab.tree.separators(area, &mut separators);
    for rect in separators {
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
    write!(out, "{}", terminal::frame_end())?;
    // Drawing can scroll thumbnails or rewrap rows.
    let drawn = self.pointer.target();
    self.rehover();
    let moved = self.pointer.target() != drawn;
    // One follow-up, so hover-drawn regions cannot loop.
    let following = self.rehover == Rehover::Following;
    self.rehover =
      if moved && !following { Rehover::Due } else { Rehover::Settled };
    Ok(())
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
    session.home = Some(Home::new(&session.state.recent, screen));
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
    app.reader.scroll = app.layout.pages[app.reader.cursor.page.0].top;
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
    Act::Key(key) => {
      let tab = &mut session.tabs[session.active];
      let app = &mut tab.panes[tab.focus];
      let before = app.panel.as_ref().filter(|p| p.focused).map(|p| p.kind);
      app.reader.message.clear();
      app.error = None;
      // Clues close even when the key fails.
      let result = app.key(key, &mut session.history);
      app.after_key();
      let after = app.panel.as_ref().filter(|p| p.focused).map(|p| p.kind);
      if before != after {
        session.pointer.release();
      }
      result
    }
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
  let mut out = io::BufWriter::new(io::stdout());
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
    let wait = if pending {
      pacer.wait(now)
    } else {
      idle(session, chores.checked, chores.watched, changed)
    };
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
  for app in &session.tab().panes {
    if let Some(landing) = &app.landing {
      wait = wait.min(landing.until.saturating_duration_since(Instant::now()));
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
  use crossterm::event::{KeyCode, KeyModifiers};

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

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn frames_write_changed_shapes() -> Result<()> {
    use crate::pointer::{Hit, Region};
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
    let mut find = |wanted: fn(Hit) -> bool| -> Result<(u16, u16)> {
      for (row, column) in
        (0..39).flat_map(|row| (0..120).map(move |c| (row, c)))
      {
        if wanted(session.app().hovered(Region::Page, column, row, None)?) {
          return Ok((column, row));
        }
      }
      anyhow::bail!("no such cell")
    };
    let link = find(|hit| matches!(hit, Hit::Link(..)))?;
    let blank = find(|hit| hit == Hit::Nothing)?;
    let moved = |(column, row)| MouseEvent {
      kind: MouseEventKind::Moved,
      column,
      row,
      modifiers: KeyModifiers::NONE,
    };
    terminal::arm_shapes();
    let osc = |out: &[u8]| {
      let out = String::from_utf8_lossy(out);
      out
        .split("\x1b]22;")
        .skip(1)
        .map(|rest| rest.split('\x1b').next().unwrap_or("").to_owned())
        .collect::<Vec<_>>()
    };
    let frame = |session: &mut Session,
                 cell: Option<(u16, u16)>|
     -> Result<Vec<String>> {
      if let Some(cell) = cell {
        session.mouse(moved(cell))?;
      }
      let mut out = Vec::new();
      session.draw_frame(&mut out)?;
      Ok(osc(&out))
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
    assert_eq!(osc(&out), ["default"]);
    out.clear();
    session.hide(&mut out)?;
    assert_eq!(osc(&out), Vec::<String>::new());
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

  #[test]
  fn start_screen_drops_open_or_type() -> Result<()> {
    use base64::Engine;
    let directory = tempfile::tempdir()?;
    let pdf = directory.path().join("a.pdf");
    // MuPDF repairs the missing cross-reference table.
    std::fs::write(
      &pdf,
      "%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n\
       2 0 obj <</Type/Pages/Kids[3 0 R]/Count 1>> endobj\n\
       3 0 obj <</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]>> endobj\n\
       trailer <</Root 1 0 R>>\n%%EOF\n",
    )?;
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
