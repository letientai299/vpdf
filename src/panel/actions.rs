use super::{
  Action, Annotation, App, Destination, Entry, Input, JumpDescription,
  MouseButton, MouseEvent, MouseEventKind, Panel, PanelKind, Position, Request,
  Result, Row, Shelf, Target, recent_rows,
};

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
        self.text_origin(position)?;
      }
      self.text_mode = true;
      // Presentation retains the reading mode.
      if let Some(panel) = &mut self.panel {
        panel.focused = false;
      }
      return self.relayout();
    }
    if kind == PanelKind::Outline {
      self.want_outline();
    }
    if self.panel.as_ref().is_some_and(|panel| panel.kind == kind) {
      if kind == PanelKind::Links {
        self.refill_panel(kind)?;
      }
      if !self.panel_focused() {
        self.blur()?;
      }
      if let Some(panel) = &mut self.panel {
        panel.focused = true;
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
    let mut panel = Panel::new(kind, rows);
    panel.note = self.panel_note(kind);
    panel.height = self.screen.rows;
    if kind == PanelKind::Outline {
      panel.select(self.section().unwrap_or(0));
    }
    self.panel = Some(panel);
    self.relayout()
  }

  /// Refills an open `kind` panel in place, keeping focus and the
  /// selected row; a menu for a row that left closes.
  pub fn refill_panel(&mut self, kind: PanelKind) -> Result<()> {
    if self.panel.as_ref().is_none_or(|panel| panel.kind != kind) {
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    let note = self.panel_note(kind);
    let Some(panel) = &mut self.panel else { return Ok(()) };
    let kept = panel.refill(rows, note);
    if !kept && self.context_menu.as_ref().is_some_and(|menu| menu.row) {
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
    let mut panel = Panel::new(PanelKind::Recent, recent_rows(entries));
    panel.shelf = Some(Shelf::new(entries, &self.password, &self.wake));
    panel.height = self.screen.rows;
    self.panel = Some(panel);
    self.relayout()
  }

  /// Whether the recent view covers the pane.
  pub fn covered(&self) -> bool {
    self
      .panel
      .as_ref()
      .is_some_and(|panel| panel.is_recent_view() && panel.focused)
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
    self.requested.clear();
    if self.reader.uses_cursor() {
      self.ensure_cursor()?;
    }
    Ok(())
  }

  pub fn panel_input(&mut self, input: Input) -> Result<bool> {
    let Some(panel) = &mut self.panel else { return Ok(false) };
    if !panel.focused {
      return Ok(false);
    }
    if panel.navigate(input, usize::from(self.screen.rows)) {
      return Ok(true);
    }
    let covered = panel.is_recent_view();
    match input.action {
      Action::Quit | Action::ClosePanel => {
        self.close_panel()?;
      }
      Action::Yank | Action::YankLine => {
        if let Some(text) = panel.yank_text() {
          self.reader.store(input.register, text)?;
        }
      }
      // The focused pane owns text motions and selections.
      Action::Motion(_)
      | Action::Visual(_)
      | Action::Object(..)
      | Action::Reselect
      | Action::WordSearch
      | Action::Definition
      | Action::Cancel => {}
      Action::Peek if panel.kind == PanelKind::Annotations => {
        if let Some(Destination::Annotation { position, annotation }) =
          panel.rows.get(panel.selected).map(|row| row.destination.clone())
        {
          self.preview_annotation(position, &annotation);
        }
      }
      Action::OpenUrl => {
        if let Some(Destination::Link(target @ Target::Uri(_))) =
          panel.rows.get(panel.selected).map(|row| row.destination.clone())
        {
          self.follow(&target)?;
        }
      }
      Action::Follow => self.panel_follow(Action::Follow)?,
      Action::Swap if !covered => self.panel_follow(Action::Swap)?,
      // Commands, searches, windows, and tabs still work; the covered
      // document takes nothing else.
      Action::Command
      | Action::Search(_)
      | Action::Window(_)
      | Action::Tab(_)
      | Action::Panel(_)
      | Action::Overview => {
        return Ok(false);
      }
      _ if covered => {}
      _ => return Ok(false),
    }
    Ok(true)
  }

  pub fn panel_mouse(&mut self, mouse: MouseEvent, clicks: u8) -> Result<()> {
    if matches!(mouse.kind, MouseEventKind::Down(_)) && !self.panel_focused() {
      self.blur()?;
    }
    let covered = self.covered();
    let clicked = self.panel.as_mut().and_then(|panel| panel.pointer(mouse));
    if self.covered() != covered {
      self.relayout()?;
    }
    if mouse.kind == MouseEventKind::Down(MouseButton::Right) {
      self.panel_context(mouse);
      return Ok(());
    }
    if mouse.kind == MouseEventKind::Down(MouseButton::Middle) {
      let panel = self.panel.as_ref();
      let row = panel.and_then(|panel| panel.row_at(mouse.column, mouse.row));
      return row.map_or(Ok(()), |index| self.panel_split(index));
    }
    let Some(panel) = &mut self.panel else { return Ok(()) };
    let pressed = matches!(mouse.kind, MouseEventKind::Down(_));
    let followed = if pressed {
      std::mem::replace(&mut panel.clicked, clicked)
    } else {
      panel.clicked
    };
    let Some(index) = clicked else { return Ok(()) };
    if covered {
      return self.panel_follow(Action::Follow);
    }
    if clicks > 1 && followed == Some(index) {
      return self.panel_refocus();
    }
    self.panel_follow(Action::Swap)
  }

  /// Enter for the row just followed.
  fn panel_refocus(&mut self) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(row) = panel.rows.get(panel.selected) else { return Ok(()) };
    match row.destination.clone() {
      // Reuses the view the first click opened.
      Destination::Mark(_) => self.panel_follow(Action::Follow),
      Destination::Annotation { position, annotation } => {
        self.focus_content(Some((position, annotation)))
      }
      Destination::None => Ok(()),
      _ => self.focus_content(None),
    }
  }

  /// Middle click: the row in a split.
  pub(crate) fn panel_split(&mut self, index: usize) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(row) = panel.rows.get(index) else { return Ok(()) };
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

  fn panel_follow(&mut self, action: Action) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(row) = panel.rows.get(panel.selected) else { return Ok(()) };
    let destination = row.destination.clone();
    let focus = action == Action::Follow;
    let search = panel.kind == PanelKind::Results;
    let heading = (panel.kind == PanelKind::Outline)
      .then(|| row.label.trim().to_owned())
      .filter(|title| !title.is_empty());
    let annotation = match destination {
      Destination::File(path) => {
        self.request = Some(Request::PanelOpen { path, position: None, focus });
        return Ok(());
      }
      Destination::Mark(name) => {
        self.request = Some(Request::PanelMark { name, focus });
        return Ok(());
      }
      Destination::None => return Ok(()),
      Destination::Position(position) => {
        self.land_at(position)?;
        self
          .reader
          .describe_jump(search.then(|| self.search_description(position)));
        None
      }
      Destination::Jump { position, description } => {
        self.land_at(position)?;
        self.reader.describe_jump(description);
        None
      }
      Destination::Link(target) => {
        self.follow(&target)?;
        if !matches!(target, Target::Uri(_))
          && let Some(title) = heading
        {
          self.reader.describe_jump(Some(JumpDescription::Heading(title)));
        }
        None
      }
      Destination::Annotation { position, annotation } => {
        self.land_at(position)?;
        Some((position, annotation))
      }
    };
    if self.overview.is_some() {
      self.close_overview();
    }
    if focus {
      self.focus_content(annotation)?;
    }
    Ok(())
  }

  /// Leaves the panel for content; notes open.
  fn focus_content(
    &mut self,
    annotation: Option<(Position, Annotation)>,
  ) -> Result<()> {
    self.reader.cancel();
    if annotation.is_some() {
      self.reader.enter_cursor();
    } else {
      self.reader.normal();
    }
    if let Some(panel) = &mut self.panel {
      panel.focused = false;
    }
    self.relayout()?;
    if let Some((position, annotation)) = annotation {
      self.preview_annotation(position, &annotation);
    }
    Ok(())
  }
}
