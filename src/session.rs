use crate::Result;
use crate::app::{App, Request, TabMove, Tabline};
use crate::graphics::Graphics;
use crate::home::{Home, Outcome};
use crate::keys::PanelKind;
use crate::layout::Screen;
use crate::prompt::History;
use crate::state::State;
use crate::terminal::{self, Terminal};
use crate::text::PageId;
use crossterm::event::{self, Event, KeyEventKind};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Open documents as tabs; only the active tab draws and owns graphics.
pub struct Session {
  pub tabs: Vec<App>,
  pub active: usize,
  pub history: History,
  pub quit: bool,
  pub state: State,
  /// Start screen while no document is open.
  pub home: Option<Home>,
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
      store,
      password: password.to_owned(),
      text,
    }
  }

  pub fn save(&mut self) -> Result<()> {
    let Some(store) = &self.store else { return Ok(()) };
    self.state.save(store, &self.history)
  }

  /// Records the active tab and saves when it moved.
  fn checkpoint(&mut self) -> Result<()> {
    if self.running() && self.state.remember(&self.tabs[self.active]) {
      self.save()?;
    }
    Ok(())
  }

  /// Records every tab and saves before exit.
  fn finish(&mut self) -> Result<()> {
    let active = self.tabs.get(self.active);
    let others =
      self.tabs.iter().filter(|app| active.is_none_or(|a| a.id != app.id));
    for app in others.chain(active) {
      self.state.remember(app);
    }
    self.save()
  }

  pub fn app(&mut self) -> &mut App {
    &mut self.tabs[self.active]
  }

  /// Opens `path` in a new tab, or in place of the active one.
  pub fn open(&mut self, path: &Path, tab: bool, screen: Screen) -> Result<()> {
    let path = std::fs::canonicalize(path)?;
    if let Some(index) = self.tabs.iter().position(|app| app.path == path) {
      return self.activate(index);
    }
    let mut app = self.load(&path, screen)?;
    if let Some(entry) = self.state.entry(&path) {
      app.restore(entry)?;
    }
    self.state.remember(&app);
    if self.tabs.is_empty() {
      self.tabs.push(app);
      return self.install(Graphics::default(), screen);
    }
    if tab {
      self.tabs.insert(self.active + 1, app);
      return self.activate(self.active + 1);
    }
    let mut old = std::mem::replace(&mut self.tabs[self.active], app);
    self.state.remember(&old);
    old.graphics.forget(old.id);
    self.install(std::mem::take(&mut old.graphics), screen)
  }

  fn load(&self, path: &Path, screen: Screen) -> Result<App> {
    let mut secret = self.password.clone();
    loop {
      match App::with_screen(path, &secret, self.text, screen) {
        Err(error) if error.to_string() == "PDF password required" => {
          secret = crate::prompt::password()?;
        }
        result => return result,
      }
    }
  }

  pub fn activate(&mut self, index: usize) -> Result<()> {
    if index == self.active || index >= self.tabs.len() {
      return Ok(());
    }
    let old = &mut self.tabs[self.active];
    let screen = old.screen;
    let graphics = std::mem::take(&mut old.graphics);
    self.active = index;
    self.install(graphics, screen)
  }

  /// Closes the active tab; the session ends after the last.
  pub fn close(&mut self) -> Result<()> {
    let mut app = self.tabs.remove(self.active);
    self.state.remember(&app);
    if self.tabs.is_empty() {
      return self.save();
    }
    app.graphics.forget(app.id);
    self.active = self.active.min(self.tabs.len() - 1);
    self.install(std::mem::take(&mut app.graphics), app.screen)
  }

  /// Hands the shared graphics and screen to the active tab.
  fn install(&mut self, mut graphics: Graphics, screen: Screen) -> Result<()> {
    let names = self.tabs.iter().map(App::name).collect();
    let app = &mut self.tabs[self.active];
    graphics.doc = app.id;
    app.graphics = graphics;
    app.requested.clear();
    app.screen = screen;
    app.tabline = Tabline { names, active: self.active };
    app.relayout()
  }

  fn step(&self, step: TabMove) -> usize {
    let count = self.tabs.len();
    match step {
      TabMove::Next(n) => (self.active + n) % count,
      TabMove::Previous(n) => (self.active + count - n % count) % count,
      TabMove::Go(n) => n.clamp(1, count) - 1,
    }
  }

  /// Applies what the active tab asked for after an input.
  pub fn settle(&mut self) -> Result<()> {
    let app = &mut self.tabs[self.active];
    if app.quit {
      return self.close();
    }
    let screen = app.screen;
    match app.request.take() {
      Some(Request::Open { path, tab }) => self.open(&path, tab, screen),
      Some(Request::Tab(step)) => self.activate(self.step(step)),
      Some(Request::Recent) => {
        let rows = crate::panel::recent_rows(&self.state.recent);
        self.app().show_panel(PanelKind::Recent, rows)
      }
      Some(Request::QuitAll) => {
        self.quit = true;
        Ok(())
      }
      None => Ok(()),
    }
  }

  fn running(&self) -> bool {
    !self.quit && (!self.tabs.is_empty() || self.home.is_some())
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
      Ok(Outcome::Stay) => {}
      Err(error) => home.error = Some(error.to_string()),
    }
    self.home = Some(home);
    Ok(())
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
    let app = &mut session.tabs[session.active];
    dirty |= app.raster_updates(&mut out)?;
    dirty |= app.search_updates()?;
    dirty |= app.ocr_updates()?;
    app.clipboard(&mut out)?;
    let frame_due = last_frame.elapsed() >= Duration::from_micros(8333);
    if frame_due {
      dirty |= app.animate();
      if dirty {
        let current = app.title();
        if current != title {
          let clean = terminal::clean(&current, 200);
          write!(out, "\x1b]2;{clean}\x07")?;
          title = current;
        }
        trace.draw(app, &mut out, &mut buffer)?;
        dirty = false;
        last_frame = Instant::now();
      }
    }
    let wait = if !dirty && app.animation.is_none() {
      Duration::from_millis(8)
    } else {
      Duration::from_micros(8333)
        .saturating_sub(last_frame.elapsed())
        .max(Duration::from_millis(1))
    };
    if !event::poll(wait)? {
      if watched.elapsed() > Duration::from_millis(400) {
        watched = Instant::now();
        let app = session.app();
        match app.watch() {
          Ok(reloaded) => dirty |= reloaded,
          Err(error) => {
            app.error = Some(format!("Reload failed: {error}"));
            dirty = true;
          }
        }
      }
      if saved.elapsed() > Duration::from_secs(2) {
        saved = Instant::now();
        if let Err(error) = session.checkpoint() {
          session.app().error = Some(format!("State not saved: {error}"));
        }
      }
      continue;
    }
    trace.input();
    let event = event::read()?;
    if matches!(event, Event::Key(_)) {
      app.reader.message.clear();
      app.error = None;
    }
    let result = match event {
      Event::Key(key) if key.kind != KeyEventKind::Release => {
        app.key(key, &mut session.history)
      }
      Event::Paste(text) => {
        if let Some(prompt) = &mut app.prompt {
          prompt.insert(&text);
        }
        Ok(())
      }
      Event::Resize(_, _) => app.resize(),
      Event::Mouse(mouse) => app.mouse(mouse),
      _ => Ok(()),
    };
    if let Err(error) = result.and_then(|()| session.settle()) {
      session.app().error = Some(error.to_string());
    }
    dirty = true;
  }
  Ok(())
}
