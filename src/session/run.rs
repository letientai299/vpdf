use super::{
  Act, App, Arc, Cause, Duration, Event, Home, Instant, IsTerminal,
  KeyEventKind, MouseKind, Ordering, Outcome, Pacer, PageId, PathBuf, Reply,
  Result, Session, Shape, Sight, View, Write, io, sight, terminal,
};
use crate::pdf_text::PdfText;

/// Opens paths or the start screen.
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
  session.observer = crate::tmux_observer::Observer::start();
  // Before input; a spool that fails to start leaves tiles encoded.
  let _ = crate::spool::start(crate::graphics::session());
  // The image cache reads the probe and the spool for its budget.
  session.graphics = crate::graphics::Graphics::default();
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
      &mut PdfText(&mut app.doc),
    )?;
    app.show_page(app.reader.cursor.page)?;
    app.ensure_cursor()?;
  }
  run_saving(&mut session)
}

/// Runs the session and saves its state even when the loop fails, as
/// after the terminal hangs up.
pub(super) fn run_saving(session: &mut Session) -> Result<()> {
  let result = run(session);
  let saved = session.finish();
  result.and(saved)
}

/// Background results for every pane of the active tab.
pub(super) fn updates(
  session: &mut Session,
  out: &mut impl Write,
) -> Result<bool> {
  let mut dirty = session.images_lost(out)?;
  dirty |= session.image_updates(out);
  let tab = &mut session.tabs[session.active];
  for app in &mut tab.panes {
    dirty |= app.raster_updates(&mut session.graphics);
    let shelf = app.panel.as_mut().and_then(|panel| panel.shelf.as_mut());
    dirty |= shelf.is_some_and(crate::recent::Shelf::updates);
    // A failed update shows in its pane instead of ending the session.
    let updated = app.search_updates().and_then(|search| {
      Ok(
        search
          | app.ocr_updates()?
          | app.outline_updates()?
          | app.crop_updates()?,
      )
    });
    dirty |= updated.unwrap_or_else(|error| {
      app.error = Some(error.to_string());
      true
    });
  }
  Ok(dirty)
}

/// Reloads the active tab's panes after their files change; their old
/// images go at the next draw.
pub(super) fn watch(session: &mut Session, force: bool) -> bool {
  let touched = session.watcher.touched();
  let tab = &mut session.tabs[session.active];
  let mut dirty = false;
  for index in 0..tab.panes.len() {
    let app = &tab.panes[index];
    if !(force || app.settling.is_some() || touched.contains(&app.path)) {
      continue;
    }
    let app = &mut tab.panes[index];
    dirty |= app.watch().unwrap_or_else(|error| {
      app.error = Some(format!("Reload failed: {error}"));
      true
    });
  }
  dirty
}

/// Pointer input the start screen ignores: motion, drags, leaving.
pub(super) fn passive(event: &Event) -> bool {
  match event {
    Event::Mouse(mouse) => {
      matches!(mouse.kind, MouseKind::Moved | MouseKind::Drag(_))
    }
    Event::Leave => true,
    _ => false,
  }
}

pub(super) fn act(event: Event) -> Option<Act> {
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
pub(super) fn input(
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
pub(super) fn apply(
  session: &mut Session,
  act: Act,
  out: &mut impl Write,
) -> bool {
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

pub(super) fn run(session: &mut Session) -> Result<()> {
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
    dirty |= session.observe(&mut out)?;
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
pub(super) struct Chores {
  checked: Instant,
  written: Instant,
  touched: Instant,
  watched: Instant,
}

impl Chores {
  pub(super) fn new(now: Instant) -> Self {
    Self { checked: now, written: now, touched: now, watched: now }
  }

  /// Clues, file checks, tmux looks, and saved positions; whether a
  /// frame is due.
  pub(super) fn run(
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
    if force && session.observer.is_none() && session.sight != Sight::Focused {
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
pub(super) fn set_title(out: &mut impl Write, title: &str) -> Result<()> {
  if crate::caps::get().is_none_or(crate::caps::Caps::title_stack) {
    write!(out, "\x1b]2;{}\x07", terminal::clean(title, 200))?;
  }
  Ok(())
}

/// How long the idle loop may sleep: until the next file check, state
/// check after a change, clue, or settled file change.
pub(super) fn idle(
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
pub(super) enum Landing {
  Wait,
  /// A document opened in a tab.
  Opened,
  /// The start screen redraws: its prompt took text, or an error shows.
  Shown,
}

/// Sends a reply the terminal waits for.
pub(super) fn answer(out: &mut impl Write, reply: &str) -> Result<()> {
  out.write_all(reply.as_bytes())?;
  out.flush()?;
  Ok(())
}

/// Shows a failure in the focused pane instead of ending the session.
pub(super) fn report(session: &mut Session, result: Result<()>) {
  if let Err(error) = result
    && !session.tabs.is_empty()
  {
    session.app().error = Some(error.to_string());
  }
}

impl Session {
  pub(super) fn running(&self) -> bool {
    !self.quit
      && !self.hangup.load(Ordering::Relaxed)
      && (!self.tabs.is_empty() || self.home.is_some())
  }

  /// Shows the next tmux notice once the status line is free; whether
  /// it did.
  pub(super) fn notify(&mut self) -> bool {
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

  /// Asks tmux how it shows the pane, for `cause`; whether the reader
  /// shows again and needs a frame.
  pub(super) fn look(
    &mut self,
    cause: Cause,
    out: &mut impl Write,
  ) -> Result<bool> {
    match terminal::pane_view() {
      Some(view) => self.turn(sight(self.sight, cause, view), out),
      // The tick asks again.
      None if cause == Cause::Tick => Ok(false),
      None => self.turn(Sight::Unfocused, out),
    }
  }

  /// Moves to `sight`, hiding on the way in; whether the reader shows
  /// again and needs a frame.
  pub(super) fn turn(
    &mut self,
    sight: Sight,
    out: &mut impl Write,
  ) -> Result<bool> {
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
  pub(super) fn shown_by(
    &mut self,
    act: &Act,
    out: &mut impl Write,
  ) -> Result<bool> {
    let sight = match act {
      Act::Key(_) => Sight::Focused,
      Act::Mouse(_) if self.sight == Sight::Hidden => Sight::Unfocused,
      _ => return Ok(false),
    };
    self.turn(sight, out)
  }

  fn observe(&mut self, out: &mut impl Write) -> Result<bool> {
    let Some(next) =
      self.observer.as_ref().and_then(crate::tmux_observer::Observer::take)
    else {
      return Ok(false);
    };
    self.observation(next, out)
  }

  pub(super) fn observation(
    &mut self,
    next: crate::tmux_observer::Snapshot,
    out: &mut impl Write,
  ) -> Result<bool> {
    let initial = self.observed.is_none();
    let lost =
      self.observed.as_ref().is_some_and(|old| old.clients != next.clients);
    let returned = self
      .observed
      .as_ref()
      .is_some_and(|old| old.view == View::Hidden && next.view != View::Hidden);
    if lost {
      self.lose_images(out)?;
    }
    let cause =
      if initial || returned || lost { Cause::Start } else { Cause::Tick };
    let dirty = self.turn(sight(self.sight, cause, next.view), out)?;
    let resized = if let Some(screen) = next.screen
      && self.screen != Some(screen)
    {
      self.resize(screen)?;
      true
    } else {
      false
    };
    self.observed = Some(next);
    Ok(dirty || resized || lost)
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
  pub(super) fn home(
    &mut self,
    out: &mut crate::perf::Output<impl Write>,
  ) -> Result<()> {
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
