use super::{
  App, Dropped, Hit, Hover, Instant, MouseButton, MouseEvent, MouseEventKind,
  Pixel, Region, Rehover, Result, Route, Session, Shape, Step, Target, Write,
  answer, terminal,
};

/// Trackpad input carried on a wheel report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
  Turn(vpdf_ui::input::wheel::Precise),
  Pinch(f32),
}

impl Session {
  /// One terminal mouse report; whether it needs a frame.
  pub fn mouse_report(
    &mut self,
    report: vpdf_model::input::Mouse,
  ) -> Result<bool> {
    self.mouse_report_at(report, Instant::now())
  }

  /// A report in screen cells, clamped to the grid.
  pub(super) fn screen_mouse(
    &self,
    report: vpdf_model::input::Mouse,
  ) -> Option<MouseEvent> {
    let screen = self.screen?;
    let mut mouse = report.cells(screen.cell())?;
    (mouse.column, mouse.row) = grid_cell(screen, report);
    Some(mouse)
  }

  /// One mouse report that arrived at `now`; whether it needs a frame.
  pub fn mouse_report_at(
    &mut self,
    report: vpdf_model::input::Mouse,
    now: Instant,
  ) -> Result<bool> {
    self.report_at(report, now, None)
  }

  /// A trackpad turn or pinch at a wheel report; whether it needs a
  /// frame.
  pub(super) fn gesture_report(
    &mut self,
    report: vpdf_model::input::Mouse,
    trackpad: Gesture,
  ) -> Result<bool> {
    self.report_at(report, Instant::now(), Some(trackpad))
  }

  fn report_at(
    &mut self,
    report: vpdf_model::input::Mouse,
    now: Instant,
    trackpad: Option<Gesture>,
  ) -> Result<bool> {
    // Reports in flight after `:mouse off`.
    if !terminal::mouse() {
      return Ok(false);
    }
    let Some(screen) = self.screen else { return Ok(false) };
    let (column, row) = grid_cell(screen, report);
    if let Some(forward) = report.history() {
      return self.history_button(column, row, forward);
    }
    let Some(mouse) = self.screen_mouse(report) else {
      return Ok(false);
    };
    let pixel =
      report.pixel().map(|(x, y)| Pixel { x, y, cell: screen.cell() });
    self.pointed(mouse, pixel, now, trackpad)
  }

  /// `Ctrl-o` or `Ctrl-i` where pointed.
  fn history_button(
    &mut self,
    column: u16,
    row: u16,
    forward: bool,
  ) -> Result<bool> {
    let modifiers = vpdf_model::input::KeyModifiers::NONE;
    let mouse =
      MouseEvent { kind: MouseEventKind::Moved, column, row, modifiers };
    // One button at a time.
    if self.pointer.holding() || self.on_completion(mouse) {
      return Ok(false);
    }
    if let Ok(app) = self.app() {
      app.calm_hover();
    }
    if self.app().map_or(Ok(false), App::dismiss_click)? {
      self.pointer.release();
      return Ok(true);
    }
    let Some((index, _)) = self.under(mouse) else { return Ok(false) };
    if let Ok(app) = self.app() {
      app.interrupt();
    }
    self.pointer.release();
    self.focus(self.active, index)?;
    let history =
      vpdf_ui::keys::Input::plain(vpdf_ui::keys::Action::History(forward));
    if let Ok(app) = self.app() {
      app.action(history)?;
    }
    Ok(true)
  }

  /// A report in cells; whether it needs a frame.
  pub fn mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
    self.pointed(mouse, None, Instant::now(), None)
  }

  fn stationary_scroll(
    &self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> bool {
    !self.pointer.holding()
      && self.resizing.is_none()
      && (self.hover_animating()
        || self.pointer.turn_end().is_some_and(|end| now < end))
      && self.pointer.spot().is_some_and(|(last, at)| {
        (last.column, last.row, last.modifiers, at)
          == (mouse.column, mouse.row, mouse.modifiers, pixel)
      })
  }

  fn pointer_moved(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> bool {
    if self.stationary_scroll(mouse, pixel, now) {
      return false;
    }
    let before = self.pointer.target();
    let hover = self.hit(mouse, pixel);
    let seen = before.map(Hover::shown) != hover.map(Hover::shown);
    let onto = |hover: Option<Hover>| hover.map(|h| (h.pane, h.hit));
    let arrived = onto(before) != onto(hover);
    let changed = self.pointer.hover(mouse, pixel, hover);
    if arrived {
      self.moved_onto();
    }
    if changed {
      self.show_hover();
    } else if let Some(app) = self.pointed_pane() {
      app.rest_again();
    }
    let left = self.leave_check();
    // Bare motion: a release got lost.
    let dropped = self.app().is_ok_and(App::drop_held);
    let resized = self.resizing.take().is_some();
    dropped || resized || seen || left
  }

  fn pointed(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
    trackpad: Option<Gesture>,
  ) -> Result<bool> {
    if mouse.kind == MouseEventKind::Moved {
      return Ok(self.pointer_moved(mouse, pixel, now));
    }
    if let Some(changed) = self.edge_event(mouse, pixel, now)? {
      return Ok(changed);
    }
    let wheel = vpdf_ui::pointer::Wheel::of(mouse.kind).is_some();
    if wheel && self.focused().prompt().is_some() {
      return Ok(false);
    }
    // Presses and releases end rests in every pane.
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
      for app in
        self.tab().panes.iter_mut().filter_map(super::Pane::document_mut)
      {
        app.calm_hover();
      }
    }
    // A double click splits once.
    if mouse.kind == MouseEventKind::Down(MouseButton::Middle)
      && self.pointer.repeats(mouse, pixel, now)
    {
      return Ok(false);
    }
    // A double click keeps a note open.
    if mouse.kind == MouseEventKind::Down(MouseButton::Left)
      && self.app().is_ok_and(|app| app.on_anchor(mouse, pixel))
      && self.pointer.repeats(mouse, pixel, now)
    {
      return Ok(false);
    }
    if matches!(mouse.kind, MouseEventKind::Down(_))
      && let Some(drawn) = self.press_first(mouse, pixel, now)?
    {
      return Ok(drawn);
    }
    // Snapshot before disarm hides clue rows.
    let (panes, focused) = self.tabs[self.active].pointer_panes();
    let route = self.pointer.route(&panes, focused, mouse, pixel, now);
    let gesture = wheel && self.pointer.turn(mouse, pixel, now);
    let interrupts = vpdf_ui::pointer::interrupts(mouse.kind)
      && !(wheel
        && route.as_ref().is_some_and(|route| route.region == Region::Overlay));
    let armed = if let Ok(app) = self.app() {
      let armed = interrupts
        && (!app.keys.pending().is_empty()
          || app.overlays.clues().shown().is_some()
          || app.reader.operator.is_some());
      if interrupts {
        app.disarm();
      }
      armed
    } else {
      false
    };
    let Some(route) = route else { return Ok(armed) };
    let Some(index) = self.tab().index(route.pane) else { return Ok(armed) };
    if route.focus {
      // Focusing releases captures; keep this one.
      let active = self.active;
      self.keeping_pointer(|session| session.focus(active, index))?;
    }
    if self.tab().panes[index].is_recent() {
      return self.recent_pointer(index, mouse, pixel, trackpad);
    }
    let app = self.tab().panes[index]
      .document_mut()
      .ok_or_else(|| anyhow::anyhow!("No document"))?;
    // Per-pixel drags; edges scroll.
    let reading = |app: &App| {
      let menu =
        app.overlays.menu().map(vpdf_ui::context::ContextMenu::selected);
      let held = (app.grabbing(), app.scrub_row());
      let view = (app.view_key(), app.layout.pan().to_bits(), held);
      (
        app.reader.cursor,
        app.reader.selection,
        view,
        app.gesture.snapshot(),
        menu,
      )
    };
    let before =
      matches!(mouse.kind, MouseEventKind::Drag(_)).then(|| reading(app));
    if !gesture {
      app.wheel.cancel();
    }
    // Turns past an end move nothing.
    let view = wheel.then(|| app.wheel_view());
    // Like keys, presses clear old notes.
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      app.clear_notes();
    }
    match trackpad {
      None => app.mouse(route)?,
      Some(Gesture::Turn(precise)) => app.precise_wheel(route, precise)?,
      Some(Gesture::Pinch(factor)) => app.pinch(route, factor)?,
    }
    let moved = view.is_none_or(|view| view != app.wheel_view());
    let read = before.is_none_or(|before| before != reading(app));
    let changed = read && moved;
    Ok(armed || changed)
  }

  fn recent_pointer(
    &mut self,
    index: usize,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    trackpad: Option<Gesture>,
  ) -> Result<bool> {
    let turns = match trackpad {
      Some(Gesture::Pinch(_)) => return Ok(false),
      Some(Gesture::Turn(precise)) => precise.whole,
      None => 1,
    };
    if turns == 0 {
      return Ok(false);
    }
    if matches!(mouse.kind, MouseEventKind::Drag(_))
      && self.tab().panes[index]
        .recent()
        .and_then(|home| home.view.shelf())
        .is_none_or(|shelf| !shelf.dragging())
    {
      return Ok(false);
    }
    let mut report = vpdf_model::input::Mouse::from(mouse);
    if let Some(pixel) = pixel {
      report.at = vpdf_model::input::Spot::Pixel { x: pixel.x, y: pixel.y };
    }
    let terminal = matches!(self.frontend, crate::raster::Frontend::Terminal);
    for _ in 0..turns {
      let (tab, history) = (&mut self.tabs[self.active], &mut self.history);
      let Some(home) = tab.panes[index].recent_mut() else { return Ok(false) };
      let outcome = home.handle(
        vpdf_model::input::Event::Mouse(report),
        history,
        terminal,
      )?;
      self.record_recent_layout();
      self.recent_outcome(outcome)?;
    }
    Ok(true)
  }

  /// Presses taken before panes; whether drawn.
  fn press_first(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<Option<bool>> {
    // One button at a time.
    if self.pointer.busy(mouse) {
      return Ok(Some(false));
    }
    // A new left press: release lost.
    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
      self.resizing = None;
    }
    self.press_hover_pane(mouse)?;
    if self.app().map_or(Ok(false), |app| -> Result<bool> {
      Ok(app.item_press(mouse)? || app.context_press(mouse))
    })? {
      self.pointer.release();
      return Ok(Some(true));
    }
    // Blank completion rows stay; clue rows close.
    if self.on_completion(mouse) {
      return Ok(Some(false));
    }
    if self.app().map_or(Ok(false), |app| app.preview_press(mouse))? {
      if mouse.kind == MouseEventKind::Down(MouseButton::Middle) {
        // Its run drops a double click's repeat.
        let (panes, focused) = self.tab().pointer_panes();
        self.pointer.route(&panes, focused, mouse, pixel, now);
      } else {
        self.pointer.release();
      }
      return Ok(Some(true));
    }
    if self.app().map_or(Ok(false), App::dismiss_click)? {
      self.pointer.release();
      return Ok(Some(true));
    }
    if self.status_press(mouse)? {
      return Ok(Some(true));
    }
    if let Some(edge) = self.edge_at(mouse.column, mouse.row) {
      // Keys and notes end, as on blanks.
      if let Ok(app) = self.app() {
        app.interrupt();
      }
      // Boundaries never focus; left presses drag.
      if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return Ok(Some(true));
      }
      return self.edge_press(edge, mouse, pixel, now).map(Some);
    }
    Ok(None)
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
    let typing = tab.is_some_and(|tab| tab.panes[tab.focus].prompt().is_some());
    let Some(drop) = self.drop_step(body, typing, out)? else {
      return Ok(false);
    };
    if self.confirming_removal() {
      answer(out, &drop.finish(false))?;
      return Ok(false);
    }
    let done =
      crate::dnd::target(&drop, typing).and_then(|target| self.dropped(target));
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
        answer(out, vpdf_terminal::dnd::CANCEL)?;
        anyhow::bail!("Drop failed: {error}")
      }
    }
  }

  /// File drops open tabs; text edits prompts.
  pub(super) fn dropped(&mut self, target: Target) -> Result<()> {
    let path = match target {
      Target::Text(text) => {
        return self
          .interact(super::interaction::Interaction::Paste(text))
          .map(|_| ());
      }
      Target::Pdf(path) => path,
    };
    let screen = self.screen.unwrap_or(self.focused().screen);
    self.open_reference(&path, !self.focused().is_recent(), screen)
  }

  /// Whether a press hits completions.
  fn on_completion(&mut self, mouse: MouseEvent) -> bool {
    self.focused().prompt().is_some()
      && self.focused_in(Region::Overlay, mouse).is_some()
  }

  /// `mouse` routed in the focused pane's `region`.
  fn focused_in(&mut self, region: Region, mouse: MouseEvent) -> Option<Route> {
    let pane = self.focused().pointer_pane();
    let local = vpdf_ui::pointer::route(&[pane], pane.id, None, mouse);
    local.filter(|route| route.region == region)
  }

  /// Acts on a pressed status item.
  fn status_press(&mut self, mouse: MouseEvent) -> Result<bool> {
    let MouseEventKind::Down(button) = mouse.kind else { return Ok(false) };
    let Some((index, route)) = self.under(mouse) else { return Ok(false) };
    if route.region != Region::Status {
      return Ok(false);
    }
    let pane = &self.tabs[self.active].panes[index];
    let item = pane.chrome_at(route.mouse.column, route.mouse.row);
    let Some(item) = item else { return Ok(false) };
    self.focus(self.active, index)?;
    if self.focused().is_recent() {
      match item.press(button) {
        Some(vpdf_ui::chrome::Press::Tab(index)) => self.activate(index)?,
        Some(vpdf_ui::chrome::Press::CloseTab(index)) => {
          self.close_tab(index)?;
        }
        _ => return Ok(false),
      }
    } else {
      let app = self.app()?;
      app.interrupt();
      app.chrome_press(item, button)?;
    }
    self.pointer.release();
    Ok(true)
  }

  /// Pane index and route under `mouse`.
  pub(super) fn under(&mut self, mouse: MouseEvent) -> Option<(usize, Route)> {
    let (panes, focused) = self.tabs.get(self.active)?.pointer_panes();
    let route = vpdf_ui::pointer::route(&panes, focused, None, mouse)?;
    Some((self.tab().index(route.pane)?, route))
  }

  /// The pointer left the window; whether a hover ended.
  pub fn pointer_left(&mut self) -> bool {
    let changed = self.pointer.leave();
    self.show_hover();
    self.leave_check() || changed
  }

  /// Closes hover previews the pointer left.
  fn leave_check(&mut self) -> bool {
    let spot = self.pointer.spot().map(|(mouse, _)| mouse);
    let Some(tab) = self.tabs.get_mut(self.active) else { return false };
    let mut left = false;
    for app in tab.panes.iter_mut().filter_map(super::Pane::document_mut) {
      if app.hover_peek().is_none() {
        continue;
      }
      let pane = app.pointer_pane();
      let route = spot.and_then(|mouse| {
        vpdf_ui::pointer::route(&[pane], pane.id, None, mouse)
      });
      let at = route.map(|route| (route.mouse.column, route.mouse.row));
      left |= app.hover_leave(at);
    }
    left
  }

  /// Inside another pane's preview focuses it; outside closes it.
  fn press_hover_pane(&mut self, mouse: MouseEvent) -> Result<()> {
    let tab = self.tab();
    let shows = tab.panes.iter().position(|pane| {
      pane.document().is_some_and(|app| app.hover_peek().is_some())
    });
    let Some(index) = shows.filter(|index| *index != tab.focus) else {
      return Ok(());
    };
    let Some(app) = tab.panes[index].document_mut() else { return Ok(()) };
    if app.in_preview(mouse) {
      return self.focus(self.active, index);
    }
    app.drop_hover_preview();
    Ok(())
  }

  /// Opens a due hover preview, if any.
  pub fn hover_preview(&mut self, now: Instant) -> bool {
    !self.pointer.holding()
      && self.resting().is_some_and(|app| app.hover_preview(now))
  }

  /// The pane under the pointer.
  fn pointed_pane(&mut self) -> Option<&mut App> {
    let pane = self.pointer.target()?.pane;
    let tab = self.tabs.get_mut(self.active)?;
    tab
      .panes
      .iter_mut()
      .find(|app| app.id == pane)
      .and_then(super::Pane::document_mut)
  }

  /// The pointed pane, when free to preview.
  pub(super) fn resting(&mut self) -> Option<&mut App> {
    let tab = self.tabs.get(self.active)?;
    // Keyboard overlays hold rests everywhere.
    let owner = tab.panes[tab.focus].document()?.input_owner();
    let free = !owner.is_overlay();
    // Document previews share one picture.
    let pictured = |app: &App| app.overlays.preview().is_some();
    if !free || tab.panes.iter().filter_map(super::Pane::document).any(pictured)
    {
      return None;
    }
    self.pointed_pane()
  }

  /// Retests the last spot; the view changed.
  pub(super) fn rehover(&mut self) {
    if let Some((mouse, pixel)) = self.pointer.spot() {
      let hover = self.deferred(mouse).or_else(|| self.hit(mouse, pixel));
      self.pointer.refresh(hover);
    }
    self.show_hover();
    self.leave_check();
  }

  fn hover_animating(&self) -> bool {
    let pane = self.pointer.target().map(|hover| hover.pane);
    self.tabs.get(self.active).is_some_and(|tab| {
      tab
        .panes
        .iter()
        .filter_map(super::Pane::document)
        .any(|app| Some(app.id) == pane && app.reader.animation().active())
    })
  }

  /// Empty hover while scrolling defers page extraction.
  fn deferred(&mut self, mouse: MouseEvent) -> Option<Hover> {
    self.skimmed = false;
    if self.edge_hover(mouse).is_some() {
      return None;
    }
    let (index, Route { pane, region, .. }) = self.under(mouse)?;
    if region != Region::Page {
      return None;
    }
    let turning = self.tabs[self.active].panes[index]
      .document()?
      .reader
      .animation()
      .active()
      || self.pointer.turn_end().is_some_and(|end| Instant::now() < end);
    if !turning {
      return None;
    }
    self.skimmed = true;
    Some(Hover { pane, region, hit: Hit::Nothing, beneath: None })
  }

  /// Gives each pane its hover.
  fn show_hover(&mut self) {
    let hover = self.pointer.target();
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    for pane in &mut tab.panes {
      let pointed = hover.filter(|hover| hover.pane == pane.id);
      match &mut pane.body {
        super::Body::Document(app) => app.point(pointed),
        super::Body::Recent(home) => {
          home.view.point(pointed.map(|hover| hover.hit));
        }
      }
    }
  }

  /// Motion picks a menu row or item.
  fn moved_onto(&mut self) {
    let Some(hover) = self.pointer.target() else { return };
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    let app = tab
      .panes
      .iter_mut()
      .find(|app| app.id == hover.pane)
      .and_then(super::Pane::document_mut);
    if let Some(app) = app {
      app.moved_onto(hover.hit);
    }
  }

  /// A held gesture's shape, else the hover's.
  pub(super) fn shape(&self) -> Shape {
    if let Some(edge) = self.resizing {
      return edge.hit().shape();
    }
    let tab = &self.tabs[self.active];
    if tab.panes[tab.focus].document().is_some_and(App::grabbing) {
      return Shape::Grabbing;
    }
    if self.tabs[self.active]
      .panes
      .iter()
      .filter_map(super::Pane::document)
      .any(App::sweeping)
    {
      return Shape::Crosshair;
    }
    self.pointer.target().map_or_default(|hover| hover.hit).shape()
  }

  /// What the pointer rests on at `mouse`.
  pub(super) fn hit(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
  ) -> Option<Hover> {
    if let Some(hover) = self.edge_hover(mouse) {
      return Some(hover);
    }
    let (index, route) = self.under(mouse)?;
    let Route { pane, region, mouse, .. } = route;
    let (column, row) = (mouse.column, mouse.row);
    if let Some(home) = self.tab().panes[index].recent() {
      let hit = home.view.hit(column, row);
      return (hit != Hit::Nothing).then_some(Hover {
        pane,
        region,
        hit,
        beneath: None,
      });
    }
    let app = self.tab().panes[index].document_mut()?;
    // Hover is advisory; failures find nothing.
    let hit = app.hovered(region, column, row, pixel).unwrap_or_default();
    let below = if region.popup() {
      app.hovered(Region::Page, column, row, pixel)
    } else {
      Ok(hit)
    };
    let beneath = match below {
      Ok(Hit::Link(page, index)) => Some((page, index)),
      _ => None,
    };
    Some(Hover { pane, region, hit, beneath })
  }

  /// The pointer may rest elsewhere now.
  pub(super) fn release_pointer(&mut self, out: &mut impl Write) -> Result<()> {
    self.drop_pointer();
    terminal::hand_over(out)?;
    // Out before the slow `tmux display`.
    out.flush()?;
    Ok(())
  }

  /// Forgets the pointer: hover, held presses, and gestures.
  pub(super) fn drop_pointer(&mut self) {
    self.pointer.leave();
    self.resizing = None;
    // Its release may never come.
    self.pointer.release();
    for app in self.panes_mut() {
      app.drop_held();
    }
  }

  /// Retests deferred page hover after scrolling.
  pub fn hover_settled(&mut self, now: Instant) -> bool {
    let due = self.hover_settle_due().is_some_and(|due| now >= due);
    if due {
      self.skimmed = false;
    }
    due
  }

  /// When the deferred page hover is due.
  pub fn hover_settle_due(&self) -> Option<Instant> {
    (self.skimmed && !self.hover_animating())
      .then(|| self.pointer.turn_end())
      .flatten()
  }

  /// Steps the held edge drag; whether drawn.
  pub fn autoscroll(&mut self, now: Instant) -> bool {
    let held = self.pointer.held_pane();
    let mut changed = false;
    for app in self.panes_mut() {
      // Lost releases stop it too.
      if held != Some(app.id) {
        app.autoscroll.cancel();
        continue;
      }
      match app.autoscroll_step(now) {
        Ok(moved) => changed |= moved,
        Err(error) => {
          app.autoscroll.cancel();
          app.error = Some(error.to_string());
          changed = true;
        }
      }
    }
    changed
  }

  /// When the held edge drag next scrolls.
  pub fn autoscroll_due(&self) -> Option<Instant> {
    let panes = &self.tabs.get(self.active)?.panes;
    panes
      .iter()
      .filter_map(super::Pane::document)
      .find_map(|app| app.autoscroll.deadline())
  }
}

/// The report's cell; padding past the grid reports cells beyond it.
fn grid_cell(
  screen: vpdf_ui::layout::Screen,
  report: vpdf_model::input::Mouse,
) -> (u16, u16) {
  let (column, row) = report.cell(screen.cell());
  (
    column.min(screen.columns.saturating_sub(1)),
    row.min(screen.rows.saturating_sub(1)),
  )
}
