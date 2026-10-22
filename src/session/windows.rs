use super::{
  App, At, Axis, Direction, Node, Notice, PasswordRequired, Path, Place, Rect,
  Result, Screen, Session, Tab, TabMove, Tabline, Window, io, neighbor,
};

pub(super) fn full(screen: Screen) -> Rect {
  Rect { column: 0, row: 0, columns: screen.columns, rows: screen.rows }
}

impl Session {
  pub(super) fn content(&self, mut screen: Screen) -> Screen {
    if self.state.perf {
      let rows = vpdf_ui::perf::rows(screen);
      screen.rows -= rows;
      screen.origin.1 += rows;
    }
    screen
  }

  pub(super) fn area(&self) -> Rect {
    self.screen.map_or_default(|screen| {
      let content = self.content(screen);
      Rect { row: content.origin.1 - screen.origin.1, ..full(content) }
    })
  }

  /// Focuses a pane and blurs the old one.
  pub(super) fn focus(&mut self, tab: usize, pane: usize) -> Result<()> {
    if self.active == tab && self.tabs[tab].focus == pane {
      return Ok(());
    }
    self.leave()?;
    let target = &mut self.tabs[tab];
    if target.focus != pane {
      target.last = Some(target.panes[target.focus].id);
    }
    target.focus = pane;
    self.active = tab;
    self.refocus();
    // A rest from before focus is stale.
    self.app().calm_hover();
    self.arrange()
  }

  pub(super) fn leave(&mut self) -> Result<()> {
    if let Some(tab) = self.tabs.get_mut(self.active)
      && let Some(app) = tab.panes.get_mut(tab.focus)
    {
      app.blur()?;
    }
    self.pointer.release();
    Ok(())
  }

  /// Sizes every pane of the active tab and marks the focused one.
  pub(super) fn arrange(&mut self) -> Result<()> {
    let Some(screen) = self.screen else { return Ok(()) };
    let showing = self.tabs[self.active].order();
    for id in std::mem::replace(&mut self.shown, showing.clone()) {
      if showing.contains(&id) {
        continue;
      }
      let hidden = self.tabs.iter_mut().flat_map(|tab| &mut tab.panes);
      if let Some(app) = hidden.into_iter().find(|app| app.id == id) {
        app.trim();
        self.graphics.forget(id);
      }
    }
    let names =
      self.tabs.iter().map(|tab| tab.panes[tab.focus].name()).collect();
    let active = self.active;
    let area = self.area();
    let tab = &mut self.tabs[active];
    let mut rects = Vec::new();
    tab.tree.layout(area, &mut rects);
    let mut tabline = Some(Tabline { names, active });
    for (id, rect) in rects {
      let Some(index) = tab.index(id) else { continue };
      let focused = index == tab.focus;
      let app = &mut tab.panes[index];
      // Another tab or pane may have drawn over these rows. After a
      // resize the clear also erases images `WezTerm` keeps in cells,
      // which it no longer deletes by placement id.
      app.blank = false;
      app.focused = focused;
      app.tabline = if focused {
        tabline.take().unwrap_or_default()
      } else {
        Tabline::default()
      };
      let pane = Screen {
        columns: rect.columns,
        rows: rect.rows,
        origin: (screen.origin.0 + rect.column, screen.origin.1 + rect.row),
        ..screen
      };
      let at = At { column: rect.column, row: rect.row };
      let before = (app.screen.columns, app.screen.rows, app.screen.origin);
      if before != (pane.columns, pane.rows, pane.origin) || app.at != at {
        // Hint labels and link previews sit at the old size.
        app.overlays.resized();
        app.screen = pane;
        app.at = at;
        app.tiles.invalidate();
        app.relayout()?;
      }
    }
    Ok(())
  }

  pub(super) fn existing(&self, path: &Path) -> Option<(usize, usize)> {
    if let Some(tab) = self.tabs.get(self.active)
      && tab.panes[tab.focus].path == path
    {
      return Some((self.active, tab.focus));
    }
    self.tabs.iter().enumerate().find_map(|(t, tab)| {
      tab.panes.iter().position(|app| app.path == path).map(|p| (t, p))
    })
  }

  /// Opens `path` in a new tab, or in place of the focused pane.
  pub fn open(&mut self, path: &Path, tab: bool, screen: Screen) -> Result<()> {
    self.screen = Some(screen);
    let path = std::fs::canonicalize(path)?;
    let found = self.existing(&path);
    if let Some((tab, pane)) = found {
      return self.focus(tab, pane);
    }
    let app = self.load(&path, "", screen)?;
    self.leave()?;
    if tab || self.tabs.is_empty() {
      return self.insert_tab(app);
    }
    let tab = self.tab();
    let focus = tab.focus;
    let old = std::mem::replace(&mut tab.panes[focus], app);
    let id = tab.panes[focus].id;
    tab.tree.replace(old.id, id);
    self.retire(&old);
    self.refocus();
    self.arrange()
  }

  /// Loads `path` and restores its saved view.
  pub(super) fn load(
    &mut self,
    path: &Path,
    password: &str,
    screen: Screen,
  ) -> Result<App> {
    let mut secret = self.secret.take().unwrap_or_else(|| password.to_owned());
    let mut app = loop {
      match App::configured(
        path,
        &secret,
        false,
        screen,
        self.frontend.clone(),
        &self.settings,
      ) {
        Err(error)
          if error.is::<PasswordRequired>()
            && matches!(self.frontend, crate::raster::Frontend::Terminal) =>
        {
          secret = crate::password::password()?;
        }
        result => break result?,
      }
    };
    self.watcher.add(path);
    if let Some(entry) = self.state.entry(path) {
      app.restore(entry)?;
    }
    self.state.remember_pane(&app);
    self.changed();
    Ok(app)
  }

  /// Saves a closing pane's place, deletes its images, and stops
  /// watching its file.
  fn retire(&mut self, app: &App) {
    self.graphics.forget(app.id);
    self.watcher.remove(&app.path);
    self.state.remember_pane(app);
  }

  pub fn activate(&mut self, index: usize) -> Result<()> {
    if index == self.active || index >= self.tabs.len() {
      return Ok(());
    }
    let focus = self.tabs[index].focus;
    self.focus(index, focus)
  }

  /// Closes the active tab; the session ends after the last.
  pub fn close(&mut self) -> Result<()> {
    self.close_tab(self.active)
  }

  /// Closes a tab; the last quits.
  pub(super) fn close_tab(&mut self, index: usize) -> Result<()> {
    if index >= self.tabs.len() {
      return Ok(());
    }
    if index == self.active {
      self.leave()?;
    }
    let tab = self.tabs.remove(index);
    for app in &tab.panes {
      self.retire(app);
    }
    if self.tabs.is_empty() {
      return self.save();
    }
    if index < self.active {
      self.active -= 1;
    }
    self.active = self.active.min(self.tabs.len() - 1);
    self.refocus();
    self.arrange()
  }

  /// Closes the focused pane, then its tab when it was the last.
  pub(super) fn close_pane(&mut self) -> Result<()> {
    if self.tab().panes.len() == 1 {
      return self.close();
    }
    self.leave()?;
    let area = self.area();
    let app = self.tab().detach(area);
    self.retire(&app);
    self.refocus();
    self.arrange()
  }

  /// Splits the focused pane; the new pane shows `file` or the same view.
  pub(super) fn split(
    &mut self,
    axis: Axis,
    file: Option<&Path>,
    place: Place,
  ) -> Result<()> {
    // The same document reuses the password that unlocked it.
    let (path, password) = match file {
      Some(file) => (std::fs::canonicalize(file)?, String::new()),
      None => (self.app().path.clone(), self.app().password.clone()),
    };
    if file.is_some()
      && let Some((tab, pane)) = self.existing(&path)
    {
      return self.focus(tab, pane);
    }
    let screen = self.app().screen;
    anyhow::ensure!(Node::fits(axis, full(screen)), "Not enough room");
    let mut app = self.load(&path, &password, screen)?;
    if file.is_none() {
      app.mirror(self.app())?;
    }
    let tab = self.tab();
    let target = tab.panes[tab.focus].id;
    tab.tree.split(target, axis, app.id, place);
    tab.panes.push(app);
    let index = tab.panes.len() - 1;
    self.focus(self.active, index)
  }

  /// A split beside, placed by `land`; focus stays.
  pub(super) fn split_beside(
    &mut self,
    land: impl FnOnce(&mut App) -> Result<()>,
  ) -> Result<()> {
    let (tab, pane) = (self.active, self.tab().focus);
    // A double click must not split twice.
    self.keeping_pointer(|session| {
      session.split(Axis::Columns, None, Place::After)?;
      // Focus returns even when landing fails.
      let landed = land(session.app());
      session.focus(tab, pane)?;
      landed
    })
  }

  /// Keeps the click run across `work`.
  pub(super) fn keeping_pointer<R>(
    &mut self,
    work: impl FnOnce(&mut Self) -> R,
  ) -> R {
    let pointer = std::mem::take(&mut self.pointer);
    let result = work(self);
    self.pointer = pointer;
    result
  }

  pub(super) fn window(
    &mut self,
    command: Window,
    (count, explicit): (usize, bool),
    file: Option<&Path>,
  ) -> Result<()> {
    let area = self.area();
    let tab = self.tab();
    let id = tab.panes[tab.focus].id;
    let order = tab.order();
    let at = order.iter().position(|pane| *pane == id).unwrap_or(0);
    let total = order.len();
    let target = match command {
      Window::Split(axis) => return self.split(axis, file, Place::Before),
      Window::Close => return self.close_pane(),
      Window::Only => return self.only(),
      Window::ToTab => return self.move_to_tab(),
      Window::Focus(direction) => {
        let mut rects = Vec::new();
        tab.tree.layout(area, &mut rects);
        neighbor(&rects, id, direction)
      }
      Window::Next if explicit => Some(order[count.clamp(1, total) - 1]),
      Window::Next => Some(order[(at + count) % total]),
      Window::Previous => Some(order[(at + total - count % total) % total]),
      Window::Last => tab.last,
      Window::Equalize => {
        tab.tree.equalize();
        None
      }
      Window::Resize(axis, step) => {
        let cells = step * i32::try_from(count).unwrap_or(1);
        tab.tree.resize(id, axis, cells, area);
        None
      }
      Window::Extend(direction) => {
        tab.tree.extend(id, direction, i32::try_from(count).unwrap_or(1), area);
        None
      }
      Window::Maximize(axis) => {
        tab.tree.resize(id, axis, i32::from(u16::MAX), area);
        None
      }
      Window::Exchange => {
        tab.tree.exchange(id);
        None
      }
      Window::Rotate(forward) => {
        anyhow::ensure!(
          tab.tree.rotate(id, forward),
          "Cannot rotate when another pane is split"
        );
        None
      }
      Window::New => {
        self.record();
        self.split(Axis::Rows, None, Place::Before)?;
        let recent = self.state.recent.clone();
        return self.app().show_recent(&recent);
      }
      Window::Move(direction) => {
        tab.tree.move_to_edge(id, direction);
        None
      }
    };
    match target.and_then(|id| self.tab().index(id)) {
      Some(index) => self.focus(self.active, index),
      None => self.arrange(),
    }
  }

  /// `Ctrl-w o`: closes every pane but the focused one.
  pub(super) fn only(&mut self) -> Result<()> {
    if self.tab().panes.len() == 1 {
      return Ok(());
    }
    self.leave()?;
    let tab = self.tab();
    let keep = tab.panes[tab.focus].id;
    let closing: Vec<_> =
      tab.panes.extract_if(.., |app| app.id != keep).collect();
    tab.tree = Node::leaf(keep);
    tab.focus = 0;
    tab.last = None;
    for app in &closing {
      self.retire(app);
    }
    self.refocus();
    self.arrange()
  }

  /// `Ctrl-w T`: moves the focused pane into a new tab.
  fn move_to_tab(&mut self) -> Result<()> {
    if self.tab().panes.len() == 1 {
      return Ok(());
    }
    self.leave()?;
    let area = self.area();
    let app = self.tab().detach(area);
    self.insert_tab(app)
  }

  /// Puts `app` in a tab after the active one and focuses it.
  fn insert_tab(&mut self, app: App) -> Result<()> {
    let index = if self.tabs.is_empty() { 0 } else { self.active + 1 };
    self.tabs.insert(index, Tab::new(app));
    self.active = index;
    self.refocus();
    self.arrange()
  }

  /// Copies the focused view into a new tab covered by recent documents.
  pub(super) fn recent_tab(&mut self, screen: Screen) -> Result<()> {
    self.record();
    let path = self.app().path.clone();
    let password = self.app().password.clone();
    let mut app = self.load(&path, &password, screen)?;
    app.mirror(self.app())?;
    self.leave()?;
    self.insert_tab(app)?;
    let recent = self.state.recent.clone();
    self.app().show_recent(&recent)
  }

  /// Fits the start screen or every pane to a new terminal size.
  pub fn resize(&mut self, screen: Screen) -> Result<()> {
    self.perf.view.invalidate();
    self.screen = Some(screen);
    let content = self.content(screen);
    if let Some(home) = &mut self.home {
      home.view.resize(
        content,
        At { row: content.origin.1 - screen.origin.1, ..At::default() },
      );
    }
    if self.tabs.is_empty() {
      return Ok(());
    }
    for app in self.tabs.iter_mut().flat_map(|tab| &mut tab.panes) {
      app.at = At { column: u16::MAX, row: u16::MAX };
    }
    self.arrange()
  }

  /// Fits the start screen or panes to a `measured` screen; when its
  /// cells could not be measured, keeps the layout and says why on the
  /// last row of `grid`, the terminal's columns and rows.
  pub(super) fn refit(
    &mut self,
    measured: Result<Option<Screen>>,
    grid: impl FnOnce() -> io::Result<(u16, u16)>,
  ) -> Result<()> {
    match measured {
      Ok(Some(screen)) => {
        self.unmeasured = None;
        self.resize(screen)
      }
      Ok(None) => Ok(()),
      Err(error) => {
        let (columns, rows) = grid()?;
        let error = error.to_string();
        self.unmeasured = Some(Notice::new(rows, columns, error));
        Ok(())
      }
    }
  }
}

impl Tab {
  pub(super) fn new(app: App) -> Self {
    Self { tree: Node::leaf(app.id), panes: vec![app], focus: 0, last: None }
  }

  pub(super) fn index(&self, id: u32) -> Option<usize> {
    self.panes.iter().position(|pane| pane.id == id)
  }

  /// Pane ids in screen order.
  pub fn order(&self) -> Vec<u32> {
    self.tree.leaves()
  }

  pub(super) fn detach(&mut self, area: Rect) -> App {
    let id = self.panes[self.focus].id;
    let mut rects = Vec::new();
    self.tree.layout(area, &mut rects);
    let peer =
      self.last.filter(|peer| *peer != id && self.index(*peer).is_some());
    let next = peer.or_else(|| {
      [Direction::Right, Direction::Down, Direction::Left, Direction::Up]
        .into_iter()
        .find_map(|direction| neighbor(&rects, id, direction))
    });
    let app = self.panes.remove(self.focus);
    self.tree.remove(id);
    self.focus = next.and_then(|id| self.index(id)).unwrap_or(0);
    self.last = None;
    app
  }
}

impl Session {
  pub fn tab(&mut self) -> &mut Tab {
    &mut self.tabs[self.active]
  }

  pub fn app(&mut self) -> &mut App {
    let tab = self.tab();
    &mut tab.panes[tab.focus]
  }

  /// Readies the newly focused pane to request its view again.
  pub(super) fn refocus(&mut self) {
    self.app().tiles.invalidate();
  }

  pub(super) fn step(&self, step: TabMove) -> usize {
    let count = self.tabs.len();
    match step {
      TabMove::Next(n) => (self.active + n) % count,
      TabMove::Previous(n) => (self.active + count - n % count) % count,
      TabMove::Go(n) => n.clamp(1, count) - 1,
    }
  }
}
