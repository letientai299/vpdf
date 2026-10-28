use super::{Alternate, App, Body, Pane, Result, Session};

impl Session {
  pub(super) fn alternate_reference(&mut self) -> Result<Option<Alternate>> {
    let Some(app) = self.focused_mut().document_mut() else { return Ok(None) };
    Ok(Some(Alternate {
      path: app.path.clone(),
      position: app.reading_position()?,
      password: app.password.clone(),
      manual: app.manual.clone(),
    }))
  }

  pub(super) fn show_recent(&mut self) -> Result<()> {
    if self.focused().is_recent() {
      return Ok(());
    }
    self.record();
    let alternate = self.alternate_reference()?;
    let mut pane = self.recent_pane(self.focused().screen);
    pane.alternate = alternate;
    self.replace_pane(pane)
  }

  /// Replaces the focused pane; its document becomes the alternate.
  pub(super) fn replace_keeping_alternate(
    &mut self,
    mut pane: Pane,
  ) -> Result<()> {
    let alternate = self.alternate_reference()?;
    pane.alternate = alternate.or_else(|| self.focused_mut().alternate.take());
    self.replace_pane(pane)
  }

  /// Shows Recent without an alternate.
  pub(super) fn close_to_recent(&mut self) -> Result<()> {
    self.show_recent()?;
    self.focused_mut().alternate = None;
    Ok(())
  }

  pub(super) fn replace_pane(&mut self, pane: Pane) -> Result<()> {
    self.leave()?;
    let tab = self.tab();
    let old = std::mem::replace(&mut tab.panes[tab.focus], pane);
    tab.tree.replace(old.id, tab.panes[tab.focus].id);
    self.retarget_help(old.id, self.focused().id);
    self.retire(&old);
    self.changed();
    drop(old);
    self.refocus();
    self.arrange()
  }

  pub(super) fn alternate(&mut self, delete: bool) -> Result<()> {
    let Some(alternate) = self.focused().alternate.as_ref() else {
      anyhow::bail!("No alternate document");
    };
    if !delete
      && alternate.manual.is_none()
      && let Some((tab, pane)) = self.existing(&alternate.path)
    {
      return self.focus(tab, pane);
    }
    let alternate = self.focused_mut().alternate.take();
    let Some(alternate) = alternate else { return Ok(()) };
    let current = if delete { None } else { self.alternate_reference()? };
    let screen = self.focused().screen;
    let mut app = match alternate.manual {
      Some(manual) => App::help_document(
        manual,
        screen,
        self.frontend.clone(),
        &self.settings,
      ),
      None => self.load_once(&alternate.path, &alternate.password, screen),
    }
    .map_err(|error| anyhow::anyhow!("Cannot reopen alternate: {error}"))?;
    if let Err(error) = app.land_at(alternate.position) {
      if app.manual.is_none() {
        self.watcher.remove(&app.path);
      }
      return Err(error);
    }
    self.reopen_sidebar(&mut app);
    let mut pane = Pane::from(app);
    pane.alternate = current;
    self.replace_pane(pane)?;
    self.remember_open(self.active, self.tabs[self.active].focus);
    self.revive_help();
    Ok(())
  }

  pub(super) fn delete_buffer(&mut self) -> Result<()> {
    match &self.focused().body {
      Body::Document(_) if self.focused().alternate.is_some() => {
        self.alternate(true)
      }
      Body::Recent(_) => self.close_pane(),
      Body::Document(app) if app.manual.is_some() => self.close_pane(),
      Body::Document(_) => self.close_to_recent(),
    }
  }

  pub(super) fn dismiss_recent(&mut self, quit: bool) -> Result<()> {
    if self.focused().alternate.is_some() {
      return self.alternate(false);
    }
    if quit { self.close_pane() } else { Ok(()) }
  }
}
