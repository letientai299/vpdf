use super::Pane;
use super::{
  App, Cause, Drawable, Hover, Instant, Notice, Rect, Rehover, Result, Session,
  Sight, Tab, Write, terminal,
};
use crate::trace::Observation;
use vpdf_model::scene::Surface;
use vpdf_model::style::Role;
use vpdf_terminal::painter::Painter;
use vpdf_terminal::style::sgr;

impl Drawable for Session {
  /// Unfocused panes first, so the focused pane's prompt places the
  /// terminal cursor last.
  fn draw_frame(&mut self, out: &mut Vec<u8>) -> Result<()> {
    self.present_startup();
    self.prepare_frame();
    // Without a spot the pointer is elsewhere.
    if self.pointer.spot().is_some() {
      terminal::point(out, self.shape())?;
    }
    write!(out, "{}{}", terminal::frame_start(), sgr(Role::Reset))?;
    self.graphics.begin(out)?;
    let area = self.area();
    let (tab, graphics) = (&mut self.tabs[self.active], &mut self.graphics);
    let snap_regions = graphics.crop_quirk();
    let mut painter = Painter::new(out).with_graphics(graphics);
    self
      .settings
      .scene_theme(&mut painter, vpdf_model::theme::Colors::Terminal);
    let result = Self::scene_tab(
      tab,
      area,
      self.unmeasured.as_ref(),
      &mut painter,
      |app, painter| app.paint_pane(painter, snap_regions),
    );
    painter.finish()?;
    result?;
    graphics.end(out)?;
    if self.state.perf
      && let Some(screen) = self.screen
    {
      self.perf.draw(out, screen)?;
    }
    write!(out, "{}", terminal::frame_end())?;
    self.finish_frame();
    Ok(())
  }

  fn flushed(&mut self, at: Instant) {
    self.perf.frame(at);
  }

  fn observe(&self) -> Observation {
    let tab = &self.tabs[self.active];
    let mut seen = Observation::pane(&tab.panes[tab.focus], &self.graphics);
    seen.busy = tab.panes.iter().filter_map(Pane::document).any(App::busy);
    seen.outcome.tab = self.active + 1;
    seen.outcome.pane_count = tab.panes.len();
    seen.outcome.resources = self
      .tabs
      .iter()
      .enumerate()
      .flat_map(|(index, tab)| {
        tab.panes.iter().filter_map(Pane::document).map(move |app| (index, app))
      })
      .map(|(index, app)| {
        let tiles = self.graphics.owner_bytes(app.id);
        super::native::Resources::of(app, index == self.active, tiles)
      })
      .collect();
    seen
  }
}

impl Session {
  pub(super) fn scene_tab<'a, S: Surface<'a>>(
    tab: &mut Tab,
    area: Rect,
    notice: Option<&Notice>,
    scene: &mut S,
    mut pane: impl FnMut(&mut Pane, &mut S) -> Result<()>,
  ) -> Result<()> {
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        pane(&mut tab.panes[index], scene)?;
      }
    }
    tab.tree.scene_dividers(scene, area);
    pane(&mut tab.panes[tab.focus], scene)?;
    if let Some(notice) = notice {
      notice.scene(scene);
    }
    Ok(())
  }

  pub(super) fn prepare_frame(&mut self) {
    for app in self.tab().panes.iter_mut().filter_map(Pane::document_mut) {
      app.drop_stray_preview();
    }
    self.rehover();
  }

  pub(super) fn finish_frame(&mut self) {
    // Drawing can scroll thumbnails or rewrap rows.
    let drawn = self.pointer.target().map(Hover::shown);
    self.rehover();
    let moved = self.pointer.target().map(Hover::shown) != drawn;
    // One follow-up, so hover-drawn regions cannot loop.
    let following = self.rehover == Rehover::Following;
    self.rehover =
      if moved && !following { Rehover::Due } else { Rehover::Settled };
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
    let Some(screen) = self.screen else { return Ok(()) };
    let mut overlay = Vec::new();
    if self.perf.draw(&mut overlay, screen)? {
      let (start, end) = (terminal::frame_start(), terminal::frame_end());
      self.perf.exclude(start.len() + end.len());
      write!(out, "{start}")?;
      out.write_all(&overlay)?;
      write!(out, "{end}")?;
      out.flush()?;
    }
    Ok(())
  }

  /// After a frame reaches the terminal: starts work the shown panes
  /// deferred until their whole view was on screen.
  pub fn drawn(&mut self) {
    if let Some(tab) = self.tabs.get_mut(self.active) {
      tab
        .panes
        .iter_mut()
        .filter_map(Pane::document_mut)
        .for_each(App::start_deferred);
    }
  }

  /// Deletes every placement once tmux shows something else, which they
  /// would cover, and holds frames until tmux shows the pane again.
  pub fn hide(&mut self, out: &mut impl Write) -> Result<()> {
    self.sight = Sight::Hidden;
    write!(out, "{}", terminal::frame_end())?;
    self.release_pointer(out)?;

    self.graphics.clear(out)
  }

  /// Uploads image `id` again after the terminal evicted it.
  pub(super) fn missing(&mut self, id: u32) {
    if !self.graphics.missing(id) {
      return;
    }
    // Tiles no longer kept render again.
    for app in self.panes_mut() {
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
    for home in self
      .tabs
      .iter_mut()
      .flat_map(|t| &mut t.panes)
      .filter_map(Pane::recent_mut)
    {
      home.images_lost();
    }
    self.drop_pointer();
    for app in self.panes_mut() {
      app.tiles.invalidate();
      app.blank = false;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io;

  fn split_session(path: &std::path::Path, text_mode: bool) -> Result<Session> {
    let screen = crate::screen::cells(80, 24);
    let mut session = Session::new(None);
    session.open(path, true, screen)?;
    session.app().unwrap().command("vsplit")?;
    session.settle()?;
    session.focus(session.active, 0)?;
    for app in session.tabs[session.active]
      .panes
      .iter_mut()
      .filter_map(Pane::document_mut)
    {
      app.text_mode = text_mode;
      app.images_stale = true;
    }
    session.unmeasured = Some(Notice::new(24, 80, "Resize pending".into()));
    Ok(session)
  }

  fn legacy_tab(session: &mut Session, out: &mut impl Write) -> Result<()> {
    let area = session.area();
    let tab = &mut session.tabs[session.active];
    for index in 0..tab.panes.len() {
      if index != tab.focus {
        tab.panes[index]
          .document_mut()
          .unwrap()
          .draw_pane(&mut session.graphics, out)?;
      }
    }
    crate::presentation::draw(out, None, |scene| {
      tab.tree.scene_dividers(scene, area);
    })?;
    tab.panes[tab.focus]
      .document_mut()
      .unwrap()
      .draw_pane(&mut session.graphics, out)?;
    crate::presentation::draw(out, None, |scene| {
      session.unmeasured.as_ref().unwrap().scene(scene);
    })
  }

  #[test]
  fn inactive_panes_dim_with_focus() -> Result<()> {
    use vpdf_model::scene::{FillArea, Layer, Op, Recorder, Stage};

    let root = tempfile::tempdir()?;
    let path = root.path().join("panes.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    for text_mode in [false, true] {
      let mut session = split_session(&path, text_mode)?;
      for focus in [0, 1] {
        session.focus(session.active, focus)?;
        let area = session.area();
        let tab = &mut session.tabs[session.active];
        let mut recorder = Recorder::new(|_| false);
        Session::scene_tab(tab, area, None, &mut recorder, |app, scene| {
          app.scene_pane(scene, false)
        })?;
        let scene = recorder.into_scene();
        let veils: Vec<_> = scene
          .iter()
          .filter_map(|op| match op {
            Op::Fill(fill)
              if fill.color().resolve(
                vpdf_model::theme::Colors::Terminal,
                vpdf_model::theme::Opacity::default(),
              ) == {
                let [r, g, b] =
                  vpdf_model::theme::Colors::Terminal.palette().background;
                [r, g, b, 51]
              } =>
            {
              Some(fill)
            }
            _ => None,
          })
          .collect();
        assert_eq!(veils.len(), 1);
        let inactive = tab.panes[1 - focus].document().unwrap();
        assert_eq!(veils[0].layer(), Layer::AboveText);
        let stage = Stage { screen: inactive.screen, left: inactive.left };
        assert_eq!(veils[0].stage(), stage);
        assert_eq!(
          veils[0].area(),
          FillArea::Cells {
            column: 0,
            columns: inactive.screen.columns,
            rows: inactive.screen.rows - 1,
          }
        );
      }
    }
    Ok(())
  }

  #[test]
  fn configured_pane_opacity() -> Result<()> {
    use vpdf_model::scene::{Op, Recorder};

    let root = tempfile::tempdir()?;
    let path = root.path().join("panes.pdf");
    let config = root.path().join("config.toml");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    let mut session = split_session(&path, false)?;
    for (opacity, alpha) in [(1.0, 255), (0.2, 51), (0.1, 26), (0.0, 0)] {
      std::fs::write(
        &config,
        format!("[appearance.opacity]\ninactive = {opacity}"),
      )?;
      let loaded =
        crate::config::Options { config: Some(config.clone()) }.load();
      assert!(loaded.warning.is_none());
      let app = session.tabs[session.active].panes[1].document_mut().unwrap();
      app.settings = loaded.settings;
      let mut recorder = Recorder::new(|_| false);
      app.scene_pane(&mut recorder, false)?;
      let scene = recorder.into_scene();
      let veil = scene.iter().last().and_then(|op| match op {
        Op::Fill(fill) => Some(fill.color().resolve(
          vpdf_model::theme::Colors::Terminal,
          app.settings.opacity(),
        )),
        _ => None,
      });
      let [r, g, b] = vpdf_model::theme::Colors::Terminal.palette().background;
      assert_eq!(veil, (alpha > 0).then_some([r, g, b, alpha]));
    }
    Ok(())
  }

  #[test]
  fn failed_output_stops_panes() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("frame.pdf");
    crate::fixtures::pdf(&path, &["first", "second"], "")?;
    for text_mode in [false, true] {
      let mut reference = split_session(&path, text_mode)?;
      let mut full = Vec::new();
      legacy_tab(&mut reference, &mut full)?;
      let divider = full
        .windows("│".len())
        .position(|bytes| bytes == "│".as_bytes())
        .unwrap();
      for limit in [
        0,
        1,
        100,
        divider,
        divider + 1,
        full.len() / 2,
        full.len() - 1,
        full.len(),
      ] {
        let mut streamed = split_session(&path, text_mode)?;
        let mut legacy = split_session(&path, text_mode)?;
        let mut expected_bytes = vec![0; limit];
        let mut expected = io::Cursor::new(expected_bytes.as_mut_slice());
        let old = legacy_tab(&mut legacy, &mut expected);
        let mut actual_bytes = vec![0; limit];
        let mut actual = io::Cursor::new(actual_bytes.as_mut_slice());
        let area = streamed.area();
        let tab = &mut streamed.tabs[streamed.active];
        let snap_regions = streamed.graphics.crop_quirk();
        let mut painter =
          Painter::new(&mut actual).with_graphics(&mut streamed.graphics);
        let new = Session::scene_tab(
          tab,
          area,
          streamed.unmeasured.as_ref(),
          &mut painter,
          |app, painter| app.paint_pane(painter, snap_regions),
        );
        let new = painter.finish().map_err(anyhow::Error::from).and(new);
        let kind = |result: Result<()>| {
          result
            .err()
            .map(|error| error.downcast::<io::Error>().unwrap().kind())
        };
        let wanted = (limit < full.len()).then_some(io::ErrorKind::WriteZero);
        assert_eq!(kind(new), wanted, "limit {limit}");
        assert_eq!(kind(old), wanted, "limit {limit}");
        assert_eq!(actual.into_inner(), expected.into_inner(), "limit {limit}");
        if limit == 0 {
          let tab = &streamed.tabs[streamed.active];
          assert!(
            tab.panes.iter().all(|app| app
              .document()
              .unwrap()
              .tiles
              .wanted()
              .is_empty())
          );
          assert!(tab.panes[tab.focus].document().unwrap().images_stale);
          assert!(!tab.panes[1].document().unwrap().images_stale);
        }
        for (left, right) in streamed.tabs[streamed.active]
          .panes
          .iter()
          .zip(&legacy.tabs[legacy.active].panes)
        {
          assert_eq!(
            left.document().unwrap().blank,
            right.document().unwrap().blank,
            "limit {limit}"
          );
          assert_eq!(
            left.document().unwrap().complete,
            right.document().unwrap().complete,
            "limit {limit}"
          );
          assert_eq!(
            left.document().unwrap().images_stale,
            right.document().unwrap().images_stale,
            "limit {limit}"
          );
          assert_eq!(
            left.document().unwrap().tiles.wanted(),
            right.document().unwrap().tiles.wanted(),
            "limit {limit}"
          );
        }
      }
    }
    Ok(())
  }
}
