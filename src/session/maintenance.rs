//! Shared session maintenance policy.
use super::{App, Duration, Instant, Session};

/// Reading-state check interval.
const CHECK: Duration = Duration::from_millis(250);
/// Fallback file-watch interval.
const WATCH: Duration = Duration::from_secs(2);

/// Reloads changed files in the active tab.
fn watch(session: &mut Session, force: bool, now: Instant) -> bool {
  let touched = session.watcher.touched();
  let tab = &mut session.tabs[session.active];
  let mut dirty = false;
  for index in 0..tab.panes.len() {
    let app = &tab.panes[index];
    if !(force || app.settling.is_some() || touched.contains(&app.path)) {
      continue;
    }
    let app = &mut tab.panes[index];
    dirty |= app.watch_at(now).unwrap_or_else(|error| {
      app.error = Some(format!("Reload failed: {error}"));
      true
    });
  }
  dirty
}

/// Maintenance timestamps and last input.
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

  pub(super) fn wait(
    &self,
    session: &mut Session,
    now: Instant,
    changed: bool,
  ) -> Duration {
    let wait = idle(
      session,
      self.checked,
      self.watched,
      changed || self.touched > self.checked,
      now,
    );
    if session.sidebars_due() { Duration::ZERO } else { wait }
  }

  pub(super) fn watch_due(&self, now: Instant) -> bool {
    now.duration_since(self.watched) >= WATCH
  }

  pub(super) fn drawn_since(&self, drawn: Instant) -> bool {
    drawn > self.checked
  }

  pub(super) fn touch(&mut self, now: Instant) {
    self.touched = now;
  }

  /// Checks files, clues, and saved positions.
  pub(super) fn run(&mut self, session: &mut Session, now: Instant) -> bool {
    let mut dirty = session.app().clue_due(now);
    let force = self.watch_due(now);
    if force {
      self.watched = now;
    }
    dirty |= watch(session, force, now);
    let active = session.active;
    for app in session
      .tabs
      .iter_mut()
      .enumerate()
      .filter(|(index, _)| *index != active)
      .flat_map(|(_, tab)| &mut tab.panes)
    {
      if let Err(error) = app
        .outline_updates()
        .and_then(|_| app.figure_updates())
        .and_then(|_| app.sidebar_updates())
      {
        app.error = Some(error.to_string());
      }
    }
    if session.sidebars_due() {
      for app in session.panes_mut() {
        app.prepare_sidebars();
      }
    }
    // Exits and hangups save remaining changes.
    if now.duration_since(self.checked) >= CHECK {
      self.checked = now;
      session.record();
      let due = now.duration_since(self.touched) > Duration::from_secs(5)
        || now.duration_since(self.written) > Duration::from_secs(60);
      match if due { session.flush() } else { Ok(false) } {
        Ok(true) => self.written = self.checked,
        Ok(false) => {}
        Err(error) => {
          session.app().error = Some(format!("State not saved: {error}"));
        }
      }
    }
    dirty
  }
}

/// Waits until the next shared deadline.
pub(super) fn idle(
  session: &mut Session,
  checked: Instant,
  watched: Instant,
  changed: bool,
  now: Instant,
) -> Duration {
  let mut wait = WATCH.saturating_sub(now.duration_since(watched));
  if changed || session.unsaved {
    wait = wait.min(CHECK.saturating_sub(now.duration_since(checked)));
  }
  if let Some(clue) = session.app().clue_wait(now) {
    wait = wait.min(clue);
  }
  let held = session.pointer.holding();
  let due = session.resting().and_then(App::hover_due);
  if let Some(due) = due.filter(|_| !held) {
    wait = wait.min(due.saturating_duration_since(now));
  }
  for due in
    [session.autoscroll_due(), session.hover_settle_due()].into_iter().flatten()
  {
    wait = wait.min(due.saturating_duration_since(now));
  }
  for app in &session.tab().panes {
    if let Some(until) = app.reader.landing().deadline() {
      wait = wait.min(until.saturating_duration_since(now));
    }
    if let Some(until) = app.bar.deadline() {
      wait = wait.min(until.saturating_duration_since(now));
    }
  }
  if session.tab().panes.iter().any(|app| app.settling.is_some()) {
    wait = wait.min(crate::watch::SETTLE);
  }
  wait
}

impl Session {
  fn sidebars_due(&self) -> bool {
    self.sight != super::Sight::Hidden
      && !self.pointer.holding()
      && self.tabs[self.active]
        .panes
        .iter()
        .all(|app| app.complete && !app.images_stale)
      && self.panes().any(App::sidebars_deferred)
  }

  pub(super) fn maintain(&mut self, now: Instant) -> bool {
    let mut dirty = false;
    for app in &mut self.tab().panes {
      dirty |= app.reader.expire_flash(now);
      dirty |= app.bar_expired(now);
    }
    dirty |= self.autoscroll(now);
    dirty |= self.hover_preview(now);
    dirty |= self.hover_settled(now);
    dirty
  }

  pub(super) fn animate(&mut self, now: Instant) -> bool {
    let mut dirty = false;
    for app in &mut self.tab().panes {
      dirty |= app.animate_at(now);
    }
    dirty
  }

  pub(super) fn animating(&self) -> bool {
    self.tabs[self.active]
      .panes
      .iter()
      .any(|app| app.reader.animation().active())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Result;
  use crate::session::Sight;
  use vpdf_model::text::Position;
  use vpdf_ui::page::landing::{Landing, Span};
  use vpdf_ui::scrollbar::Bar;

  fn opened(root: &std::path::Path) -> Result<Session> {
    let path = root.join("timers.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut session = Session::new(Some(root.join("state.json")));
    session.open(&path, true, crate::screen::cells(80, 24))?;
    session.record();
    session.flush()?;
    Ok(session)
  }

  #[test]
  fn first_view_respects_cancel() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let start = Instant::now();
    let mut chores = Chores::new(start);
    chores.run(&mut session, start);
    assert!(matches!(session.app().figures, crate::jobs::Figures::Deferred));
    session.app().complete = true;
    session.app().start_deferred();
    assert!(session.app().outline_pending());
    assert!(matches!(session.app().figures, crate::jobs::Figures::Running(_)));
    assert!(
      session.app().sidebar_loading(vpdf_ui::keys::PanelKind::Annotations)
    );
    assert!(session.app().sidebar_loading(vpdf_ui::keys::PanelKind::Links));
    session.app().command("cancel")?;
    session.app().start_deferred();
    chores.run(&mut session, start);
    assert!(matches!(session.app().figures, crate::jobs::Figures::Idle));
    assert!(
      !session.app().sidebar_pending(vpdf_ui::keys::PanelKind::Annotations)
    );
    assert!(!session.app().sidebar_pending(vpdf_ui::keys::PanelKind::Links));
    session.app().want_figures();
    assert!(matches!(session.app().figures, crate::jobs::Figures::Running(_)));
    Ok(())
  }

  #[test]
  fn idle_sidebars_run_concurrently() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let other = root.path().join("other.pdf");
    crate::fixtures::pdf(&other, &["other"], "")?;
    let screen = session.app().screen;
    session.open(&other, true, screen)?;
    for tab in &mut session.tabs {
      for app in &mut tab.panes {
        app.complete = true;
      }
    }
    session.tabs[0].panes[0].complete = false;
    session.app().load_outline();
    let start = Instant::now();
    let mut chores = Chores::new(start);
    chores.run(&mut session, start);
    assert!(matches!(
      session.tabs[session.active].panes[0].figures,
      crate::jobs::Figures::Running(_)
    ));
    let mut loading = 0;
    for tab in &session.tabs {
      assert!(tab.panes[0].outline_pending());
      assert!(matches!(tab.panes[0].figures, crate::jobs::Figures::Running(_)));
      for kind in
        [vpdf_ui::keys::PanelKind::Annotations, vpdf_ui::keys::PanelKind::Links]
      {
        assert!(tab.panes[0].sidebar_pending(kind));
        loading += usize::from(tab.panes[0].sidebar_loading(kind));
      }
    }
    // Sidebar workers share one budget across panes.
    assert_eq!(loading, crate::sidebar::budget().min(4));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !session.app().figure_updates()? {
      assert!(Instant::now() < deadline);
      std::thread::sleep(Duration::from_millis(1));
    }
    chores.run(&mut session, start);
    while matches!(
      session.tabs[0].panes[0].figures,
      crate::jobs::Figures::Running(_)
    ) {
      assert!(Instant::now() < deadline);
      chores.run(&mut session, start);
      std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
      session.tabs[0].panes[0].figures,
      crate::jobs::Figures::Ready(_)
    ));
    Ok(())
  }

  #[test]
  fn expiry_deadlines_clear_once() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    session.app().command("vsplit")?;
    session.settle()?;
    session.record();
    session.flush()?;
    let start = Instant::now();
    for app in &mut session.tab().panes {
      app.reader.flash(app.reader.cursor, Span::Line, start);
      app.bar.observe(0.0, start);
      app.bar.observe(1.0, start);
    }
    let chores = Chores::new(start);
    assert_eq!(chores.wait(&mut session, start, false), Landing::DURATION);
    let before =
      (start + Landing::DURATION).checked_sub(Duration::from_nanos(1)).unwrap();
    assert!(!session.maintain(before));
    assert_eq!(
      chores.wait(&mut session, before, false),
      Duration::from_nanos(1)
    );
    assert!(session.maintain(start + Landing::DURATION));
    assert!(!session.maintain(start + Landing::DURATION));
    assert_eq!(
      chores.wait(&mut session, start + Landing::DURATION, false),
      Bar::SHOWN.checked_sub(Landing::DURATION).unwrap()
    );
    assert!(session.maintain(start + Bar::SHOWN));
    assert!(!session.maintain(start + Bar::SHOWN));
    assert_eq!(
      chores.wait(&mut session, start + Bar::SHOWN, false),
      WATCH.checked_sub(Bar::SHOWN).unwrap()
    );
    assert!(session.tab().panes.iter().all(|app| app.bar.deadline().is_none()
      && app.reader.landing().deadline().is_none()));
    Ok(())
  }

  #[test]
  fn cancelled_flash_does_not_wake() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let start = Instant::now();
    session.app().reader.flash(Position::default(), Span::Line, start);
    session.app().reader.clear_flash();
    let chores = Chores::new(start);
    assert_eq!(chores.wait(&mut session, start, false), WATCH);
    assert!(!session.maintain(start + Landing::DURATION));
    Ok(())
  }

  #[test]
  fn changes_schedule_recording_and_saves() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let store = root.path().join("state.json");
    session.record();
    session.flush()?;
    let start = Instant::now();
    let mut chores = Chores::new(start);
    session.app().command("2")?;
    chores.touch(start + Duration::from_millis(1));
    assert_eq!(chores.wait(&mut session, start, false), CHECK);
    assert!(!chores.run(
      &mut session,
      (start + CHECK).checked_sub(Duration::from_nanos(1)).unwrap()
    ));
    assert!(!session.unsaved);
    chores.run(&mut session, start + CHECK);
    assert!(session.unsaved);
    assert_eq!(
      crate::state::State::load(&store).recent[0].cursor.page,
      vpdf_model::text::PageId(0)
    );
    chores.run(&mut session, start + Duration::from_secs(5));
    assert!(session.unsaved);
    chores.run(&mut session, start + Duration::from_millis(5250));
    assert!(!session.unsaved);
    assert_eq!(
      crate::state::State::load(&store).recent[0].cursor.page,
      vpdf_model::text::PageId(1)
    );
    assert!(!chores.watch_due(start + Duration::from_millis(5250)));
    Ok(())
  }

  #[test]
  fn continuous_input_still_saves() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let start = Instant::now();
    let mut chores = Chores::new(start);
    for seconds in [5, 60, 61] {
      let now = start + Duration::from_secs(seconds);
      chores.touch(now);
      session.unsaved = true;
      chores.run(&mut session, now);
      assert_eq!(session.unsaved, seconds <= 60);
    }
    Ok(())
  }

  #[test]
  fn hidden_maintenance_keeps_animation_pending() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let start = Instant::now();
    session.app().reader.animate_to(100.0, start);
    session.sight = Sight::Hidden;
    let before = session.app().reader.scroll;
    session.maintain(start + Duration::from_millis(16));
    assert_eq!(session.app().reader.scroll, before);
    assert!(session.animating());
    session.sight = Sight::Focused;
    assert!(session.animate(start + Duration::from_millis(16)));
    assert!(session.app().reader.scroll > before);
    Ok(())
  }

  #[test]
  fn fallback_watch_reloads_after_settling() -> Result<()> {
    let root = tempfile::tempdir()?;
    let mut session = opened(root.path())?;
    let path = session.app().path.clone();
    crate::fixtures::pdf(&path, &["changed document"], "")?;
    let start = Instant::now();
    let mut chores = Chores::new(start);
    assert_eq!(chores.wait(&mut session, start, false), WATCH);
    assert!(!chores.watch_due(
      (start + WATCH).checked_sub(Duration::from_nanos(1)).unwrap()
    ));
    assert!(chores.watch_due(start + WATCH));
    chores.run(&mut session, start + WATCH);
    assert!(session.app().settling.is_some());
    assert!(
      chores.wait(&mut session, start + WATCH, false) <= crate::watch::SETTLE
    );
    assert!(
      !chores.run(
        &mut session,
        (start + WATCH + crate::watch::SETTLE)
          .checked_sub(Duration::from_nanos(1))
          .unwrap()
      )
    );
    assert!(chores.run(&mut session, start + WATCH + crate::watch::SETTLE));
    assert_eq!(session.app().doc.pages.len(), 1);
    assert!(session.app().images_stale);
    assert!(session.app().settling.is_none());
    Ok(())
  }
}
