use super::{
  App, Dropped, Event, Hit, Home, Hover, Instant, Landing, MouseButton,
  MouseEvent, MouseEventKind, Path, Pixel, Region, Rehover, Result, Route,
  Session, Shape, Step, Target, Write, answer, terminal,
};

impl Session {
  /// One terminal mouse report; whether it needs a frame.
  pub fn mouse_report(
    &mut self,
    report: vpdf_terminal::decode::Mouse,
  ) -> Result<bool> {
    self.mouse_report_at(report, Instant::now())
  }

  /// One mouse report that arrived at `now`; whether it needs a frame.
  pub fn mouse_report_at(
    &mut self,
    report: vpdf_terminal::decode::Mouse,
    now: Instant,
  ) -> Result<bool> {
    // Reports in flight after `:mouse off`.
    if !terminal::mouse() {
      return Ok(false);
    }
    let Some(screen) = self.screen else { return Ok(false) };
    // Padding past the grid reports cells beyond it.
    let (column, row) = report.cell(screen.cell());
    let column = column.min(screen.columns.saturating_sub(1));
    let row = row.min(screen.rows.saturating_sub(1));
    if let Some(forward) = report.history() {
      return self.history_button(column, row, forward);
    }
    let Some(mut mouse) = report.cells(screen.cell()) else {
      return Ok(false);
    };
    (mouse.column, mouse.row) = (column, row);
    let pixel =
      report.pixel().map(|(x, y)| Pixel { x, y, cell: screen.cell() });
    self.pointed(mouse, pixel, now)
  }

  /// `Ctrl-o` or `Ctrl-i` where pointed.
  fn history_button(
    &mut self,
    column: u16,
    row: u16,
    forward: bool,
  ) -> Result<bool> {
    let modifiers = crossterm::event::KeyModifiers::NONE;
    let mouse =
      MouseEvent { kind: MouseEventKind::Moved, column, row, modifiers };
    // One button at a time.
    if self.pointer.holding() || self.on_completion(mouse) {
      return Ok(false);
    }
    self.app().calm_hover();
    if self.app().dismiss_click()? {
      self.pointer.release();
      return Ok(true);
    }
    let Some((index, _)) = self.under(mouse) else { return Ok(false) };
    self.app().interrupt();
    self.pointer.release();
    self.focus(self.active, index)?;
    let history =
      vpdf_ui::keys::Input::plain(vpdf_ui::keys::Action::History(forward));
    self.app().action(history)?;
    Ok(true)
  }

  /// A report in cells; whether it needs a frame.
  pub fn mouse(&mut self, mouse: MouseEvent) -> Result<bool> {
    self.pointed(mouse, None, Instant::now())
  }

  fn pointed(
    &mut self,
    mouse: MouseEvent,
    pixel: Option<Pixel>,
    now: Instant,
  ) -> Result<bool> {
    if mouse.kind == MouseEventKind::Moved {
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
      let app = self.app();
      let dropped = app.drop_held();
      let resized = self.resizing.take().is_some();
      return Ok(dropped || resized || seen || left);
    }
    if let Some(changed) = self.edge_event(mouse, pixel, now)? {
      return Ok(changed);
    }
    let wheel = vpdf_ui::pointer::Wheel::of(mouse.kind).is_some();
    if wheel && self.app().overlays.prompt().is_some() {
      return Ok(false);
    }
    // Presses and releases end rests in every pane.
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
      for app in &mut self.tab().panes {
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
      && self.app().on_anchor(mouse, pixel)
      && self.pointer.repeats(mouse, pixel, now)
    {
      return Ok(false);
    }
    if matches!(mouse.kind, MouseEventKind::Down(_))
      && let Some(drawn) = self.press_first(mouse, pixel, now)?
    {
      return Ok(drawn);
    }
    let tab = &self.tabs[self.active];
    // Snapshot before disarm hides clue rows.
    let panes: Vec<_> = tab.panes.iter().map(App::pointer_pane).collect();
    let focused = tab.panes[tab.focus].id;
    let route = self.pointer.route(&panes, focused, mouse, pixel, now);
    let gesture = wheel && self.pointer.turn(mouse, pixel, now);
    let interrupts = vpdf_ui::pointer::interrupts(mouse.kind)
      && !(wheel
        && route.as_ref().is_some_and(|route| route.region == Region::Overlay));
    let app = self.app();
    // Disarming changes the status line and hides clue rows.
    let armed = interrupts
      && (!app.keys.pending().is_empty()
        || app.overlays.clues().shown().is_some()
        || app.reader.operator.is_some());
    if interrupts {
      app.disarm();
    }
    let Some(route) = route else { return Ok(armed) };
    let Some(index) = self.tab().index(route.pane) else { return Ok(armed) };
    if route.focus {
      // Focusing releases captures; keep this one.
      let active = self.active;
      self.keeping_pointer(|session| session.focus(active, index))?;
    }
    let app = &mut self.tab().panes[index];
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
    app.mouse(route)?;
    let moved = view.is_none_or(|view| view != app.wheel_view());
    let read = before.is_none_or(|before| before != reading(app));
    let changed = read && moved;
    Ok(armed || changed)
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
    let app = self.app();
    if app.item_press(mouse)? || app.context_press(mouse) {
      self.pointer.release();
      return Ok(Some(true));
    }
    // Blank completion rows stay; clue rows close.
    if self.on_completion(mouse) {
      return Ok(Some(false));
    }
    if self.app().preview_press(mouse)? {
      if mouse.kind == MouseEventKind::Down(MouseButton::Middle) {
        // Its run drops a double click's repeat.
        let tab = self.tab();
        let panes: Vec<_> = tab.panes.iter().map(App::pointer_pane).collect();
        let focused = tab.panes[tab.focus].id;
        self.pointer.route(&panes, focused, mouse, pixel, now);
      } else {
        self.pointer.release();
      }
      return Ok(Some(true));
    }
    if self.app().dismiss_click()? {
      self.pointer.release();
      return Ok(Some(true));
    }
    if self.status_press(mouse)? {
      return Ok(Some(true));
    }
    if let Some(edge) = self.edge_at(mouse.column, mouse.row) {
      // Keys and notes end, as on blanks.
      self.app().interrupt();
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
    let typing =
      tab.is_some_and(|tab| tab.panes[tab.focus].overlays.prompt().is_some());
    let Some(drop) = self.drop_step(body, typing, out)? else {
      return Ok(false);
    };
    let done = crate::dnd::target(&drop, typing)
      .and_then(|target| self.dropped(&drop, target));
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

  /// Types a drop into the open prompt, or opens its PDF: in the view
  /// that shows it, or the pane under the drop.
  pub(super) fn dropped(
    &mut self,
    drop: &Dropped,
    target: Target,
  ) -> Result<()> {
    let path = match target {
      Target::Text(text) => {
        let tab = &mut self.tabs[self.active];
        return tab.panes[tab.focus].paste(&text, &mut self.history);
      }
      Target::Pdf(path) => std::fs::canonicalize(path)?,
    };
    if let Some((tab, pane)) = self.existing(&path) {
      return self.focus(tab, pane);
    }
    let (column, row) = drop.cell;
    let under = |app: &App| app.pointer_pane().rect.contains(column, row);
    // A load that fails says so in the pane the drop chose.
    if let Some(index) = self.tab().panes.iter().position(under) {
      self.focus(self.active, index)?;
    }
    let screen = self.screen.unwrap_or(self.app().screen);
    // While the drop lasts, so a file it only promises still exists.
    self.open(&path, false, screen)
  }

  /// A drop on the start screen: a prompt takes its text, else its PDF
  /// opens in a tab.
  pub(super) fn home_drop(
    &mut self,
    home: &mut Home,
    body: &str,
    out: &mut impl Write,
  ) -> Result<Landing> {
    let typing = home.view.typing();
    let drop = match self.drop_step(body, typing, out) {
      Ok(Some(drop)) => drop,
      Ok(None) => return Ok(Landing::Wait),
      Err(error) => {
        home.view.fail(error.to_string());
        return Ok(Landing::Shown);
      }
    };
    let opened = match crate::dnd::target(&drop, typing) {
      Ok(Target::Text(text)) => {
        home.event(Event::Paste(text), &mut self.history).map(|_| false)
      }
      Ok(Target::Pdf(path)) => self
        .open(&path, true, self.screen.unwrap_or(home.view.screen()))
        .map(|()| true),
      Err(error) => Err(error),
    };
    answer(out, &drop.finish(opened.is_ok()))?;
    match opened {
      Ok(true) => Ok(Landing::Opened),
      Ok(false) => Ok(Landing::Shown),
      Err(error) => {
        home.view.fail(error.to_string());
        Ok(Landing::Shown)
      }
    }
  }

  /// Opens `path` from the start screen in a new tab; whether it did. A
  /// failure shows on the start screen.
  pub(super) fn leave_home(&mut self, home: &mut Home, path: &Path) -> bool {
    let opened =
      self.open(path, true, self.screen.unwrap_or(home.view.screen()));
    if let Err(error) = &opened {
      home.view.fail(error.to_string());
    }
    opened.is_ok()
  }

  /// Whether a press hits completions.
  fn on_completion(&mut self, mouse: MouseEvent) -> bool {
    self.app().overlays.prompt().is_some()
      && self.focused_in(Region::Overlay, mouse).is_some()
  }

  /// `mouse` routed in the focused pane's `region`.
  fn focused_in(&mut self, region: Region, mouse: MouseEvent) -> Option<Route> {
    let pane = self.app().pointer_pane();
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
    let app = &self.tabs[self.active].panes[index];
    let item = app.chrome_at(route.mouse.column, route.mouse.row);
    let Some(item) = item else { return Ok(false) };
    self.focus(self.active, index)?;
    let app = self.app();
    app.interrupt();
    app.chrome_press(item, button)?;
    self.pointer.release();
    Ok(true)
  }

  /// Pane index and route under `mouse`.
  pub(super) fn under(&mut self, mouse: MouseEvent) -> Option<(usize, Route)> {
    let tab = self.tabs.get(self.active)?;
    let panes: Vec<_> = tab.panes.iter().map(App::pointer_pane).collect();
    let route =
      vpdf_ui::pointer::route(&panes, tab.panes[tab.focus].id, None, mouse)?;
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
    for app in &mut tab.panes {
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
    let shows = tab.panes.iter().position(|app| app.hover_peek().is_some());
    let Some(index) = shows.filter(|index| *index != tab.focus) else {
      return Ok(());
    };
    let app = &mut tab.panes[index];
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
    tab.panes.iter_mut().find(|app| app.id == pane)
  }

  /// The pointed pane, when free to preview.
  pub(super) fn resting(&mut self) -> Option<&mut App> {
    let tab = self.tabs.get(self.active)?;
    // Keyboard overlays hold rests everywhere.
    let owner = tab.panes[tab.focus].input_owner();
    let free = !owner.is_overlay();
    // Previews and recent cards share one picture.
    let pictured =
      |app: &App| app.overlays.preview().is_some() || app.covered();
    if !free || tab.panes.iter().any(pictured) {
      return None;
    }
    self.pointed_pane()
  }

  /// Retests the last spot; the view changed.
  pub(super) fn rehover(&mut self) {
    if let Some((mouse, pixel)) = self.pointer.spot() {
      let hover = self.hit(mouse, pixel);
      self.pointer.refresh(hover);
    }
    self.show_hover();
    self.leave_check();
  }

  /// Gives each pane its hover.
  fn show_hover(&mut self) {
    let hover = self.pointer.target();
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    for app in &mut tab.panes {
      app.point(hover.filter(|hover| hover.pane == app.id));
    }
  }

  /// Motion picks a menu row or item.
  fn moved_onto(&mut self) {
    let Some(hover) = self.pointer.target() else { return };
    let Some(tab) = self.tabs.get_mut(self.active) else { return };
    let app = tab.panes.iter_mut().find(|app| app.id == hover.pane);
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
    if tab.panes[tab.focus].grabbing() {
      return Shape::Grabbing;
    }
    if self.tabs[self.active].panes.iter().any(App::sweeping) {
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
    let app = &mut self.tab().panes[index];
    let (column, row) = (mouse.column, mouse.row);
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
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.drop_held();
    }
  }

  /// Steps the held edge drag; whether drawn.
  pub fn autoscroll(&mut self, now: Instant) -> bool {
    let held = self.pointer.held_pane();
    let mut changed = false;
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
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
    panes.iter().find_map(|app| app.autoscroll.deadline())
  }
}
