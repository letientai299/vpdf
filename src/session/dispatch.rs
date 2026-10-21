use super::{
  Axis, KeyEvent, Mark, PanelKind, Path, Place, Position, Request, Result,
  Screen, Session, Sight, terminal,
};

impl Session {
  pub(super) fn pick(&mut self) -> Result<()> {
    if self.picker.is_none() {
      self.picker = Some(crate::programs::Picker::start()?);
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

  fn dispatch(
    &mut self,
    request: Option<Request>,
    screen: Screen,
  ) -> Result<()> {
    match request {
      Some(Request::Open { path, tab }) => self.open(&path, tab, screen),
      Some(Request::RecentTab) => self.recent_tab(screen),
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
        let rows = self.app().mark_rows(&global);
        self.app().show_panel(kind, rows)
      }
      Some(Request::SetMark(name)) => {
        let app = self.app();
        let mark =
          Mark { path: app.path.clone(), cursor: app.reading_position()? };
        self.state.marks.insert(name, mark);
        self.changed();
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
      // Each key's request applies before the next.
      Some(Request::Keys(keys)) => {
        let mut keys = keys.into_iter();
        while let Some(key) = keys.next() {
          self.interact(super::interaction::Interaction::Key(key))?;
          if self.quit || self.tabs.is_empty() {
            break;
          }
          if self.unlock.is_some() || self.picker.is_some() {
            self.pending_keys.extend(keys);
            break;
          }
        }
        Ok(())
      }
      Some(Request::SplitMark(name)) => {
        let mark = self.global_mark(name)?;
        if mark.path == self.app().path {
          return self.split_beside(|app| app.land_at(mark.cursor));
        }
        self.panel_open(&mark.path, Some(mark.cursor), false, screen)
      }
      Some(Request::Pick)
        if matches!(self.frontend, crate::raster::Frontend::Native(_)) =>
      {
        self.pick()
      }
      Some(Request::Pick) => {
        match crate::programs::pick_pdf(&std::env::current_dir()?)? {
          Some(path) => self.open(&path, false, screen),
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
      self.focus(target.0, target.1)
    } else {
      // A second middle press must still find its run.
      self.keeping_pointer(|session| session.focus(source.0, source.1))
    }
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
