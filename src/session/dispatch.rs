use super::{
  Axis, KeyEvent, Mark, PanelKind, Path, Place, Position, Request, Result,
  Screen, Session, Sight, terminal,
};

impl Session {
  pub(super) fn reload_config(&mut self) {
    let loaded = self.config_source.reload(&self.settings);
    let previous_warning = self.config_warning.take();
    self.config_warning = loaded.warning.clone();
    self.settings = loaded.settings;
    vpdf_ui::panel::set_ascii_glyphs(self.settings.ascii_glyphs());
    for tab in &mut self.tabs {
      for app in &mut tab.panes {
        if previous_warning.as_ref() == Some(&app.reader.message) {
          app.reader.notice("");
        }
        app.settings = self.settings.clone();
        app.disarm();
        if let Some(menu) = app.overlays.menu_mut() {
          menu.refresh_bindings(self.settings.bindings());
        }
        if app.region_notice.as_ref() == Some(&app.reader.message) {
          let drawing = app
            .overlays
            .region()
            .is_some_and(|region| region.draft().is_some());
          app.show_region_hint(drawing);
        }
        if let Some(hints) = app.overlays.hints_mut() {
          hints.reset_input();
        }
        if let Some(prompt) = app.overlays.prompt_mut() {
          prompt.set_bindings(
            self
              .settings
              .bindings()
              .map(vpdf_ui::bindings::Context::prompt(prompt.kind())),
          );
          prompt.reset_input();
        }
      }
    }
    if let Some(unlock) = &mut self.unlock {
      unlock.prompt.set_bindings(
        self.settings.bindings().map(vpdf_ui::bindings::Context::Password),
      );
      unlock.prompt.reset_input();
    }
    self.startup_warning = loaded.warning;
    if let Some(home) = &mut self.home {
      home.configure(&self.settings);
      home.view.reset_input();
      if home.view.error() == previous_warning.as_deref() {
        home.view.clear_error();
      }
    }
    self.present_startup();
    self.refresh_help();
  }

  pub(super) fn set_theme(&mut self, theme: vpdf_model::theme::Theme) {
    self.settings.set_theme(theme);
    for tab in &mut self.tabs {
      for app in &mut tab.panes {
        app.settings.set_theme(theme);
      }
    }
    if let Some(home) = &mut self.home {
      home.configure(&self.settings);
    }
    self.refresh_help();
  }

  pub(super) fn pick(&mut self, tab: bool) -> Result<()> {
    if self.picker.is_none() {
      self.picker = Some(crate::programs::Picker::start(tab)?);
    }
    Ok(())
  }

  pub(super) fn toggle_perf(&mut self, on: Option<bool>) -> Result<()> {
    self.state.set_perf(on.unwrap_or(!self.state.perf));
    self.changed();
    self.perf.running(self.state.perf && self.sight != Sight::Hidden);
    if let Some(screen) = self.screen {
      self.resize(screen)?;
    }
    Ok(())
  }

  /// Applies what the focused pane asked for after an input.
  pub fn settle(&mut self) -> Result<()> {
    if self.tabs.is_empty() {
      return Ok(());
    }
    let fallback = self.app().screen;
    if self.app().quit {
      return self.close_pane();
    }
    let screen = self.screen.unwrap_or(fallback);
    let request = self.app().request.take();
    let retry = if matches!(self.frontend, crate::raster::Frontend::Native(_)) {
      request.clone()
    } else {
      None
    };
    let result = self.dispatch(request, screen);
    self.unlock_result(result, retry.map(super::unlock::Operation::Request))
  }

  fn replay_keys(&mut self, keys: Vec<KeyEvent>) -> Result<()> {
    let mut keys = keys.into_iter();
    while let Some(key) = keys.next() {
      self.interact(super::interaction::Interaction::Key(key))?;
      if self.quit || self.tabs.is_empty() {
        break;
      }
      if self.blocked() {
        self.pending_keys.extend(keys);
        break;
      }
    }
    Ok(())
  }

  fn dispatch(
    &mut self,
    request: Option<Request>,
    screen: Screen,
  ) -> Result<()> {
    match request {
      Some(Request::HelpBeside { topic, invocation }) => {
        self.request_help_at(&topic, invocation, screen, true)
      }
      Some(Request::Help { topic, invocation }) => {
        self.request_help(&topic, invocation, screen)
      }
      Some(Request::Open { path, tab }) => self.open(&path, tab, screen),
      Some(Request::Print) => self.print(),
      Some(Request::RecentTab) => self.recent_tab(screen),
      Some(Request::ForgetRecent(paths)) => self.forget_recent(&paths),
      Some(Request::RemoveMark(name)) => self.remove_mark(name),
      Some(Request::PanelOpen { path, position, focus }) => {
        self.panel_open(&path, position, focus, screen)
      }
      Some(Request::PanelMark { name, focus }) => {
        let mark = self.global_mark(name)?;
        self.panel_open(&mark.path, Some(mark.cursor), focus, screen)
      }
      Some(Request::Tab(step)) => self.activate(self.step(step)),
      Some(Request::Window { command, count, explicit, file }) => {
        self.window(command, (count, explicit), file.as_deref())
      }
      Some(Request::Panel(kind)) => {
        self.pointer.release();
        if kind == PanelKind::Recent {
          self.record();
          let recent = self.state.recent.clone();
          return self.app().show_recent(&recent);
        }
        let global = self.state.marks.clone();
        self.app().show_marks(&global)
      }
      Some(Request::SetMark(name)) => {
        let app = self.app();
        anyhow::ensure!(
          app.manual.is_none(),
          "Global marks unavailable for help"
        );
        let mark = Mark::new(app.path.clone(), app.reading_position()?);
        self.state.set_mark(name, mark);
        self.refill_marks();
        self.changed();
        Ok(())
      }
      Some(Request::MarkSet) => {
        self.refill_marks();
        Ok(())
      }
      Some(Request::GoMark(name, exact)) => {
        let mark = self.global_mark(name)?;
        self.open(&mark.path, true, screen)?;
        self.app().go_to(mark.cursor, exact)
      }
      Some(Request::SplitAt(position)) => {
        self.split_beside(|app| app.land_at(position))
      }
      Some(Request::SplitTo(target)) => {
        self.split_beside(|app| app.follow(&target))
      }
      Some(Request::Keys(keys)) => self.replay_keys(keys),
      Some(Request::SplitMark(name)) => self.split_mark(name, screen),
      Some(Request::Pick { tab })
        if matches!(self.frontend, crate::raster::Frontend::Native(_)) =>
      {
        self.pick(tab)
      }
      Some(Request::Pick { tab }) => {
        match crate::programs::pick_pdf(&std::env::current_dir()?)? {
          Some(path) => self.open(&path, tab, screen),
          None => Ok(()),
        }
      }
      Some(Request::TabClose(index)) => {
        self.close_tab(index.unwrap_or(self.active))
      }
      Some(Request::QuitAll) => {
        self.quit = true;
        Ok(())
      }
      Some(Request::ConfigReload) => {
        self.reload_config();
        Ok(())
      }
      Some(Request::Theme(theme)) => {
        self.set_theme(theme);
        Ok(())
      }
      Some(Request::Perf(on)) => self.toggle_perf(on),
      Some(Request::Mouse(on)) => {
        if !on {
          self.drop_pointer();
          self.pointer_left();
        }
        terminal::capture_mouse(on)
      }
      None => Ok(()),
    }
  }

  fn split_mark(&mut self, name: char, screen: Screen) -> Result<()> {
    let mark = self.global_mark(name)?;
    if mark.path == self.app().path {
      self.state.refresh_history(self.store.as_deref());
      self.split_beside(|app| app.land_at(mark.cursor))?;
      self.remember_open(self.active, self.tabs[self.active].focus);
      return Ok(());
    }
    self.panel_open(&mark.path, Some(mark.cursor), false, screen)
  }

  fn global_mark(&self, name: char) -> Result<Mark> {
    let mark = self.state.marks.get(&name).cloned();
    mark.ok_or_else(|| anyhow::anyhow!("Mark not set: {name}"))
  }

  fn panel_open(
    &mut self,
    path: &Path,
    position: Option<Position>,
    focus: bool,
    screen: Screen,
  ) -> Result<()> {
    let path = std::fs::canonicalize(path)?;
    self.state.refresh_history(self.store.as_deref());
    let source = (self.active, self.tab().focus);
    // The recent view picks a document for the pane it covers.
    let here = focus && self.app().covered();
    let found = self.existing(&path);
    let target = if let Some(target) = found {
      target
    } else {
      if focus {
        self.open(&path, !here, screen)?;
      } else {
        // Beside the source, which keeps focus.
        self.keeping_pointer(|session| {
          session.split(Axis::Columns, Some(&path), Place::After)
        })?;
      }
      (self.active, self.tab().focus)
    };
    if here && target != source {
      let app = &mut self.tabs[source.0].panes[source.1];
      if let Some(panel) = &mut app.panel {
        panel.view.focus(false);
      }
      app.relayout()?;
    }
    let app = &mut self.tabs[target.0].panes[target.1];
    if let Some(position) = position {
      app.land_at(position)?;
    }
    app.close_overview();
    if focus {
      app.reader.normal();
      app.reader.cancel();
      if let Some(panel) = &mut app.panel {
        panel.view.focus(false);
      }
      app.relayout()?;
      self.focus(target.0, target.1)?;
    } else {
      // A second middle press must still find its run.
      self.keeping_pointer(|session| session.focus(source.0, source.1))?;
    }
    if found.is_some() {
      self.remember_open(target.0, target.1);
    }
    Ok(())
  }

  /// Types `key` into the focused pane.
  pub(super) fn type_key(&mut self, key: KeyEvent) -> Result<()> {
    let tab = &mut self.tabs[self.active];
    // `App::key` closes the focused one's.
    let focus = tab.focus;
    for (index, app) in tab.panes.iter_mut().enumerate() {
      if index != focus {
        app.drop_hover_preview();
      }
    }
    let app = &mut tab.panes[tab.focus];
    let before =
      app.panel.as_ref().filter(|p| p.view.focused()).map(|p| p.view.kind());
    app.clear_notes();
    // Keys may change the tree mid-drag.
    self.resizing = None;
    // Clues close even when the key fails.
    let result = app.key(key, &mut self.history);
    app.after_key();
    let after =
      app.panel.as_ref().filter(|p| p.view.focused()).map(|p| p.view.kind());
    if before != after {
      self.pointer.release();
    }
    result
  }
}
