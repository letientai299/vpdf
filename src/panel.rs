//! The side panel: its rows, loaders, and actions.

use crate::Result;
use crate::app::{App, Request};
use crate::state::Entry;
use crate::state::Mark;
use std::collections::BTreeMap;
use std::path::PathBuf;
use vpdf_model::document::Target;
use vpdf_model::geometry::At;
use vpdf_model::input::{MouseEvent, MouseEventKind};
use vpdf_terminal::terminal;
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::{
  Activation, Destination, Focus, Follow, Panel, Row, RowKind,
};
use vpdf_ui::split::Direction;
use vpdf_ui::window::Window;

pub struct PanelSession {
  pub view: Panel,
}

impl PanelSession {
  pub fn new(kind: PanelKind, rows: Vec<Row>) -> Self {
    Self { view: Panel::new(kind, rows) }
  }

  pub(crate) fn scene<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
    at: At,
    width: u16,
    screen: Screen,
    pictures: bool,
  ) -> std::result::Result<(), S::Error> {
    self.view.scene(
      scene,
      vpdf_ui::panel::recent::Frame {
        at,
        width,
        screen,
        now: crate::state::now(),
      },
      pictures,
    )
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

  /// Marks the first figure on the page in view, else the nearest one
  /// before it.
  pub fn track_figures(&mut self) {
    let page = self.navigation_position().page;
    let (Some(panel), crate::jobs::Figures::Ready(entries)) =
      (&mut self.panel, &self.figures)
    else {
      return;
    };
    if panel.view.kind() != PanelKind::Figures {
      return;
    }
    let rows = panel.view.rows();
    let figure = |index: &usize| {
      rows.get(*index).is_some_and(|row| row.kind != RowKind::Mention)
    };
    let after = entries.partition_point(|(at, _)| *at < page);
    let mut on_page =
      (after..entries.len()).take_while(|&at| entries[at].0 == page);
    let current =
      on_page.find(figure).or_else(|| (0..after).rev().find(figure));
    panel.view.mark_current(current);
    let label = self.doc.label(page);
    let place = format!("{}/{}", Row::page(label), self.doc.pages.len());
    panel.view.set_place(&place);
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
      PanelKind::Results => {
        Row::results(self.search.hits(), |page| self.doc.label(page).to_owned())
      }
      PanelKind::Figures => match &self.figures {
        crate::jobs::Figures::Ready(entries) => entries
          .iter()
          .enumerate()
          .map(|(index, (page, figure))| {
            Row::figure(
              figure,
              *page,
              index + 1,
              Row::page(self.doc.label(*page)),
            )
          })
          .collect(),
        _ => Vec::new(),
      },
      PanelKind::Annotations => {
        let rows = self.annotation_rows().into_iter().filter_map(|row| {
          let Destination::Annotation { position, .. } = &row.destination
          else {
            return None;
          };
          Some((position.page, row))
        });
        Row::paged(rows, |page| self.doc.label(page).to_owned())
      }
      PanelKind::Jumps => self.jump_rows(),
      PanelKind::Version => Row::sections(&crate::version::panel_text(
        matches!(self.frontend, crate::raster::Frontend::Terminal),
      )),
      PanelKind::Info => Row::sections(&self.info()),
      PanelKind::Text | PanelKind::Recent | PanelKind::Marks => Vec::new(),
    })
  }

  /// PDF metadata, document facts, reader statistics, terminal caps.
  pub fn info(&self) -> String {
    let metadata = self.doc.metadata();
    let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let page = self.navigation_position().page;
    vpdf_ui::panel::Info {
      document: vpdf_ui::panel::DocumentInfo {
        metadata: &metadata,
        path: self.document_identity(),
        home: home.as_deref(),
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
  let mut entries: Vec<_> = entries.collect();
  entries.sort_by_key(|entry| std::cmp::Reverse(entry.used));
  let mut groups: Vec<(PathBuf, Vec<Row>)> = Vec::new();
  for entry in entries {
    let parent = entry.path.parent().unwrap_or(&entry.path).to_path_buf();
    let index = groups
      .iter()
      .position(|(path, _)| *path == parent)
      .unwrap_or_else(|| {
        groups.push((parent, Vec::new()));
        groups.len() - 1
      });
    let mut row = Row::recent(&entry.path, entry.cursor.page, home.as_deref());
    row.label =
      entry.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    row.depth = 1;
    row.style = vpdf_ui::panel::Style::Path;
    groups[index].1.push(row);
  }
  let mut rows = Vec::new();
  for (parent, files) in groups {
    let label = home
      .as_deref()
      .and_then(|home| parent.strip_prefix(home).ok())
      .map_or_else(
        || parent.display().to_string(),
        |rest| PathBuf::from("~").join(rest).display().to_string(),
      );
    rows.push(Row {
      style: vpdf_ui::panel::Style::Heading,
      ..Row::new(
        label,
        format!("{} files", files.len()),
        Destination::Directory(parent),
      )
    });
    rows.extend(files);
  }
  rows
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
    if kind == PanelKind::Annotations {
      self.want_annotations();
    }
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
    let note = self.panel_note(kind);
    let topic = self.panel_topic(kind);
    // Groups form once the panel has a height to reveal in.
    let (rows, grouped) =
      if kind.grouped() { (Vec::new(), Some(rows)) } else { (rows, None) };
    let mut panel = PanelSession::new(kind, rows);
    panel.view.resize(self.screen.rows);
    if let Some(rows) = grouped {
      panel.view.regroup(rows, note, topic, self.landed(kind));
    } else {
      panel.view.set_note(note);
      panel.view.set_topic(topic);
    }
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
    if self.panel_kind() != Some(kind) {
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    // Info refills each frame; regrouping would undo a row search.
    let same = |panel: &PanelSession| panel.view.rows() == rows.as_slice();
    if kind == PanelKind::Info && self.panel.as_ref().is_some_and(same) {
      return Ok(());
    }
    self.refill_rows(kind, rows);
    Ok(())
  }

  /// Gives the open `kind` panel `rows`, keeping groups and selection.
  pub(crate) fn refill_rows(&mut self, kind: PanelKind, rows: Vec<Row>) {
    let note = self.panel_note(kind);
    let topic = self.panel_topic(kind).to_owned();
    let landed = self.landed(kind);
    let Some(panel) = &mut self.panel else { return };
    let kept = if kind.grouped() {
      panel.view.regroup(rows, note, &topic, landed)
    } else {
      let kept = panel.view.refill(rows, note);
      panel.view.set_topic(&topic);
      kept
    };
    if kind == PanelKind::Outline {
      self.outline.project(&mut panel.view);
    }
    if !kept
      && kind != PanelKind::Info
      && self.overlays.menu().is_some_and(vpdf_ui::context::ContextMenu::is_row)
    {
      self.overlays.close_menu();
    }
  }

  /// The search hit the reader landed on, for Results.
  fn landed(&self, kind: PanelKind) -> Option<vpdf_model::text::Position> {
    self.search.landed().filter(|_| kind == PanelKind::Results)
  }

  fn panel_kind(&self) -> Option<PanelKind> {
    self.panel.as_ref().map(|panel| panel.view.kind())
  }

  /// Marks the landed hit in an open Results panel.
  pub(crate) fn mark_landed(&mut self) {
    let landed = self.search.landed();
    if let Some(panel) = &mut self.panel
      && panel.view.kind() == PanelKind::Results
    {
      panel.view.mark_landed(landed);
    }
  }

  pub(crate) fn refresh_panel_note(&mut self, kind: PanelKind) {
    let note = self.panel_note(kind);
    let topic = self.panel_topic(kind).to_owned();
    if let Some(panel) = &mut self.panel
      && panel.view.kind() == kind
    {
      panel.view.set_note(note);
      panel.view.set_topic(&topic);
    }
  }

  pub(crate) fn panel_note(&self, kind: PanelKind) -> &'static str {
    if kind == PanelKind::Annotations && self.annotations_loading()
      || kind == PanelKind::Outline && self.outline_pending()
      || kind == PanelKind::Figures
        && matches!(self.figures, crate::jobs::Figures::Running(_))
      || kind == PanelKind::Results && self.search.phase().running()
    {
      "loading"
    } else if kind == PanelKind::Outline && self.doc.inferred {
      "inferred"
    } else {
      ""
    }
  }

  /// What the panel's rows answer: the search query for Results.
  fn panel_topic(&self, kind: PanelKind) -> &str {
    if kind == PanelKind::Results { &self.search.query().text } else { "" }
  }

  pub fn close_panel(&mut self) -> Result<()> {
    if self.panel_focused() {
      self.blur()?;
    }
    self.panel = None;
    self.relayout()?;
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

  pub(crate) fn confirming_removal(&self) -> bool {
    self.panel.as_ref().is_some_and(|panel| panel.view.confirming())
  }

  pub(crate) fn confirm_removal(&mut self, key: vpdf_model::input::KeyEvent) {
    if let Some(panel) = &mut self.panel {
      let event = panel.view.confirm_key(key);
      self.request_removal(event);
    }
  }

  /// A press on the open confirmation, in screen cells.
  pub(crate) fn confirm_click(&mut self, mut mouse: MouseEvent) {
    let Some(panel) = &mut self.panel else { return };
    let (Some(column), Some(row)) = (
      mouse.column.checked_sub(self.at.column),
      mouse.row.checked_sub(self.at.row),
    ) else {
      return;
    };
    (mouse.column, mouse.row) = (column, row);
    let event = panel.view.confirm_press(mouse);
    self.request_removal(event);
  }

  fn request_removal(&mut self, event: vpdf_ui::panel::Event) {
    self.request = match event {
      vpdf_ui::panel::Event::Forget(paths) => {
        Some(Request::ForgetRecent(paths))
      }
      vpdf_ui::panel::Event::RemoveMark(name) => {
        Some(Request::RemoveMark(name))
      }
      _ => return,
    };
  }

  /// Rebuilds an open Marks panel, keeping its folds.
  pub(crate) fn refill_marks(&mut self, global: &BTreeMap<char, Mark>) {
    if self.panel_kind() != Some(PanelKind::Marks) {
      return;
    }
    let rows = self.mark_rows(global);
    self.refill_rows(PanelKind::Marks, rows);
  }

  pub fn panel_input(&mut self, input: Input) -> Result<bool> {
    if let Action::Window(Window::Extend(direction)) = input.action
      && self.panel_focused()
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
      event @ (vpdf_ui::panel::Event::Forget(_)
      | vpdf_ui::panel::Event::RemoveMark(_)) => self.request_removal(event),
      vpdf_ui::panel::Event::Fold { operation, count } => {
        if !panel.view.fold(operation, count) {
          self.outline.fold(&mut panel.view, operation, count);
        }
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
    let press = self.panel.as_mut().map(|panel| panel.view.press(mouse));
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
      Destination::None | Destination::Directory(_) => return Ok(()),
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
      // Landing on the Jumps `now` row would record it.
      Follow::Land { .. } if self.selects_now() => {
        self.land_here()?;
        Focus::Content
      }
      Follow::Land { position, description } => {
        self.land_at(position)?;
        self.reader.describe_jump(description);
        if self.panel_kind() == Some(PanelKind::Results) {
          self.search.land(position);
          self.mark_landed();
        }
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

#[cfg(test)]
mod recent_tests {
  use super::*;

  #[test]
  fn groups_keep_paths_and_recency() {
    let entries = [
      crate::state::tests::entry("/one/a.pdf", 2, 30),
      crate::state::tests::entry("/two/a.pdf", 3, 20),
      crate::state::tests::entry("/one/日本 語.pdf", 4, 10),
    ];
    let rows = history_rows(entries.iter());
    let targets: Vec<_> =
      rows.iter().map(|row| row.destination.clone()).collect();
    assert!(
      targets
        == vec![
          Destination::Directory("/one".into()),
          Destination::File("/one/a.pdf".into()),
          Destination::File("/one/日本 語.pdf".into()),
          Destination::Directory("/two".into()),
          Destination::File("/two/a.pdf".into()),
        ]
    );
    assert_eq!(rows[1].label, "a.pdf");
    assert_eq!(rows[2].label, "日本 語.pdf");
  }
}

#[cfg(test)]
mod recent_path_tests {
  use super::*;
  use std::os::unix::ffi::OsStringExt;

  #[test]
  fn display_preserves_native_identity() {
    let path = PathBuf::from(std::ffi::OsString::from_vec(
      b"/books/file\xff.pdf".to_vec(),
    ));
    let mut entry = crate::state::tests::entry("placeholder.pdf", 0, 0);
    entry.path = path.clone();
    let rows = history_rows(std::iter::once(&entry));
    assert!(rows[1].destination == Destination::File(path));
    assert_eq!(rows[1].label, "file�.pdf");
  }
}
