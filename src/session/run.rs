use super::interaction::Interaction;
use super::maintenance::Chores;
use super::{
  Act, Arc, Cause, Duration, Event, Instant, IsTerminal, Ordering, Pacer,
  Result, Session, Sight, View, Write, io, sight, terminal,
};
use vpdf_terminal::input::{Notice, notice};

/// Opens paths or the start screen.
pub fn view(
  references: &[vpdf_model::reference::Reference],
  config: crate::config::Loaded,
) -> Result<()> {
  anyhow::ensure!(io::stdout().is_terminal(), "TUI requires a terminal");
  let mut session = Session::configured(crate::state::location(), config);
  crate::state::watch_hangup(
    session.store.clone(),
    Arc::clone(&session.hangup),
    Arc::clone(&session.latest),
  )?;
  terminal::restore_on_panic();
  let (_terminal, screen) = vpdf_terminal::gate::enter()?;
  session.set_system_palette(
    vpdf_terminal::caps::get()
      .map_or_default(vpdf_terminal::caps::Caps::palette),
  );
  session.observer = vpdf_terminal::tmux_observer::Observer::start();
  // Before input; a spool that fails to start leaves tiles encoded.
  let _ = vpdf_terminal::spool::start(vpdf_terminal::graphics::session());
  // The image cache reads the probe and the spool for its budget.
  session.graphics = vpdf_terminal::graphics::Graphics::live();
  if references.is_empty() {
    session.recent_tab(screen)?;
    return run_saving(&mut session);
  }
  for reference in references {
    session.open_reference(reference, true, screen)?;
  }
  session.activate(0)?;
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
  for app in tab.panes.iter_mut().filter_map(super::Pane::document_mut) {
    dirty |= app.raster_updates(&mut session.graphics);
    dirty |= app.job_updates();
  }
  dirty |= session.recent_updates();
  Ok(dirty)
}

pub(super) fn act(event: Event) -> Option<Act> {
  match event {
    Event::Key(key) if key.kind != vpdf_model::input::KeyEventKind::Release => {
      Some(Act::Key(key))
    }
    Event::Paste(text) => Some(Act::Paste(text)),
    Event::Resize(report) => Some(Act::Resize(report)),
    Event::Mouse(mouse) => (mouse.action().is_some()
      || mouse.history().is_some())
    .then_some(Act::Mouse(mouse)),
    Event::Leave => Some(Act::Leave),
    Event::Reply(reply) => notice(&reply).map(|notice| match notice {
      Notice::Missing(id) => Act::Missing(id),
      Notice::ImageStatus(body) => Act::ImageStatus(body.into()),
      Notice::Drop(body) => Act::Drop(body.into()),
    }),
    Event::Focus(gained) => Some(Act::Focus(gained)),
    Event::Key(_) => None,
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
    Act::Key(key) => return session.interact(Interaction::Key(key)),
    Act::Paste(text) => return session.interact(Interaction::Paste(text)),
    Act::Resize(report) => {
      let measured = terminal::resized(report, session.screen);
      session.refit(measured, crossterm::terminal::size)
    }
    Act::Mouse(mouse) => {
      return Ok(session.interact(Interaction::Mouse(mouse))? || shown);
    }
    Act::Leave => {
      let changed = session.interact(Interaction::Leave)?;
      return session.settle().map(|()| changed);
    }
    Act::Drop(body) => {
      let changed = session.drop_event(&body, out)?;
      return session.settle().map(|()| changed);
    }
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
  done.and_then(|()| session.settle()).map(|()| true)
}

/// Handles and settles one input; whether it needs a frame.
pub(super) fn apply(
  session: &mut Session,
  act: Act,
  out: &mut impl Write,
) -> bool {
  let result = input(session, act, out);
  result.unwrap_or_else(|error| {
    report(session, Err(error));
    true
  })
}

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
    dirty |= session.clean_state();
    dirty |= session.help_updates(&mut out);
    dirty |= session.print_update();
    if terminal::resize_deferred() {
      let measured = terminal::remeasure().map(Some);
      let resized = session.refit(measured, crossterm::terminal::size);
      report(session, resized);
      dirty = true;
    }
    dirty |= session.observe(&mut out)?;
    session.perf.bytes(Instant::now(), out.take());
    session.perf_tick(&mut out)?;
    dirty |= updates(session, &mut out)?;
    if let Ok(app) = session.app() {
      app.clipboard(&mut out)?;
    }
    dirty |= session.notify();
    let now = Instant::now();
    let shown = session.sight != Sight::Hidden;
    if pacer.ready(now) && shown {
      dirty |= session.animate(now);
      if dirty {
        let current = session.focused().title();
        if current != title {
          terminal::set_title(&mut out, &current)?;
          title = current;
        }
        trace.draw(session, &mut out, &mut buffer)?;
        session.drawn();
        dirty = session.redraw_due();
        pacer.drawn(now);
        drawn = now;
      }
    }
    let animating = session.animating();
    // A hidden reader draws nothing until tmux shows it again.
    let pending = (dirty || animating) && shown;
    let now = Instant::now();
    let changed = chores.drawn_since(drawn);
    let wait = if pending {
      pacer.wait(now)
    } else {
      chores.wait(session, now, changed)
    };
    let wait = wait_time(session, wait, now);
    let event = vpdf_terminal::input::next(Some(wait))?;
    // Streams of pointer reports must not starve them.
    if event.is_none() || now.elapsed() >= wait {
      let now = Instant::now();
      let watch_due = chores.watch_due(now);
      dirty |= chores.run(session, now);
      if watch_due
        && session.observer.is_none()
        && session.sight != Sight::Focused
      {
        dirty |= session.look(Cause::Tick, &mut out)?;
      }
    }
    #[cfg(feature = "bench-events")]
    if let Some(event) = &event
      && trace.benchmark(event)?
    {
      dirty = true;
      continue;
    }
    let Some(act) = event.and_then(act) else { continue };
    let (typed, arrived) = (act.typed(), Instant::now());
    // Motion and per-pixel drags often change nothing.
    if apply(session, act, &mut out) {
      if typed {
        chores.touch(arrived);
        trace.input(arrived)?;
      }
      dirty = true;
    }
  }
  Ok(())
}

pub(super) fn wait_time(
  session: &Session,
  mut wait: Duration,
  now: Instant,
) -> Duration {
  wait = session.print_wait(wait);
  if let Some(image) = session.image_wait(now) {
    wait = wait.min(image);
  }
  if session.state.perf && session.sight != Sight::Hidden {
    wait = wait.min(session.perf.wait(now));
  }
  wait
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
    session.focused_mut().fail(error.to_string());
  }
}

impl Session {
  pub(super) fn running(&self) -> bool {
    !self.quit && !self.hangup.load(Ordering::Relaxed) && !self.tabs.is_empty()
  }

  /// Shows the next tmux notice once the status line is free; whether
  /// it did.
  pub(super) fn notify(&mut self) -> bool {
    let now = Instant::now();
    let mut changed = self.image_timeout(now);
    changed |= self.maintain(now);
    let Ok(app) = self.app() else { return changed };
    let reader = &mut app.reader;
    if !reader.message.is_empty() {
      return changed;
    }
    let Some(notice) = terminal::notice() else { return changed };
    reader.notice(notice);
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
    let Some(next) = self
      .observer
      .as_ref()
      .and_then(vpdf_terminal::tmux_observer::Observer::take)
    else {
      return Ok(false);
    };
    self.observation(next, out)
  }

  pub(super) fn observation(
    &mut self,
    next: vpdf_terminal::tmux_observer::Snapshot,
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
}
