//! The side panel: its rows, loaders, and actions.

use crate::Result;
use crate::app::{App, Request};
use crate::recent::Loader;
use crate::state::Entry;
use std::io::Write;
use std::path::PathBuf;
use vpdf_model::document::Target;
use vpdf_model::geometry::At;
use vpdf_model::input::{MouseEvent, MouseEventKind};
use vpdf_terminal::{
  graphics::Graphics,
  terminal::{self},
};
use vpdf_ui::keys::{Input, PanelKind};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::{
  Activation, Destination, Focus, Follow, Panel, Row, help,
};

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

  pub fn draw(
    &mut self,
    out: &mut impl Write,
    at: At,
    width: u16,
    screen: Screen,
    graphics: Option<&mut Graphics>,
  ) -> Result<()> {
    let pictures = graphics.is_some();
    crate::presentation::draw(out, graphics, |scene| {
      self.scene(scene, at, width, screen, pictures)
    })?
  }
}

impl App {
  /// Follows section changes without interrupting panel browsing.
  pub fn track_outline(&mut self) {
    let mut reading = self.reading_parts();
    reading.outline.track(&mut reading.view, reading.panel);
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
      PanelKind::Links => self.link_rows()?,
      PanelKind::Jumps => self.jump_rows(),
      PanelKind::Annotations => self.annotation_rows()?,
      PanelKind::Help => help(|program| self.command_available(program))
        .iter()
        .map(|line| Row::parse(line))
        .collect(),
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
        path: &self.path,
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
  let home = std::env::var_os("HOME").map(PathBuf::from);
  entries
    .iter()
    .filter(|entry| entry.path.exists())
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
    if kind == PanelKind::Outline {
      self.want_outline();
    }
    if kind == PanelKind::Figures {
      self.want_figures();
    }
    if self.panel.as_ref().is_some_and(|panel| panel.view.kind() == kind) {
      if matches!(kind, PanelKind::Links | PanelKind::Figures) {
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
      && kind != PanelKind::Info
      && self.overlays.menu().is_some_and(vpdf_ui::context::ContextMenu::is_row)
    {
      self.overlays.close_menu();
    }
    Ok(())
  }

  pub(crate) fn panel_note(&self, kind: PanelKind) -> &'static str {
    if kind == PanelKind::Figures
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
