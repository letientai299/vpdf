use super::{
  App, Cause, Drawable, Hover, Instant, Rehover, Result, Session, Sight, Write,
  terminal,
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
    write!(out, "{}{}", terminal::frame_start(), vpdf_ui::theme::RESET)?;
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
      self.perf.draw(out, screen)?;
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
        self.perf.exclude(buffer.len());
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
