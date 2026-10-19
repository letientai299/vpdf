use super::{
  App, Cause, Drawable, Duration, Hover, ImageState, ImageWrite, Instant,
  Rehover, Result, Session, Sight, Write, terminal,
};
use crate::trace::Observation;

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
    self.graphics.begin(out)?;
    let area = self.area();
    let unmeasured = self.unmeasured.clone();
    let (tab, graphics) = (&mut self.tabs[self.active], &mut self.graphics);
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        tab.panes[index].draw_pane(graphics, out)?;
      }
    }
    tab.tree.draw_dividers(out, area)?;
    let focus = tab.focus;
    tab.panes[focus].draw_pane(graphics, out)?;
    if let Some(unmeasured) = unmeasured {
      unmeasured.draw(out)?;
    }
    graphics.end(out)?;
    if self.state.perf
      && let Some(screen) = self.screen
    {
      self.perf.view.draw(out, screen, false)?;
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

  fn observe(&self) -> Observation {
    let tab = &self.tabs[self.active];
    Observation::of(&tab.panes[tab.focus], &self.graphics)
  }
}

impl Session {
  /// One frame of the focused pane alone, as tests drive it.
  #[doc(hidden)]
  pub fn draw_focused(&mut self, out: &mut impl Write) -> Result<()> {
    let tab = &mut self.tabs[self.active];
    tab.panes[tab.focus].draw_alone(&mut self.graphics, out)
  }

  pub(super) fn perf_tick(&mut self, out: &mut impl Write) -> Result<()> {
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
      if self.perf.view.draw(&mut buffer, screen, false)? {
        write!(buffer, "{}", terminal::frame_end())?;
        out.write_all(&buffer)?;
        out.flush()?;
      }
    }
    Ok(())
  }

  /// After a frame reaches the terminal: starts work the shown panes
  /// deferred until their whole view was on screen.
  pub fn drawn(&mut self) {
    if let Some(tab) = self.tabs.get_mut(self.active) {
      tab.panes.iter_mut().for_each(App::start_deferred);
    }
  }

  /// Deletes every placement once tmux shows something else, which they
  /// would cover, and holds frames until tmux shows the pane again.
  pub fn hide(&mut self, out: &mut impl Write) -> Result<()> {
    self.sight = Sight::Hidden;
    write!(out, "{}", terminal::frame_end())?;
    self.release_pointer(out)?;
    if self.tabs.is_empty() {
      return self.home.as_mut().map_or(Ok(()), |home| home.release(out));
    }
    self.graphics.clear(out)
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

  pub(super) fn image_status(&mut self, body: &str) -> bool {
    let Some(result) = vpdf_terminal::clipboard::image_status(body) else {
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

  pub(super) fn image_wait(&self) -> Option<Duration> {
    match &self.image_write {
      Some(ImageWrite { state: ImageState::Waiting(until), .. }) => {
        Some(until.saturating_duration_since(Instant::now()))
      }
      _ => None,
    }
  }

  pub(super) fn image_timeout(&mut self, now: Instant) -> bool {
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

  pub(super) fn image_updates(&mut self, out: &mut impl Write) -> bool {
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
      match vpdf_terminal::caps::get().and_then(|caps| caps.clipboard) {
        Some(vpdf_terminal::caps::Support::Answered) => {}
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
      let result = vpdf_terminal::clipboard::image_remote(
        out,
        &transfer.png,
        terminal::tmux(),
      )
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

  /// Uploads image `id` again after the terminal evicted it.
  pub(super) fn missing(&mut self, id: u32) {
    if !self.graphics.missing(id) {
      return;
    }
    // Tiles no longer kept render again.
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.tiles.invalidate();
    }
  }

  /// Drops cached images after another program had the screen, which
  /// also took the focus events tmux sent meanwhile; asks tmux anew,
  /// since it sends no focus state when asked for focus events.
  pub(super) fn images_lost(&mut self, out: &mut impl Write) -> Result<bool> {
    if !terminal::images_lost() {
      return Ok(false);
    }
    self.lose_images(out)?;
    if terminal::focus_reports() {
      self.look(Cause::Start, out)?;
    }
    Ok(true)
  }

  pub(super) fn lose_images(&mut self, out: &mut impl Write) -> Result<()> {
    self.perf.view.invalidate();
    vpdf_terminal::graphics::release(out)?;
    self.graphics.lost();
    if let Some(home) = &mut self.home {
      home.images_lost();
    }
    self.drop_pointer();
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.tiles.invalidate();
      app.blank = false;
    }
    Ok(())
  }
}
