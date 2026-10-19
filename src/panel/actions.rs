use super::{
  App, Destination, Entry, Input, MouseEvent, MouseEventKind, PanelKind,
  PanelSession, Request, Result, Row, Target, recent_rows,
};
use vpdf_ui::panel::{Activation, Focus, Follow};

impl App {
  pub fn open_panel(&mut self, kind: PanelKind) -> Result<()> {
    if kind == PanelKind::Text {
      if self.panel_focused() {
        self.blur()?;
      }
      if self.overview.is_some() {
        self.close_overview();
      }
      if self.preview.is_none() {
        self.preview = Some(self.text_mode);
      }
      if !self.text_mode {
        let position =
          self.first_visible()?.unwrap_or_else(|| self.navigation_position());
        self.text().origin(position)?;
      }
      self.text_mode = true;
      // Presentation retains the reading mode.
      if let Some(panel) = &mut self.panel {
        panel.view.focus(false);
      }
      return self.relayout();
    }
    if kind == PanelKind::Outline {
      self.want_outline();
    }
    if self.panel.as_ref().is_some_and(|panel| panel.view.kind() == kind) {
      if kind == PanelKind::Links {
        self.refill_panel(kind)?;
      }
      if !self.panel_focused() {
        self.blur()?;
      }
      if let Some(panel) = &mut self.panel {
        panel.view.focus(true);
      }
      return self.relayout();
    }
    if matches!(kind, PanelKind::Recent | PanelKind::Marks) {
      self.request = Some(Request::Panel(kind));
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    self.show_panel(kind, rows)
  }

  pub fn show_panel(&mut self, kind: PanelKind, rows: Vec<Row>) -> Result<()> {
    self.blur()?;
    let mut panel = PanelSession::new(kind, rows);
    panel.view.set_note(self.panel_note(kind));
    panel.view.resize(self.screen.rows);
    if kind == PanelKind::Outline {
      panel.view.select(self.reading_view().current_section().unwrap_or(0));
    }
    self.panel = Some(panel);
    self.relayout()
  }

  /// Refills an open `kind` panel in place, keeping focus and the
  /// selected row; a menu for a row that left closes.
  pub fn refill_panel(&mut self, kind: PanelKind) -> Result<()> {
    if self.panel.as_ref().is_none_or(|panel| panel.view.kind() != kind) {
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    let note = self.panel_note(kind);
    let Some(panel) = &mut self.panel else { return Ok(()) };
    let kept = panel.view.refill(rows, note);
    if !kept
      && self
        .context_menu
        .as_ref()
        .is_some_and(vpdf_ui::context::ContextMenu::is_row)
    {
      self.context_menu = None;
    }
    Ok(())
  }

  pub(crate) fn panel_note(&self, kind: PanelKind) -> &'static str {
    if kind == PanelKind::Outline && self.doc.inferred {
      "inferred"
    } else {
      ""
    }
  }

  /// Covers the pane with `entries` and a card for the selected one,
  /// releasing what the hidden document rebuilds on return.
  pub fn show_recent(&mut self, entries: &[Entry]) -> Result<()> {
    self.blur()?;
    self.overview = None;
    if self.preview.is_some() {
      self.close_text_preview()?;
    }
    self.trim();
    let mut panel = PanelSession::recent(
      recent_rows(entries),
      entries,
      &self.password,
      &self.wake,
    );
    panel.view.resize(self.screen.rows);
    self.panel = Some(panel);
    self.relayout()
  }

  /// Whether the recent view covers the pane.
  pub fn covered(&self) -> bool {
    self.panel.as_ref().is_some_and(|panel| panel.view.covers_page())
  }

  pub fn close_panel(&mut self) -> Result<()> {
    if self.panel_focused() {
      self.blur()?;
    }
    let covered = self.covered();
    self.panel = None;
    self.relayout()?;
    // Changes while covered waited; settling picks them up.
    if covered {
      self.watch()?;
    }
    Ok(())
  }

  /// Returns from the text preview.
  pub fn close_text_preview(&mut self) -> Result<()> {
    if let Some(previous) = self.preview.take() {
      self.text_mode = previous;
    }
    self.tiles.invalidate();
    if self.reader.uses_cursor() {
      self.ensure_cursor()?;
    }
    Ok(())
  }

  pub fn panel_input(&mut self, input: Input) -> Result<bool> {
    let Some(panel) = &mut self.panel else { return Ok(false) };
    match panel.view.input(input, usize::from(self.screen.rows)) {
      vpdf_ui::panel::Event::Unhandled => return Ok(false),
      vpdf_ui::panel::Event::Stay => {}
      vpdf_ui::panel::Event::Close => self.close_panel()?,
      vpdf_ui::panel::Event::Copy { text, register } => {
        self.reader.store(register, text)?;
      }
      vpdf_ui::panel::Event::Preview { position, annotation } => {
        self.preview_annotation(position, &annotation);
      }
      vpdf_ui::panel::Event::OpenUri(uri) => self.follow(&Target::Uri(uri))?,
      vpdf_ui::panel::Event::Follow { focus } => {
        let follow = panel.view.follow(&self.search);
        self.follow_panel(follow, focus)?;
      }
    }
    Ok(true)
  }

  pub fn panel_mouse(&mut self, mouse: MouseEvent, clicks: u8) -> Result<()> {
    if matches!(mouse.kind, MouseEventKind::Down(_)) && !self.panel_focused() {
      self.blur()?;
    }
    let covered = self.covered();
    let press = self.panel.as_mut().map(|panel| panel.view.press(mouse));
    if self.covered() != covered {
      self.relayout()?;
    }
    let (Some(panel), Some(press)) = (&mut self.panel, press) else {
      return Ok(());
    };
    match press.activate(&mut panel.view, clicks) {
      Activation::Stay => Ok(()),
      Activation::Context => {
        self.panel_context(mouse);
        Ok(())
      }
      Activation::Split(index) => self.panel_split(index),
      Activation::Follow { focus } => {
        let follow = panel.view.follow(&self.search);
        self.follow_panel(follow, focus)
      }
      Activation::Refocus => {
        let follow = panel.view.refocus();
        self.follow_panel(follow, true)
      }
    }
  }

  /// Middle click: the row in a split.
  pub(crate) fn panel_split(&mut self, index: usize) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(row) = panel.view.rows().get(index) else { return Ok(()) };
    let request = match row.destination.clone() {
      Destination::Position(position)
      | Destination::Jump { position, .. }
      | Destination::Annotation { position, .. } => Request::SplitAt(position),
      Destination::Mark(name) => Request::SplitMark(name),
      Destination::File(path) => {
        Request::PanelOpen { path, position: None, focus: false }
      }
      Destination::Link(target) => return self.open_split(&target),
      Destination::None => return Ok(()),
    };
    self.request = Some(request);
    Ok(())
  }

  fn follow_panel(
    &mut self,
    follow: Option<Follow>,
    focus: bool,
  ) -> Result<()> {
    let Some(follow) = follow else { return Ok(()) };
    let content = match follow {
      Follow::File(path) => {
        self.request = Some(Request::PanelOpen { path, position: None, focus });
        return Ok(());
      }
      Follow::Mark(name) => {
        self.request = Some(Request::PanelMark { name, focus });
        return Ok(());
      }
      Follow::Focus(content) => return self.focus_content(content),
      Follow::Land { position, description } => {
        self.land_at(position)?;
        self.reader.describe_jump(description);
        Focus::Content
      }
      Follow::Link { target, description } => {
        self.follow(&target)?;
        if description.is_some() {
          self.reader.describe_jump(description);
        }
        Focus::Content
      }
      Follow::Annotation { position, annotation } => {
        self.land_at(position)?;
        Focus::Annotation { position, annotation }
      }
    };
    if self.overview.is_some() {
      self.close_overview();
    }
    if focus {
      self.focus_content(content)?;
    }
    Ok(())
  }

  /// Leaves the panel for content; notes open.
  fn focus_content(&mut self, focus: Focus) -> Result<()> {
    let annotation = focus.apply(&mut self.reader);
    if let Some(panel) = &mut self.panel {
      panel.view.focus(false);
    }
    self.relayout()?;
    if let Some((position, annotation)) = annotation {
      self.preview_annotation(position, &annotation);
    }
    Ok(())
  }
}
