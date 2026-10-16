use crate::Result;
use crate::app::{App, Request, TabMove, Tabline, Window};
use crate::graphics::Graphics;
use crate::home::{Home, Outcome};
use crate::keys::PanelKind;
use crate::layout::Screen;
use crate::prompt::History;
use crate::split::{Axis, Node, Rect, neighbor};
use crate::state::{Mark, State};
use crate::terminal::{self, At, Terminal};
use crate::text::PageId;
use crate::theme;
use crate::trace::Drawable;
use crate::watch::Watcher;
use crossterm::event::{self, Event, KeyEventKind, MouseEvent};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
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
}

impl Session {
  /// `store` holds recent documents and history between runs.
  pub fn new(password: &str, text: bool, store: Option<PathBuf>) -> Self {
    let state = store.as_deref().map(State::load).unwrap_or_default();
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
    }
  }

  pub fn save(&mut self) -> Result<()> {
    let Some(store) = &self.store else { return Ok(()) };
    self.state.save(store, &self.history)
  }

  /// Records the focused pane and saves when it moved.
  fn checkpoint(&mut self) -> Result<()> {
    if !self.running() || self.tabs.is_empty() {
      return Ok(());
    }
    let tab = &self.tabs[self.active];
    if self.state.remember(&tab.panes[tab.focus]) {
      self.save()?;
    }
    Ok(())
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
      .map(|app| std::mem::take(&mut app.graphics))
      .unwrap_or_default()
  }

  /// Moves focus to pane `pane` of tab `tab`, with the graphics.
  fn focus(&mut self, tab: usize, pane: usize) -> Result<()> {
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
    let app = self.load(&path, screen)?;
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
    graphics.forget(old.id);
    self.retire(&old);
    self.give(graphics);
    self.arrange()
  }

  /// Loads `path` and restores its saved view.
  fn load(&mut self, path: &Path, screen: Screen) -> Result<App> {
    let mut secret = self.password.clone();
    let mut app = loop {
      match App::with_screen(path, &secret, self.text, screen) {
        Err(error) if error.to_string() == "PDF password required" => {
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
    Ok(app)
  }

  /// Saves a closing pane's place and stops watching its file.
  fn retire(&mut self, app: &App) {
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
      graphics.forget(app.id);
      self.retire(app);
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
    graphics.forget(app.id);
    self.retire(&app);
    self.give(graphics);
    self.arrange()
  }

  /// Splits the focused pane; the new pane shows `file` or the same view.
  fn split(&mut self, axis: Axis, file: Option<&Path>) -> Result<()> {
    let screen = self.app().screen;
    let path = match file {
      Some(file) => std::fs::canonicalize(file)?,
      None => self.app().path.clone(),
    };
    let mut app = self.load(&path, screen)?;
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
    let area = self.screen.map(full).unwrap_or_default();
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
    for app in &closing {
      self.app().graphics.forget(app.id);
      self.retire(app);
    }
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
    !self.quit && (!self.tabs.is_empty() || self.home.is_some())
  }

  /// Sends a click or scroll to the pane under it, focusing that pane.
  fn mouse(&mut self, mut mouse: MouseEvent) -> Result<()> {
    let tab = self.tab();
    let hit = tab.panes.iter().position(|app| {
      let (at, screen) = (app.at, app.screen);
      (at.column..at.column + screen.columns).contains(&mouse.column)
        && (at.row..at.row + screen.rows).contains(&mouse.row)
    });
    let Some(index) = hit else { return Ok(()) };
    if index != tab.focus {
      self.focus(self.active, index)?;
    }
    let app = self.app();
    mouse.column -= app.at.column;
    mouse.row -= app.at.row;
    app.mouse(mouse)
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
    }
    Ok(true)
  }

  /// Draws the start screen and handles one input.
  fn home(&mut self, out: &mut impl Write) -> Result<()> {
    let Some(mut home) = self.home.take() else { return Ok(()) };
    home.draw(out)?;
    let outcome = home.event(event::read()?, &mut self.history);
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
    let area = self.screen.map(full).unwrap_or_default();
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

/// Opens `paths`, or the start screen without any.
pub fn view(
  paths: &[PathBuf],
  password: &str,
  text: bool,
  page: Option<usize>,
) -> Result<()> {
  anyhow::ensure!(io::stdout().is_terminal(), "TUI requires a terminal");
  let mut session = Session::new(password, text, crate::state::location());
  let _terminal = Terminal::enter()?;
  let screen = terminal::size()?;
  if paths.is_empty() {
    session.home = Some(Home::new(&session.state.recent, screen));
    run(&mut session)?;
    return session.finish();
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
  run(&mut session)?;
  session.finish()
}

/// Background results for every pane of the active tab.
fn updates(session: &mut Session, out: &mut impl Write) -> Result<bool> {
  let mut dirty = session.images_lost(out)?;
  let tab = session.tab();
  for index in 0..tab.panes.len() {
    dirty |= tab.lend(index, App::raster_updates);
    let app = &mut tab.panes[index];
    dirty |= app.search_updates()?;
    dirty |= app.ocr_updates()?;
    dirty |= app.outline_updates()?;
  }
  Ok(dirty)
}

/// Reloads the focused pane after its file changes.
fn watch(session: &mut Session, force: bool) -> bool {
  let path = session.app().path.clone();
  let touched = session.watcher.touched(&path);
  let app = session.app();
  if !(touched || force || app.settling.is_some()) {
    return false;
  }
  match app.watch() {
    Ok(reloaded) => reloaded,
    Err(error) => {
      app.error = Some(format!("Reload failed: {error}"));
      true
    }
  }
}

/// Handles one input; errors show in the focused pane.
fn input(session: &mut Session, event: Event) -> Result<()> {
  match event {
    Event::Key(key) if key.kind != KeyEventKind::Release => {
      let tab = &mut session.tabs[session.active];
      let app = &mut tab.panes[tab.focus];
      app.reader.message.clear();
      app.error = None;
      app.key(key, &mut session.history)?;
      app.after_key()
    }
    Event::Paste(text) => {
      if let Some(prompt) = &mut session.app().prompt {
        prompt.insert(&text);
      }
      Ok(())
    }
    Event::Resize(_, _) => {
      session.screen = Some(terminal::size()?);
      for app in session.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
        app.at = At { column: u16::MAX, row: u16::MAX };
      }
      session.arrange()
    }
    Event::Mouse(mouse) => session.mouse(mouse),
    _ => Ok(()),
  }
}

fn run(session: &mut Session) -> Result<()> {
  let mut out = io::BufWriter::new(io::stdout());
  let mut trace = crate::trace::Trace::new()?;
  let mut buffer = Vec::with_capacity(64 * 1024);
  let mut dirty = true;
  let mut last_frame = Instant::now() - Duration::from_millis(9);
  let mut saved = Instant::now();
  let mut watched = Instant::now();
  let mut title = String::new();
  while session.running() {
    if session.tabs.is_empty() {
      session.home(&mut out)?;
      dirty = true;
      continue;
    }
    dirty |= updates(session, &mut out)?;
    session.app().clipboard(&mut out)?;
    if last_frame.elapsed() >= Duration::from_micros(8333) {
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
        last_frame = Instant::now();
      }
    }
    let animating =
      session.tab().panes.iter().any(|app| app.animation.is_some());
    let wait = if !dirty && !animating {
      Duration::from_millis(8)
    } else {
      Duration::from_micros(8333)
        .saturating_sub(last_frame.elapsed())
        .max(Duration::from_millis(1))
    };
    if !event::poll(wait)? {
      dirty |= session.app().clue_due()?;
      let force = watched.elapsed() > Duration::from_secs(2);
      if force {
        watched = Instant::now();
      }
      dirty |= watch(session, force);
      if saved.elapsed() > Duration::from_secs(2) {
        saved = Instant::now();
        if let Err(error) = session.checkpoint() {
          session.app().error = Some(format!("State not saved: {error}"));
        }
      }
      continue;
    }
    trace.input();
    let result = input(session, event::read()?);
    if let Err(error) = result.and_then(|()| session.settle())
      && !session.tabs.is_empty()
    {
      session.app().error = Some(error.to_string());
    }
    dirty = true;
  }
  Ok(())
}
