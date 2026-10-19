use super::{
  App, Hit, InputOwner, KeyModifiers, LinkPress, MouseButton, MouseEvent,
  MouseEventKind, Region, Result, Route, WHEEL_ROWS, Wheel,
};

impl App {
  /// A screen report's cell in this pane.
  pub(crate) fn pane_cell(&self, mouse: MouseEvent) -> (u16, u16) {
    let column = mouse.column.wrapping_sub(self.at.column);
    (column, mouse.row.wrapping_sub(self.at.row))
  }

  /// Cancels pending keys, as `Esc` does.
  pub fn disarm(&mut self) {
    self.search.forget_yank();
    self.keys.clear();
    self.reader.disarm();
    self.after_key();
  }

  /// Closes key-driven state as focus leaves.
  pub fn blur(&mut self) -> Result<()> {
    self.cancel_definition();
    // Focus loss closes this pane's overlays.
    self.dismiss_overlay();
    self.hints = None;
    self.drop_scrub();
    self.cancel_prompt()?;
    self.disarm();
    Ok(())
  }

  /// Acts on a routed pointer event.
  pub fn mouse(&mut self, route: Route) -> Result<()> {
    let Route { mouse, region, .. } = route;
    if mouse.kind != MouseEventKind::Moved {
      self.cancel_definition();
    }
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
      self.autoscroll.cancel();
    }
    if matches!(mouse.kind, MouseEventKind::Down(_)) && self.dismiss_click()? {
      return Ok(());
    }
    // A new press means a lost release.
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      self.drop_scrub();
    }
    // An overlay opened mid-press keeps the release.
    if mouse.kind == MouseEventKind::Up(MouseButton::Middle)
      && self.input_owner() != InputOwner::Content
    {
      self.grab.cancel();
    }
    let right = matches!(
      mouse.kind,
      MouseEventKind::Drag(MouseButton::Right)
        | MouseEventKind::Up(MouseButton::Right)
    );
    if right && self.context_menu.is_some() {
      return self.menu_drag(route);
    }
    // The page still scrolls under a menu.
    let wheel = Wheel::of(mouse.kind).is_some();
    if self.prompt.is_some()
      || self.context_menu.is_some() && !wheel
      || self.region.as_ref().is_some_and(vpdf_ui::region::Region::is_bar)
    {
      return Ok(());
    }
    match region {
      // The status row's wheel scrolls the pane above it.
      Region::Status if wheel => return self.menu_wheel(route),
      Region::Panel => return self.panel_mouse(mouse, route.clicks),
      Region::Overlay if wheel => {
        if let Some(turn) = Wheel::of(mouse.kind) {
          let step = if turn.forward() { 1 } else { -1 };
          self.scroll_clues(step * WHEEL_ROWS as isize);
        }
        return Ok(());
      }
      Region::Overlay | Region::Status | Region::Actions | Region::Edge => {
        return Ok(());
      }
      // Only turns: presses go to them first.
      Region::Preview => return self.page_wheel(route),
      Region::Menu => return self.menu_wheel(route),
      Region::Page | Region::Overview => {}
    }
    if matches!(mouse.kind, MouseEventKind::Down(_)) && self.panel_focused() {
      self.blur()?;
      if let Some(panel) = &mut self.panel {
        panel.view.focus(false);
      }
      self.relayout()?;
    }
    if region == Region::Overview {
      return self.overview_mouse(mouse);
    }
    // Drags from before `gp` stay hidden.
    if self.overview.is_some() {
      return Ok(());
    }
    match mouse.kind {
      kind if Wheel::of(kind).is_some() => self.page_wheel(route),
      MouseEventKind::Down(MouseButton::Left)
        if self.on_bar(mouse.column) && mouse.modifiers.is_empty() =>
      {
        self.bar_mouse(&route)
      }
      MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left)
        if self.bar.scrubbing() =>
      {
        self.bar_mouse(&route)
      }
      MouseEventKind::Down(MouseButton::Left)
        if mouse.modifiers.contains(KeyModifiers::ALT) =>
      {
        self.page_alt(route)
      }
      MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left)
        if self.gesture.alt() =>
      {
        self.page_alt(route)
      }
      MouseEventKind::Down(MouseButton::Left)
      | MouseEventKind::Drag(MouseButton::Left)
      | MouseEventKind::Up(MouseButton::Left) => self.page_left(route),
      MouseEventKind::Down(MouseButton::Right) => {
        self.page_context(route);
        Ok(())
      }
      MouseEventKind::Down(MouseButton::Middle)
      | MouseEventKind::Drag(MouseButton::Middle)
      | MouseEventKind::Up(MouseButton::Middle) => self.page_middle(&route),
      _ => Ok(()),
    }
  }

  /// Left button: select, or follow links.
  fn page_left(&mut self, route: Route) -> Result<()> {
    let mouse = route.mouse;
    match mouse.kind {
      MouseEventKind::Up(_) => {
        return match std::mem::take(&mut self.link_press) {
          LinkPress::Held(target) => {
            self.link_press = LinkPress::Followed;
            self.follow(&target)
          }
          LinkPress::Note(page, index, _) => {
            self.link_press = LinkPress::Followed;
            self.show_note(page, index)
          }
          press => {
            self.link_press = press;
            Ok(())
          }
        };
      }
      // A followed run's later presses do nothing.
      _ if route.clicks > 1
        && matches!(self.link_press, LinkPress::Followed) =>
      {
        return Ok(());
      }
      MouseEventKind::Drag(_) => {
        // An icon press selects once it drags.
        if let LinkPress::Note(.., Some(from)) = self.link_press {
          self.click_select(from, 1)?;
        }
        self.link_press = LinkPress::Idle;
      }
      _ => {
        self.gesture.reset();
        // A broken link table still selects.
        let hit = (route.clicks == 1)
          .then(|| self.pressed_hit(mouse.column, mouse.row, route.pixel));
        let hit = hit.and_then(Result::ok).unwrap_or_default();
        if let Hit::Note { page, index, text: false } = hit {
          // The cursor stays until a drag.
          self.animation.cancel();
          let from = self.press_position(&route)?;
          self.link_press = LinkPress::Note(page, index, from);
          return Ok(());
        }
        let link = self.link_of(hit).ok().flatten();
        self.link_press = link.map_or(LinkPress::Idle, LinkPress::Held);
      }
    }
    self.animation.cancel();
    if matches!(mouse.kind, MouseEventKind::Drag(_)) {
      self.edge_follow(&route)?;
      return self.drag_to(&route);
    }
    let Some(pos) = self.press_position(&route)? else { return Ok(()) };
    self.click_select(pos, route.clicks)
  }

  /// Alt-left on the page.
  fn page_alt(&mut self, route: Route) -> Result<()> {
    self.link_press = LinkPress::Idle;
    self.animation.cancel();
    match route.mouse.kind {
      MouseEventKind::Down(_) => self.alt_press(&route),
      MouseEventKind::Drag(_) => self.alt_drag(&route),
      _ => self.alt_release(),
    }
  }
}
