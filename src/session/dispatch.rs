use super::{
  Axis, KeyEvent, Mark, Pane, PanelKind, Path, Place, Position, Request,
  Result, Screen, Session, Sight, terminal,
};

impl Session {
  pub(super) fn reload_config(&mut self) {
    let loaded = self.config_source.reload(&self.settings);
    let previous_warning = self.config_warning.take();
    self.config_warning = loaded.warning.clone();
    self.settings = loaded.settings;
    vpdf_ui::panel::set_ascii_glyphs(self.settings.ascii_glyphs());
    for tab in &mut self.tabs {
      for app in tab.panes.iter_mut().filter_map(Pane::document_mut) {
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
    for home in self
      .tabs
      .iter_mut()
      .flat_map(|tab| &mut tab.panes)
      .filter_map(Pane::recent_mut)
    {
      home.configure(&self.settings);
      home.view.reset_input();
      if home.view.error() == previous_warning.as_deref() {
        home.view.clear_error();
      }
    }
    self.present_startup();
    self.refresh_help();
  }

  pub(super) fn choose_theme(&mut self, name: &str) {
    let selected = crate::config::ThemeChoice::load(name, Path::new("."));
    self.set_theme(selected.as_ref().cloned().unwrap_or_default());
    if let Err(error) = selected {
      let warning = format!("appearance.theme: {error}");
      self.startup_warning =
        Some(vpdf_model::text::printable(&warning).collect());
      self.present_startup();
    }
  }

  pub(super) fn set_theme(
    &mut self,
    theme: impl Into<crate::config::ThemeChoice>,
  ) {
    let theme = theme.into();
    self.settings.set_theme(theme.clone());
    for tab in &mut self.tabs {
      for app in tab.panes.iter_mut().filter_map(Pane::document_mut) {
        app.settings.set_theme(theme.clone());
      }
    }
    for home in self
      .tabs
      .iter_mut()
      .flat_map(|tab| &mut tab.panes)
      .filter_map(Pane::recent_mut)
    {
      home.configure(&self.settings);
    }
    self.refresh_help();
  }

  pub(super) fn pick(&mut self, root: &Path, tab: bool) -> Result<()> {
    if self.picker.is_none() {
      self.picker = Some(crate::programs::Picker::start(root, tab)?);
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
    let fallback = self.focused().screen;
    if self.focused().is_recent() {
      return Ok(());
    }
    if self.app()?.quit {
      return self.close_pane();
    }
    let screen = self.screen.unwrap_or(fallback);
    let request = self.app()?.request.take();
    let retry = if matches!(self.frontend, crate::raster::Frontend::Native(_)) {
      request.clone()
    } else {
      None
    };
    let result = self.dispatch(request, screen);
    self.unlock_result(result, retry.map(super::unlock::Operation::Request))
  }

  pub(super) fn replay_keys(&mut self, keys: Vec<KeyEvent>) -> Result<()> {
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

  fn pick_request(
    &mut self,
    root: &Path,
    tab: bool,
    screen: Screen,
  ) -> Result<()> {
    if matches!(self.frontend, crate::raster::Frontend::Native(_)) {
      return self.pick(root, tab);
    }
    match crate::programs::pick_pdf(root)? {
      Some(path) => self.open(&path, tab, screen),
      None => Ok(()),
    }
  }

  pub(super) fn dispatch(
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
      Some(Request::Reference { reference, tab }) => {
        self.open_reference(&reference, tab, screen)
      }
      Some(Request::Alternate) => self.alternate(false),
      Some(Request::DeleteBuffer) => self.delete_buffer(),
      Some(Request::NewSplit { axis, file }) => {
        self.new_split(axis, file.as_ref().map(|r| r.path.as_path()))?;
        if let Some(file) = file {
          self.app()?.apply_reference(&file.operations)?;
        }
        Ok(())
      }
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
        self.window(command, (count, explicit), file.as_ref())
      }
      Some(Request::Panel(kind)) => {
        self.pointer.release();
        if kind == PanelKind::Recent {
          self.record();
          return self.show_recent();
        }
        let global = self.state.marks.clone();
        self.app()?.show_marks(&global)
      }
      Some(Request::SetMark(name)) => self.set_global_mark(name),
      Some(Request::MarkSet) => {
        self.refill_marks();
        Ok(())
      }
      Some(Request::GoMark(name, exact)) => {
        let mark = self.global_mark(name)?;
        self.open(&mark.path, true, screen)?;
        self.app()?.go_to(mark.cursor, exact)
      }
      Some(Request::SplitAt(position)) => {
        self.split_beside(|app| app.land_at(position))
      }
      Some(Request::SplitTo(target)) => {
        self.split_beside(|app| app.follow(&target))
      }
      Some(Request::Keys(keys)) => self.replay_keys(keys),
      Some(Request::SplitMark(name)) => self.split_mark(name, screen),
      Some(Request::Pick { root, tab }) => {
        self.pick_request(&root, tab, screen)
      }
      Some(Request::TabClose(tab)) => {
        self.close_tab(tab.unwrap_or(self.active))
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
        self.choose_theme(&theme);
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
    if mark.path == self.app()?.path {
      self.state.refresh_history(self.store.as_deref());
      self.split_beside(|app| app.land_at(mark.cursor))?;
      self.remember_open(self.active, self.tabs[self.active].focus);
      return Ok(());
    }
    self.panel_open(&mark.path, Some(mark.cursor), false, screen)
  }

  fn set_global_mark(&mut self, name: char) -> Result<()> {
    let app = self.app()?;
    anyhow::ensure!(app.manual.is_none(), "Global marks unavailable for help");
    let mark = Mark::new(app.path.clone(), app.reading_position()?);
    self.state.set_mark(name, mark);
    self.refill_marks();
    self.changed();
    Ok(())
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
    // Recent entries fill their own pane.
    let here = focus && self.focused().is_recent();
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
    let app = self.tabs[target.0].panes[target.1]
      .document_mut()
      .ok_or_else(|| anyhow::anyhow!("No document"))?;
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

  pub fn type_key(&mut self, key: KeyEvent) -> Result<()> {
    self.resizing = None;
    if self.focused().is_recent() {
      return self.recent_event(vpdf_model::input::Event::Key(key));
    }
    let tab = &mut self.tabs[self.active];
    let focus = tab.focus;
    for (index, app) in tab
      .panes
      .iter_mut()
      .enumerate()
      .filter_map(|(i, p)| p.document_mut().map(|a| (i, a)))
    {
      if index != focus {
        app.drop_hover_preview();
      }
    }
    let app = tab.panes[focus]
      .document_mut()
      .ok_or_else(|| anyhow::anyhow!("No document"))?;
    let before =
      app.panel.as_ref().filter(|p| p.view.focused()).map(|p| p.view.kind());
    app.clear_notes();
    let result = app.key(key, &mut self.history);
    app.after_key();
    let after =
      app.panel.as_ref().filter(|p| p.view.focused()).map(|p| p.view.kind());
    if before != after {
      self.pointer.release();
    }
    result
  }

  pub(super) fn recent_event(
    &mut self,
    event: vpdf_model::input::Event,
  ) -> Result<()> {
    let terminal = matches!(self.frontend, crate::raster::Frontend::Terminal);
    let tab = &mut self.tabs[self.active];
    let Some(home) = tab.panes[tab.focus].recent_mut() else { return Ok(()) };
    let outcome = home.handle(event, &mut self.history, terminal)?;
    self.record_recent_layout();
    self.recent_outcome(outcome)
  }

  pub(super) fn recent_outcome(
    &mut self,
    outcome: crate::home::Outcome,
  ) -> Result<()> {
    use crate::home::Outcome;
    use vpdf_ui::keys::Action;
    let screen = self.screen.unwrap_or(self.focused().screen);
    let request = match outcome {
      Outcome::Stay => return Ok(()),
      Outcome::Open(path) => Request::Open { path, tab: false },
      Outcome::Reference(reference) => {
        Request::Reference { reference, tab: false }
      }
      Outcome::Forget(paths) => Request::ForgetRecent(paths),
      Outcome::Command(line) => return self.command(&line),
      Outcome::Quit => return self.dismiss_recent(true),
      Outcome::Dismiss => return self.dismiss_recent(false),
      Outcome::Input(input) => match input.action {
        Action::Alternate => Request::Alternate,
        Action::Window(command) => Request::Window {
          command,
          count: input.count,
          explicit: input.explicit,
          file: None,
        },
        Action::Tab(forward) => Request::Tab(match (forward, input.explicit) {
          (true, true) => super::TabMove::Go(input.count),
          (true, false) => super::TabMove::Next(input.count),
          (false, _) => super::TabMove::Previous(input.count),
        }),
        _ => return Ok(()),
      },
    };
    let result = self.dispatch(Some(request.clone()), screen);
    self.unlock_result(result, Some(super::unlock::Operation::Request(request)))
  }

  pub fn command(&mut self, line: &str) -> Result<()> {
    use vpdf_ui::commands::{self, Id, View};
    if !self.focused().is_recent() {
      self.app()?.command(line)?;
      return self.settle();
    }
    let terminal = matches!(self.frontend, crate::raster::Frontend::Terminal);
    let commands::Resolution::Command { spec, argument } = commands::resolve(
      line,
      View::Home,
      |p| crate::cmdline::available(p, terminal),
      |_| false,
    )?
    else {
      return Ok(());
    };
    let invocation = self
      .focused()
      .recent()
      .map_or_else(vpdf_ui::topics::Invocation::home, |home| {
        home.view.invocation()
      });
    if let Some(prompt) =
      crate::commands::path_prompt(spec.id, argument, terminal, || {
        invocation.clone()
      })
    {
      self.focused_mut().open_prompt(prompt);
      return Ok(());
    }
    let request =
      match crate::commands::command_request(spec, argument, || invocation)? {
        Some(request) => request,
        None if spec.id == Id::Quit => return self.close_pane(),
        None if spec.id == Id::Version => {
          if let Some(home) = self.focused_mut().recent_mut() {
            home.view.show_info(
              PanelKind::Version,
              &crate::version::panel_text(terminal),
            );
          }
          return Ok(());
        }
        None => anyhow::bail!("Open a document before :{}", spec.name),
      };
    let screen = self.screen.unwrap_or(self.focused().screen);
    let result = self.dispatch(Some(request.clone()), screen);
    self.unlock_result(result, Some(super::unlock::Operation::Request(request)))
  }
}
