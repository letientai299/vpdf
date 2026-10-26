//! The side panel: its rows, loaders, and actions.

use crate::Result;
use crate::app::{App, Request};
use crate::recent::Loader;
use crate::state::Entry;
use std::path::PathBuf;
use vpdf_model::document::Target;
use vpdf_model::geometry::At;
use vpdf_model::input::{MouseEvent, MouseEventKind};
use vpdf_terminal::terminal;
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::{Activation, Destination, Focus, Follow, Panel, Row};
use vpdf_ui::split::Direction;
use vpdf_ui::window::Window;

pub struct PanelSession {
  pub view: Panel,
  loader: Option<Loader>,
}

impl PanelSession {
  pub fn new(kind: PanelKind, rows: Vec<Row>) -> Self {
    Self { view: Panel::new(kind, rows), loader: None }
  }

  pub fn recent(
    rows: Vec<Row>,
    entries: &[Entry],
    password: &str,
    wake: &vpdf_pdf::jobs::Wake,
  ) -> Self {
    let view = Panel::recent(rows, &crate::recent::entries(entries));
    Self { view, loader: Some(Loader::new(password, wake)) }
  }

  pub fn updates(&mut self) -> bool {
    match (self.loader.as_ref(), self.view.shelf_mut()) {
      (Some(loader), Some(cards)) => loader.updates(cards),
      _ => false,
    }
  }

  pub(crate) fn scene<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
    at: At,
    width: u16,
    screen: Screen,
    pictures: bool,
  ) -> std::result::Result<(), S::Error> {
    let drawn = self.view.scene(
      scene,
      vpdf_ui::panel::recent::Frame {
        at,
        width,
        screen,
        now: crate::state::now(),
      },
      pictures,
    );
    if let (Some(loader), Some(cards)) = (&self.loader, self.view.shelf_mut()) {
      loader.request(cards);
    }
    drawn
  }
}

impl App {
  /// Follows section changes without interrupting panel browsing.
  pub fn track_outline(&mut self) {
    let previous = self.outline.current();
    let automatic =
      self.panel.as_ref().is_some_and(|panel| panel.view.auto_expand());
    {
      let mut reading = self.reading_parts();
      reading.outline.track(&mut reading.view, reading.panel);
    }
    if automatic
      && previous != self.outline.current()
      && self.overlays.menu().is_some_and(vpdf_ui::context::ContextMenu::is_row)
    {
      self.overlays.close_menu();
    }
  }

  pub fn panel_rows(&mut self, kind: PanelKind) -> Result<Vec<Row>> {
    let label = |target: &Target| match target {
      Target::Page(page) | Target::Point { page, .. } => {
        self.doc.get_label(*page).map_or_else(String::new, str::to_owned)
      }
      Target::Uri(_) => String::new(),
    };
    Ok(match kind {
      PanelKind::Outline => self
        .doc
        .outline
        .iter()
        .map(|item| Row::outline(item, label(&item.target)))
        .collect(),
      PanelKind::Results => self
        .search
        .hits()
        .iter()
        .map(|hit| Row::result(hit, self.doc.label(hit.first.page).to_owned()))
        .collect(),
      PanelKind::Figures => match &self.figures {
        crate::jobs::Figures::Ready(entries) => entries
          .iter()
          .enumerate()
          .map(|(index, (page, figure))| {
            Row::figure(
              figure,
              *page,
              index + 1,
              self.doc.label(*page).to_owned(),
            )
          })
          .collect(),
        _ => Vec::new(),
      },
      PanelKind::Annotations => self.sidebar_rows(kind),
      PanelKind::Jumps => self.jump_rows(),
      PanelKind::Version => crate::version::text(matches!(
        self.frontend,
        crate::raster::Frontend::Terminal
      ))
      .lines()
      .map(Row::parse)
      .collect(),
      PanelKind::Info => self.info().lines().map(Row::parse).collect(),
      PanelKind::Text | PanelKind::Recent | PanelKind::Marks => Vec::new(),
    })
  }

  /// PDF metadata, document facts, reader statistics, terminal caps.
  pub fn info(&self) -> String {
    let metadata = self.doc.metadata();
    let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
    let page = self.navigation_position().page;
    vpdf_ui::panel::Info {
      document: vpdf_ui::panel::DocumentInfo {
        metadata: &metadata,
        path: self.document_identity(),
        bytes,
        pages: self.doc.pages.len(),
        label: self.doc.label(page),
        bounds: self.doc.pages[page.0].bounds,
        headings: self.doc.outline.len(),
        inferred: self.doc.inferred,
        marks: self.marks.len(),
        cache_bytes: self.doc.cache_bytes(),
      },
      layout: &self.layout,
      search: &self.search,
      images: &[
        format!("Images: {} KiB resident", self.image_usage.bytes / 1024),
        format!("Transferred: {} KiB", self.image_usage.uploaded / 1024),
      ],
      render_ms: self.render_ms,
      screen: self.screen,
      text: self.text_mode,
      mouse: terminal::mouse(),
      frontend: &vpdf_terminal::caps::get()
        .map_or_else(|| "Terminal: not probed".into(), ToString::to_string),
    }
    .text()
  }
}

/// Recent documents that still exist, newest first.
pub fn recent_rows(entries: &[Entry]) -> Vec<Row> {
  history_rows(entries.iter().filter(|entry| entry.path.exists()))
}

pub(crate) fn saved_rows(entries: &[Entry]) -> Vec<Row> {
  history_rows(entries.iter())
}

fn history_rows<'a>(entries: impl Iterator<Item = &'a Entry>) -> Vec<Row> {
  let home = std::env::var_os("HOME").map(PathBuf::from);
  entries
    .map(|entry| Row::recent(&entry.path, entry.cursor.page, home.as_deref()))
    .collect()
}

impl App {
  pub fn open_panel(&mut self, kind: PanelKind) -> Result<()> {
    if kind == PanelKind::Text {
      if self.panel_focused() {
        self.blur()?;
      }
      if self.overlays.overview().is_some() {
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
    self.want_sidebar(kind);
    if kind == PanelKind::Outline {
      self.want_outline();
    }
    if kind == PanelKind::Figures {
      self.want_figures();
    }
    if self.panel.as_ref().is_some_and(|panel| panel.view.kind() == kind) {
      if matches!(kind, PanelKind::Figures | PanelKind::Annotations) {
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
      self.outline.sync(&self.doc.outline);
      {
        let mut reading = self.reading_parts();
        reading.outline.track(&mut reading.view, None);
      }
      panel.view.choose(self.outline.current().unwrap_or(0));
      self.outline.project(&mut panel.view);
      panel.view.reveal();
    }
    self.panel = Some(panel);
    self.relayout()
  }

  /// Outline refills invalidate source identities.
  pub fn refill_panel(&mut self, kind: PanelKind) -> Result<()> {
    if kind == PanelKind::Outline && !self.outline.sync(&self.doc.outline) {
      self.refresh_panel_note(kind);
      return Ok(());
    }
    if self.panel.as_ref().is_none_or(|panel| panel.view.kind() != kind) {
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    let note = self.panel_note(kind);
    let Some(panel) = &mut self.panel else { return Ok(()) };
    let kept = panel.view.refill(rows, note);
    if kind == PanelKind::Outline {
      self.outline.project(&mut panel.view);
    }
    if !kept
      && kind != PanelKind::Info
      && self.overlays.menu().is_some_and(vpdf_ui::context::ContextMenu::is_row)
    {
      self.overlays.close_menu();
    }
    Ok(())
  }

  pub(crate) fn refresh_panel_note(&mut self, kind: PanelKind) {
    let note = self.panel_note(kind);
    if let Some(panel) = &mut self.panel
      && panel.view.kind() == kind
    {
      panel.view.set_note(note);
    }
  }

  pub(crate) fn panel_note(&self, kind: PanelKind) -> &'static str {
    if self.sidebar_loading(kind)
      || kind == PanelKind::Outline && self.outline_pending()
      || kind == PanelKind::Figures
        && matches!(self.figures, crate::jobs::Figures::Running(_))
    {
      "loading"
    } else if kind == PanelKind::Outline && self.doc.inferred {
      "inferred"
    } else {
      ""
    }
  }

  /// Covers the pane with `entries` and a card for the selected one,
  /// releasing what the hidden document rebuilds on return.
  pub fn show_recent(&mut self, entries: &[Entry]) -> Result<()> {
    self.blur()?;
    self.overlays.close_overview();
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
    panel.view.set_picker(match self.frontend {
      crate::raster::Frontend::Terminal => {
        vpdf_ui::home::Picker::Terminal { fzf: crate::cmdline::on_path("fzf") }
      }
      crate::raster::Frontend::Native(_) => vpdf_ui::home::Picker::Native,
    });
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

  pub(crate) fn confirming_recent(&self) -> bool {
    self.panel.as_ref().is_some_and(|panel| panel.view.confirming())
  }

  pub(crate) fn confirm_recent(&mut self, key: vpdf_model::input::KeyEvent) {
    if let Some(panel) = &mut self.panel
      && let vpdf_ui::panel::Event::Forget(paths) = panel.view.confirm_key(key)
    {
      self.request = Some(Request::ForgetRecent(paths));
    }
  }

  pub fn panel_input(&mut self, input: Input) -> Result<bool> {
    if let Action::Window(Window::Extend(direction)) = input.action
      && self.panel_focused()
      && !self.covered()
    {
      let cells = u16::try_from(input.count).unwrap_or(u16::MAX);
      let width = match direction {
        Direction::Left => self.left.saturating_sub(cells),
        Direction::Right => self.left.saturating_add(cells),
        Direction::Up | Direction::Down => return Ok(true),
      };
      self.resize_panel(width)?;
      return Ok(true);
    }
    let Some(panel) = &mut self.panel else { return Ok(false) };
    if panel.view.focused()
      && panel.view.kind() == PanelKind::Outline
      && let Action::Result(reverse) = input.action
    {
      self.outline.repeat_search(&mut panel.view, reverse, input.count);
      self.overlays.close_menu();
      return Ok(true);
    }
    match panel.view.input(input, usize::from(self.screen.rows)) {
      vpdf_ui::panel::Event::Unhandled => return Ok(false),
      vpdf_ui::panel::Event::Forget(paths) => {
        self.request = Some(Request::ForgetRecent(paths));
      }
      vpdf_ui::panel::Event::Fold { operation, count } => {
        self.outline.fold(&mut panel.view, operation, count);
        self.overlays.close_menu();
      }
      vpdf_ui::panel::Event::AutoExpand => {
        self.outline.toggle_auto(&mut panel.view);
        self.overlays.close_menu();
      }
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
      Activation::Control(control) => {
        if let Some(action) = panel.view.control_action(control) {
          self.panel_input(Input::plain(action))?;
        }
        Ok(())
      }
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
    if self.overlays.overview().is_some() {
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
