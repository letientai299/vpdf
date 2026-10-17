use crate::Result;
use crate::app::{App, Request, TabMove, Tabline, Window};
use crate::document::PasswordRequired;
use crate::graphics::Graphics;
use crate::home::{Home, Outcome};
use crate::keys::PanelKind;
use crate::layout::Screen;
use crate::pace::Pacer;
use crate::pointer::{self, Pane, Pointer};
use crate::prompt::History;
use crate::split::{Axis, Node, Rect, neighbor};
use crate::state::{Mark, State};
use crate::terminal::{self, At, Terminal};
use crate::text::PageId;
use crate::theme;
use crate::trace::Drawable;
use crate::watch::Watcher;
use crossterm::event::{self, Event, KeyEventKind, MouseEvent, MouseEventKind};
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
  password: String,
  text: bool,
  /// Set by SIGHUP or SIGTERM, so the session ends and saves.
  hangup: Arc<AtomicBool>,
  latest: crate::hangup::Latest,
  /// Recorded positions not yet written.
  unsaved: bool,
  /// History revision last handed to the hangup thread.
  recorded: u64,
  /// Panes the last arrangement showed; ones hidden since are trimmed.
  shown: Vec<u32>,
  /// Frame interval while anything moves.
  frame: Duration,
  pointer: Pointer,
}

impl Session {
  /// `store` holds recent documents and history between runs.
  pub fn new(password: &str, text: bool, store: Option<PathBuf>) -> Self {
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
      password: password.to_owned(),
      text,
      hangup: Arc::new(AtomicBool::new(false)),
      latest: crate::hangup::Latest::default(),
      unsaved: false,
      recorded: 0,
      shown: Vec::new(),
      frame: crate::pace::FRAME,
      pointer: Pointer::default(),
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
    if let Some(tab) = self.tabs.get_mut(self.active)
      && let Some(app) = tab.panes.get_mut(tab.focus)
    {
      app.blur()?;
    }
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
      // Another tab or pane may have drawn over these rows.
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

  /// Opens `path` in a new tab, or in place of the focused pane.
  pub fn open(&mut self, path: &Path, tab: bool, screen: Screen) -> Result<()> {
    self.screen = Some(screen);
    let path = std::fs::canonicalize(path)?;
    let found = self.tabs.iter().enumerate().find_map(|(t, tab)| {
      tab.panes.iter().position(|app| app.path == path).map(|p| (t, p))
    });
    if let Some((tab, pane)) = found {
      return self.focus(tab, pane);
    }
    let password = self.password.clone();
    let app = self.load(&path, &password, screen)?;
    let mut graphics = self.take_graphics();
    if tab || self.tabs.is_empty() {
      let index = if self.tabs.is_empty() { 0 } else { self.active + 1 };
      self.tabs.insert(index, Tab::new(app));
      self.active = index;
      self.give(graphics);
      return self.arrange();
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
      match App::with_screen(path, &secret, self.text, screen) {
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
    let mut graphics = self.take_graphics();
    let tab = self.tab();
    let app = tab.panes.remove(tab.focus);
    tab.tree.remove(app.id);
    let next = tab.last.and_then(|id| tab.index(id));
    let next =
      next.or_else(|| tab.order().first().and_then(|id| tab.index(*id)));
    tab.focus = next.unwrap_or(0);
    tab.last = None;
    self.retire(&app, &mut graphics);
    self.give(graphics);
    self.arrange()
  }

  /// Splits the focused pane; the new pane shows `file` or the same view.
  fn split(&mut self, axis: Axis, file: Option<&Path>) -> Result<()> {
    let screen = self.app().screen;
    anyhow::ensure!(Node::fits(axis, full(screen)), "Not enough room");
    // The same document reuses the password that unlocked it.
    let (path, password) = match file {
      Some(file) => (std::fs::canonicalize(file)?, self.password.clone()),
      None => (self.app().path.clone(), self.app().password.clone()),
    };
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
        self.split(Axis::Rows, None)?;
        let rows = crate::panel::recent_rows(&self.state.recent);
        return self.app().show_panel(PanelKind::Recent, rows);
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
    let graphics = self.take_graphics();
    let tab = self.tab();
    let app = tab.panes.remove(tab.focus);
    tab.tree.remove(app.id);
    tab.focus = tab.order().first().and_then(|id| tab.index(*id)).unwrap_or(0);
    tab.last = None;
    self.tabs.insert(self.active + 1, Tab::new(app));
    self.active += 1;
    self.give(graphics);
    self.arrange()
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
      Some(Request::Tab(step)) => self.activate(self.step(step)),
      Some(Request::Window { command, count, explicit, file }) => {
        self.window(command, (count, explicit), file.as_deref())
      }
      Some(Request::Panel(kind)) => {
        let rows = if kind == PanelKind::Marks {
          let global = self.state.marks.clone();
          self.app().mark_rows(&global)
        } else {
          crate::panel::recent_rows(&self.state.recent)
        };
        self.app().show_panel(kind, rows)
      }
      Some(Request::SetMark(name)) => {
        let app = self.app();
        let mark = Mark { path: app.path.clone(), cursor: app.reader.cursor };
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
        Some(path) => self.choose(&path, screen),
        None => Ok(()),
      },
      Some(Request::Choose(path)) => self.choose(&path, screen),
      Some(Request::TabClose) => self.close(),
      Some(Request::QuitAll) => {
        self.quit = true;
        Ok(())
      }
      None => Ok(()),
    }
  }

  /// In a split, a picked document replaces the focused pane, like `:e`;
  /// otherwise it opens a tab.
  fn choose(&mut self, path: &Path, screen: Screen) -> Result<()> {
    let tab = self.tab().panes.len() == 1;
    self.open(path, tab, screen)
  }

  fn running(&self) -> bool {
    !self.quit
      && !self.hangup.load(Ordering::Relaxed)
      && (!self.tabs.is_empty() || self.home.is_some())
  }

  /// Presses focus; the wheel scrolls in place.
  pub fn mouse(&mut self, mouse: MouseEvent) -> Result<()> {
    let tab = &self.tabs[self.active];
    // Snapshot before disarm hides clue rows.
    let panes: Vec<_> = tab.panes.iter().map(Pane::of).collect();
    let focused = tab.panes[tab.focus].id;
    let route = self.pointer.route(&panes, focused, mouse);
    if pointer::interrupts(mouse.kind) {
      self.app().disarm();
    }
    let Some(route) = route else { return Ok(()) };
    let Some(index) = self.tab().index(route.pane) else { return Ok(()) };
    if route.focus {
      self.focus(self.active, index)?;
    }
    self.tab().panes[index].mouse(route)
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

  /// Drops cached images after another program had the screen.
  fn images_lost(&mut self, out: &mut impl Write) -> Result<bool> {
    if !terminal::images_lost() {
      return Ok(false);
    }
    crate::graphics::release(out)?;
    self.app().graphics.lost();
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.requested.clear();
      app.blank = false;
    }
    Ok(true)
  }

  /// Draws the start screen and handles one input.
  fn home(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(mut home) = self.home.take() else { return Ok(()) };
    home.draw(out)?;
    let event = loop {
      let event = event::read()?;
      if !moved(&event) {
        break event;
      }
    };
    let outcome = home.event(event, &mut self.history);
    match outcome {
      Ok(Outcome::Open(path)) => match self.open(&path, true, home.screen) {
        Ok(()) => return Ok(()),
        Err(error) => home.error = Some(error.to_string()),
      },
      Ok(Outcome::Quit) => return Ok(()),
      Ok(Outcome::Pick) => {
        let picked = std::env::current_dir()
          .map_err(anyhow::Error::from)
          .and_then(|root| crate::fzf::pick(&root));
        match picked {
          Ok(Some(path)) => match self.open(&path, true, home.screen) {
            Ok(()) => return Ok(()),
            Err(error) => home.error = Some(error.to_string()),
          },
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
    write!(out, "\x1b[?2026h\x1b[0m")?;
    self.app().graphics.begin(out)?;
    let area = self.screen.map_or_default(full);
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
    write!(out, "\x1b[?2026l")?;
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

/// How the reader starts.
pub struct Launch {
  /// Extracted text instead of Kitty graphics.
  pub text: bool,
  /// Page to open instead of the saved position.
  pub page: Option<usize>,
  /// Frames per second while anything moves.
  pub fps: u32,
}

/// Opens `paths`, or the start screen without any.
pub fn view(paths: &[PathBuf], password: &str, launch: &Launch) -> Result<()> {
  anyhow::ensure!(io::stdout().is_terminal(), "TUI requires a terminal");
  let mut session =
    Session::new(password, launch.text, crate::state::location());
  session.frame = Duration::from_secs(1) / launch.fps.max(1);
  crate::hangup::watch(
    session.store.clone(),
    Arc::clone(&session.hangup),
    Arc::clone(&session.latest),
  )?;
  terminal::restore_on_panic();
  let _terminal = Terminal::enter()?;
  // Before any pane sizes its image budget or reads input; a spool that
  // fails to start leaves tiles encoded.
  if !launch.text {
    let _ = crate::spool::start(crate::graphics::session());
  }
  let screen = terminal::size()?;
  if paths.is_empty() {
    session.home = Some(Home::new(&session.state.recent, screen));
    return run_saving(&mut session);
  }
  for path in paths {
    session.open(path, true, screen)?;
  }
  session.activate(0)?;
  let app = session.app();
  if let Some(page) = launch.page {
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
  let tab = session.tab();
  for index in 0..tab.panes.len() {
    dirty |= tab.lend(index, App::raster_updates);
    let app = &mut tab.panes[index];
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

/// A bare pointer move, which all-motion mouse tracking reports.
fn moved(event: &Event) -> bool {
  matches!(event, Event::Mouse(mouse) if mouse.kind == MouseEventKind::Moved)
}

/// Handles one input; errors show in the focused pane.
fn input(session: &mut Session, event: Event) -> Result<()> {
  match event {
    Event::Key(key) if key.kind != KeyEventKind::Release => {
      let tab = &mut session.tabs[session.active];
      let app = &mut tab.panes[tab.focus];
      app.reader.message.clear();
      app.error = None;
      // Clues close even when the key fails.
      let result = app.key(key, &mut session.history);
      app.after_key();
      result
    }
    Event::Paste(text) => {
      let tab = &mut session.tabs[session.active];
      tab.panes[tab.focus].paste(&text, &mut session.history)
    }
    Event::Resize(_, _) => session.resize(terminal::size()?),
    Event::Mouse(mouse) => session.mouse(mouse),
    _ => Ok(()),
  }
}

fn run(session: &mut Session) -> Result<()> {
  let mut out = io::BufWriter::new(io::stdout());
  let mut trace = crate::trace::Trace::new()?;
  let mut buffer = Vec::with_capacity(64 * 1024);
  let mut dirty = true;
  let mut pacer = Pacer::new(Instant::now(), session.frame);
  let mut checked = Instant::now();
  let mut written = Instant::now();
  let mut touched = Instant::now();
  let mut watched = Instant::now();
  let mut title = String::new();
  while session.running() {
    if terminal::resize_deferred() {
      let resized = terminal::size().and_then(|size| session.resize(size));
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
    let now = Instant::now();
    if pacer.ready(now) {
      for app in &mut session.tab().panes {
        dirty |= app.animate();
      }
      if dirty {
        let current = session.app().title();
        if current != title {
          let clean = terminal::clean(&current, 200);
          write!(out, "\x1b]2;{clean}\x07")?;
          title = current;
        }
        trace.draw(session, &mut out, &mut buffer)?;
        dirty = false;
        pacer.drawn(now);
      }
    }
    let animating =
      session.tab().panes.iter().any(|app| app.animation.is_some());
    let pending = dirty || animating;
    let now = Instant::now();
    let wait = if pending { pacer.wait(now) } else { Duration::from_millis(8) };
    if !event::poll(wait)? {
      dirty |= session.app().clue_due();
      let force = watched.elapsed() > Duration::from_secs(2);
      if force {
        watched = Instant::now();
      }
      dirty |= watch(session, force);
      // Positions reach memory every 250 ms and disk after a 5 s pause
      // or once a minute; exits and hangups write the rest.
      if checked.elapsed() > Duration::from_millis(250) {
        checked = Instant::now();
        session.record();
        let due = touched.elapsed() > Duration::from_secs(5)
          || written.elapsed() > Duration::from_secs(60);
        match if due { session.flush() } else { Ok(false) } {
          Ok(true) => written = checked,
          Ok(false) => {}
          Err(error) => {
            session.app().error = Some(format!("State not saved: {error}"));
          }
        }
      }
      continue;
    }
    let event = event::read()?;
    if moved(&event) {
      session.pointer.hover();
      continue;
    }
    touched = Instant::now();
    trace.input();
    let result = input(session, event).and_then(|()| session.settle());
    report(session, result);
    dirty = true;
  }
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
