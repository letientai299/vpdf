use crate::Result;
use crate::app::{App, Request};
use crate::document::{Annotation, Heading, Target};
use crate::graphics::Graphics;
use crate::keys::{Action, Input, PanelKind};
use crate::layout::Screen;
use crate::motion::Motion;
use crate::panel_layout::Layout;
use crate::panel_search::RowSearch;
use crate::pointer::{WHEEL_ROWS, Wheel};
use crate::reader::JumpDescription;
use crate::recent::Shelf;
use crate::state::Entry;
use crate::terminal;
use crate::text::{PageId, Position};
use crate::theme::{self, Color};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::io::Write;
use std::path::{Path, PathBuf};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone)]
pub enum Destination {
  Position(Position),
  Jump {
    position: Position,
    description: Option<JumpDescription>,
  },
  Annotation {
    position: Position,
    annotation: Annotation,
  },
  Link(Target),
  File(PathBuf),
  /// A global mark, resolved by the session.
  Mark(char),
  None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Style {
  Plain,
  Muted,
  Heading,
  /// `keys: description`, with the keys highlighted.
  Key,
  ModeKey,
  CommandKey,
  /// `parent/file`, with the file name highlighted.
  Path,
}

pub struct Row {
  pub label: String,
  pub detail: String,
  pub style: Style,
  pub destination: Destination,
}

impl Row {
  pub fn new(label: String, detail: String, destination: Destination) -> Self {
    Self { label, detail, style: Style::Plain, destination }
  }

  /// Reads `# heading` and `keys: description` lines.
  pub fn parse(line: &str) -> Self {
    let style = if line.starts_with("# ") {
      Style::Heading
    } else if line.contains(": ") {
      Style::Key
    } else {
      Style::Plain
    };
    let label = line.strip_prefix("# ").unwrap_or(line).to_owned();
    Self { label, detail: String::new(), style, destination: Destination::None }
  }
}

/// Narrowest dragged panel.
const MIN_WIDTH: u16 = 8;

pub struct Panel {
  pub kind: PanelKind,
  pub rows: Vec<Row>,
  pub selected: usize,
  pub focused: bool,
  /// First visible row; moves only to keep the selection shown.
  pub top: usize,
  pub search: RowSearch,
  /// Shown after the row count, such as "inferred".
  pub note: &'static str,
  /// Columns the last frame drew; presses beyond them miss the rows.
  columns: u16,
  height: u16,
  layout: Option<Layout>,
  follow_pending: bool,
  /// Cards for recent documents; a reader pane shows them over its page.
  pub shelf: Option<Shelf>,
  /// What the pointer rests on.
  pub(crate) pointing: Pointing,
  /// The hovered row's label when drawn clipped.
  pub(crate) hover_title: Option<String>,
  /// The row a click just followed.
  pub(crate) clicked: Option<usize>,
}

/// Where the pointer rests in a panel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Pointing {
  #[default]
  Away,
  Row(usize),
  /// The recent card.
  Card,
}

impl Pointing {
  pub fn row(self) -> Option<usize> {
    match self {
      Self::Row(index) => Some(index),
      Self::Away | Self::Card => None,
    }
  }
}

impl Panel {
  /// `index`'s line on screen, as scrolled.
  pub(crate) fn screen_line(&self, index: usize) -> isize {
    self.line_start(index) as isize - self.top as isize
  }

  pub fn new(kind: PanelKind, rows: Vec<Row>) -> Self {
    Self {
      kind,
      rows,
      selected: 0,
      focused: true,
      top: 0,
      search: RowSearch::default(),
      note: "",
      columns: u16::MAX,
      height: 0,
      layout: None,
      follow_pending: false,
      shelf: None,
      pointing: Pointing::Away,
      hover_title: None,
      clicked: None,
    }
  }

  /// Replaces the rows and note, keeping focus and the selection in
  /// range.
  pub fn refill(&mut self, rows: Vec<Row>, note: &'static str) {
    self.selected = self.selected.min(rows.len().saturating_sub(1));
    if self.top >= rows.len() {
      self.top = 0;
    }
    self.search.clamp(rows.len());
    self.layout = None;
    self.rows = rows;
    self.note = note;
  }

  /// Applies list motions; `page` rows per full-screen scroll.
  pub fn navigate(&mut self, input: Input, page: usize) -> bool {
    let last = self.rows.len().saturating_sub(1);
    let count = input.count;
    if self.kind == PanelKind::Help && self.layout.is_some() {
      let scroll = match input.action {
        Action::Scroll(amount) => {
          Some((amount * page as f32 * count as f32) as isize)
        }
        Action::Rows(rows) => Some(rows as isize * count as isize),
        _ => None,
      };
      if let Some(rows) = scroll {
        self.scroll_lines(rows, page);
        return true;
      }
    }
    self.selected = match input.action {
      Action::Motion(Motion::Down) => self.selected.saturating_add(count),
      Action::Motion(Motion::Up) => self.selected.saturating_sub(count),
      Action::Motion(Motion::First) => 0,
      Action::Motion(Motion::Last) => last,
      Action::Scroll(amount) => {
        let rows = (amount * page as f32 * count as f32) as isize;
        self.selected.saturating_add_signed(rows)
      }
      Action::Rows(rows) => {
        self.selected.saturating_add_signed(rows as isize * count as isize)
      }
      Action::Motion(Motion::NextParagraph | Motion::PreviousParagraph) => {
        let forward = input.action == Action::Motion(Motion::NextParagraph);
        self.heading(forward, count);
        self.selected
      }
      Action::Align(fraction) => {
        let visible = page.saturating_sub(3) as f32;
        self.top = self
          .line_start(self.selected)
          .saturating_sub((visible * fraction) as usize);
        self.selected
      }
      Action::Result(reverse) => {
        self.repeat(reverse, count);
        self.selected
      }
      _ => return false,
    }
    .min(last);
    self.height = u16::try_from(page).unwrap_or(u16::MAX);
    self.reveal();
    true
  }

  /// Returns a clicked row; only presses focus.
  pub fn pointer(&mut self, mouse: MouseEvent) -> Option<usize> {
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      self.focused = true;
    }
    let card = self.shelf.as_mut().filter(|shelf| shelf.covers(mouse.column));
    if let (Some(shelf), Some(turn)) = (card, Wheel::of(mouse.kind)) {
      let rows = isize::from(WHEEL_ROWS);
      match turn {
        Wheel::Down => shelf.scroll(rows),
        Wheel::Up => shelf.scroll(-rows),
        _ => {}
      }
      return None;
    }
    match mouse.kind {
      kind if Wheel::of(kind) == Some(Wheel::Down) => {
        let visible = usize::from(self.height.saturating_sub(2));
        self.top = self
          .top
          .saturating_add(usize::from(WHEEL_ROWS))
          .min(self.line_count().saturating_sub(visible.max(1)));
      }
      kind if Wheel::of(kind) == Some(Wheel::Up) => {
        self.top = self.top.saturating_sub(usize::from(WHEEL_ROWS));
      }
      MouseEventKind::Down(MouseButton::Left) => {
        let index = self.row_at(mouse.column, mouse.row)?;
        self.selected = index;
        return Some(index);
      }
      _ => {}
    }
    None
  }

  /// The row at a panel cell.
  pub fn row_at(&self, column: u16, row: u16) -> Option<usize> {
    if row == 0 || column + 1 >= self.columns {
      return None;
    }
    if self.layout.is_some() && row >= self.height.saturating_sub(1) {
      return None;
    }
    let line = self.top + usize::from(row) - 1;
    if line >= self.line_count() {
      return None;
    }
    Some(self.line_source(line)).filter(|&index| index < self.rows.len())
  }

  pub(crate) fn reveal(&mut self) {
    let count = usize::from(self.height.saturating_sub(2));
    let start = self.line_start(self.selected);
    let end = self.line_start(self.selected + 1);
    if start < self.top || end.saturating_sub(start) > count {
      self.top = start;
    } else if count > 0 && end > self.top + count {
      self.top = end - count;
    }
  }

  fn scroll_lines(&mut self, rows: isize, page: usize) {
    self.height = u16::try_from(page).unwrap_or(u16::MAX);
    let visible = page.saturating_sub(2).max(1);
    let offset =
      self.line_start(self.selected).saturating_sub(self.top).min(visible - 1);
    self.top = self
      .top
      .saturating_add_signed(rows)
      .min(self.line_count().saturating_sub(visible));
    self.selected = self.line_source(self.top + offset);
  }

  fn line_count(&self) -> usize {
    self.layout.as_ref().map_or(self.rows.len(), |help| help.lines.len())
  }

  fn line_source(&self, line: usize) -> usize {
    self.layout.as_ref().map_or(line, |help| {
      help.lines.get(line).map_or(self.rows.len(), |line| line.source)
    })
  }

  fn line_start(&self, row: usize) -> usize {
    self.layout.as_ref().map_or(row, |help| {
      help.starts.get(row).copied().unwrap_or(help.lines.len())
    })
  }

  fn reflow(&mut self, columns: usize) {
    if matches!(self.kind, PanelKind::Help)
      && self.layout.as_ref().is_some_and(|layout| layout.columns == columns)
      || !matches!(
        self.kind,
        PanelKind::Help | PanelKind::Links | PanelKind::Jumps
      )
    {
      return;
    }
    let source = self.line_source(self.top);
    let offset = self.top.saturating_sub(self.line_start(source));
    let selected = self.line_start(self.selected);
    let visible = usize::from(self.height.saturating_sub(2));
    let shown = (self.top..self.top + visible).contains(&selected);
    self.layout = Some(match self.kind {
      PanelKind::Links => Layout::links(&self.rows, columns, self.selected),
      PanelKind::Jumps => Layout::jumps(&self.rows, columns, self.selected),
      _ => Layout::help(&self.rows, columns),
    });
    let start = self.line_start(source);
    let length = self.line_start(source + 1).saturating_sub(start);
    self.top = start + offset.min(length.saturating_sub(1));
    if shown {
      self.reveal();
    }
  }

  /// Panel columns: dragged, else by kind.
  pub fn width(kind: PanelKind, columns: u16, dragged: Option<u16>) -> u16 {
    let wide = matches!(kind, PanelKind::Help | PanelKind::Info);
    let width = dragged.map_or_else(
      || {
        let width = if wide { columns / 2 } else { columns / 3 };
        width.clamp(1, if wide { 64 } else { 50 })
      },
      |dragged| dragged.max(MIN_WIDTH),
    );
    width.min(columns.saturating_sub(12))
  }

  /// Draws `width` columns of `screen`, with the selected document's
  /// card when the panel has a shelf; `graphics` places its thumbnail.
  pub fn draw(
    &mut self,
    out: &mut impl Write,
    at: terminal::At,
    width: u16,
    screen: Screen,
    graphics: Option<&mut Graphics>,
  ) -> Result<()> {
    let Some(mut cards) = self.shelf.take() else {
      return self.list(out, at, width, screen.rows);
    };
    if width < screen.columns || !self.focused {
      cards.hide();
      self.shelf = Some(cards);
      return self.list(out, at, width, screen.rows);
    }
    let drawn = cards.draw(self, out, at, width, screen, graphics);
    self.shelf = Some(cards);
    drawn
  }

  /// Draws the rows in `width` columns; the last is the border.
  pub fn list(
    &mut self,
    out: &mut impl Write,
    at: terminal::At,
    width: u16,
    height: u16,
  ) -> Result<()> {
    self.columns = width;
    self.height = height;
    self.hover_title = None;
    if width < 2 {
      return Ok(());
    }
    let inner = usize::from(width.saturating_sub(1));
    for row in 1..height {
      let border = at.goto(row - 1, width - 1);
      write!(out, "{border}{}│{}", theme::MUTED, theme::RESET)?;
    }
    let title = format!(" {} ", self.kind.title());
    let count = format!(" {} {}", self.rows.len(), self.note);
    write!(
      out,
      "{}{}{}{}{}{}",
      at.goto(0u16, 0u16),
      theme::badge(Color::Cyan),
      terminal::clean(&title, inner),
      theme::MUTED,
      terminal::clean(&count, inner.saturating_sub(title.width())),
      theme::RESET
    )?;
    self.reflow(inner);
    let keys = key_width(&self.rows, inner);
    let count = height.saturating_sub(2) as usize;
    self.top = self.top.min(self.line_count().saturating_sub(count.max(1)));
    let start = self.top;
    for (row, index) in (start..self.line_count()).take(count).enumerate() {
      let (source, item, keys, cut) = self.layout.as_ref().map_or_else(
        || (index, &self.rows[index], keys, None),
        |help| {
          let line = &help.lines[index];
          // The row's label line, even scrolled off.
          let label = &help.lines[help.starts[line.source]];
          (line.source, &line.row, line.keys, Some(label.clipped))
        },
      );
      let hovered = self.pointing.row() == Some(source);
      let selected = source == self.selected;
      let plain = match (selected, self.focused) {
        (true, true) => theme::SELECTED,
        (true, false) => theme::CURRENT,
        (false, true) => theme::RESET,
        (false, false) => theme::DIM,
      };
      let base = match (hovered && !selected, self.focused) {
        (true, true) => theme::HOVER,
        (true, false) => theme::HOVER_DIM,
        (false, _) => plain,
      };
      write!(out, "{}{base}", at.goto(row + 1, 0u16))?;
      let detail = terminal::clean(&item.detail, inner / 3);
      let room = if detail.is_empty() {
        inner
      } else {
        inner.saturating_sub(detail.width() + 1)
      };
      let used = item.write(out, base, room, keys, &self.search.query)?;
      if hovered {
        let full = |item: &Row| {
          item.write(&mut std::io::sink(), base, usize::MAX, keys, "")
        };
        let clipped = match cut {
          Some(clipped) => clipped,
          None => full(item)? > used,
        };
        if clipped {
          self.hover_title = Some(self.rows[source].label.clone());
        }
      }
      write!(
        out,
        "{plain}{}{}{detail}{}",
        " ".repeat(inner.saturating_sub(used + detail.width())),
        if plain == theme::RESET { theme::MUTED } else { "" },
        theme::RESET
      )?;
    }
    Ok(())
  }
}

/// Widest `keys:` column of `rows`, at most half of `inner`.
pub fn key_width(rows: &[Row], inner: usize) -> usize {
  rows
    .iter()
    .filter(|row| {
      matches!(row.style, Style::Key | Style::ModeKey | Style::CommandKey)
    })
    .filter_map(|row| row.label.split_once(": "))
    .map(|(key, _)| key.width())
    .max()
    .unwrap_or(0)
    .min(inner / 2)
}

impl Row {
  /// Writes the label within `room` columns and returns its width.
  pub fn write(
    &self,
    out: &mut impl Write,
    base: &str,
    room: usize,
    keys: usize,
    query: &str,
  ) -> Result<usize> {
    let plain = [theme::RESET, theme::DIM, theme::HOVER, theme::HOVER_DIM]
      .contains(&base);
    match (self.style, self.label.split_once(": ")) {
      (Style::Key | Style::ModeKey | Style::CommandKey, Some((key, text))) => {
        let key = terminal::clean(key, room);
        let pad = (keys.max(key.width()) + 2 - key.width())
          .min(room.saturating_sub(key.width()));
        let text =
          terminal::clean(text, room.saturating_sub(key.width() + pad));
        let style = if plain { theme::KEY } else { "" };
        let mut written = 0;
        for (start, end, token) in crate::help::key_styles(&key, self.style) {
          marked(out, &key[written..start], query, &format!("{base}{style}"))?;
          marked(out, &key[start..end], query, &format!("{base}{token}"))?;
          written = end;
        }
        marked(out, &key[written..], query, &format!("{base}{style}"))?;
        write!(out, "{base}{}", " ".repeat(pad))?;
        marked(out, &text, query, base)?;
        Ok(key.width() + pad + text.width())
      }
      (Style::Path, _) => {
        let split = self.label.rfind('/').map_or(0, |slash| slash + 1);
        let (parent, name) = self.label.split_at(split);
        // The name keeps its room; the parent loses its left end first.
        let name = terminal::clean(name, room);
        let parent = tail(parent, room.saturating_sub(name.width()));
        let muted = if plain { theme::MUTED } else { "" };
        marked(out, &parent, query, &format!("{base}{muted}"))?;
        marked(out, &name, query, &format!("{base}{}", theme::FILE))?;
        write!(out, "{base}")?;
        Ok(parent.width() + name.width())
      }
      (Style::Muted, _) => {
        let text = terminal::clean(&self.label, room);
        let muted = if plain { theme::MUTED } else { "" };
        marked(out, &text, query, &format!("{base}{muted}"))?;
        Ok(text.width())
      }
      (Style::Heading, _) => {
        let text = terminal::clean(&self.label, room);
        marked(out, &text, query, &format!("{base}{}", theme::HEADING))?;
        write!(out, "{base}")?;
        Ok(text.width())
      }
      _ => {
        let text = terminal::clean(&self.label, room);
        marked(out, &text, query, base)?;
        Ok(text.width())
      }
    }
  }
}

/// `text` within `columns`, cut from the left behind an ellipsis.
fn tail(text: &str, columns: usize) -> String {
  let text: String = crate::text::printable(text).collect();
  if text.width() <= columns {
    return text;
  }
  let Some(room) = columns.checked_sub(1) else { return String::new() };
  let mut width = 0;
  let kept: Vec<char> = text
    .chars()
    .rev()
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= room
    })
    .collect();
  std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

/// Writes `text` in `style` with query matches highlighted.
fn marked(
  out: &mut impl Write,
  text: &str,
  query: &str,
  style: &str,
) -> Result<()> {
  write!(out, "{style}")?;
  let mut written = 0;
  for (start, end) in crate::panel_search::ranges(text, query) {
    write!(
      out,
      "{}{}{}{style}",
      &text[written..start],
      theme::TEXT_MATCH,
      &text[start..end]
    )?;
    written = end;
  }
  write!(out, "{}", &text[written..])?;
  Ok(())
}

impl App {
  /// Follows section changes without interrupting panel browsing.
  pub fn track_outline(&mut self) {
    let section = self.section();
    let changed = self.current_section != section;
    self.current_section = section;
    let Some(panel) =
      self.panel.as_mut().filter(|panel| panel.kind == PanelKind::Outline)
    else {
      return;
    };
    panel.follow_pending |= changed;
    if !panel.focused
      && panel.follow_pending
      && let Some(index) = section
    {
      panel.selected = index;
      panel.reveal();
      panel.follow_pending = false;
    }
  }

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
      panel.selected = self.section().unwrap_or(0);
      panel.reveal();
    }
    self.panel = Some(panel);
    self.relayout()
  }

  /// Refills an open `kind` panel in place, keeping focus and selection.
  pub fn refill_panel(&mut self, kind: PanelKind) -> Result<()> {
    if self.panel.as_ref().is_none_or(|panel| panel.kind != kind) {
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    let note = self.panel_note(kind);
    if let Some(panel) = &mut self.panel {
      panel.refill(rows, note);
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
    panel.shelf = Some(Shelf::new(entries, &self.password));
    panel.height = self.screen.rows;
    self.panel = Some(panel);
    self.relayout()
  }

  /// Whether the recent view covers the pane.
  pub fn covered(&self) -> bool {
    self
      .panel
      .as_ref()
      .is_some_and(|panel| panel.shelf.is_some() && panel.focused)
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

  /// Returns from `gx` to the view it replaced.
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
        .map(|item| Row {
          style: if item.depth == 0 { Style::Heading } else { Style::Plain },
          ..Row::new(
            format!("{}{}", "  ".repeat(item.depth.min(12)), item.title),
            label(&item.target),
            Destination::Link(item.target.clone()),
          )
        })
        .collect(),
      PanelKind::Results => self
        .search
        .hits
        .iter()
        .map(|hit| {
          Row::new(
            hit.excerpt.clone(),
            self.doc.label(hit.first.page).to_owned(),
            Destination::Position(hit.first),
          )
        })
        .collect(),
      PanelKind::Links => self.link_rows()?,
      PanelKind::Jumps => self.jump_rows(),
      PanelKind::Annotations => self.annotation_rows()?,
      PanelKind::Help => help().iter().map(|line| Row::parse(line)).collect(),
      PanelKind::Info => self.info().lines().map(Row::parse).collect(),
      PanelKind::Text | PanelKind::Recent | PanelKind::Marks => Vec::new(),
    })
  }

  pub fn panel_input(&mut self, input: Input) -> Result<bool> {
    let Some(panel) = &mut self.panel else { return Ok(false) };
    if !panel.focused {
      return Ok(false);
    }
    if panel.navigate(input, usize::from(self.screen.rows)) {
      return Ok(true);
    }
    let covered = panel.shelf.is_some();
    match input.action {
      Action::Quit | Action::ClosePanel => {
        self.close_panel()?;
      }
      Action::Yank | Action::YankLine => {
        if let Some(row) = panel.rows.get(panel.selected) {
          let value = match (&panel.kind, &row.destination) {
            (PanelKind::Links, Destination::Link(Target::Uri(uri))) => uri,
            _ => &row.label,
          };
          let text = format!("{}\n", value.trim());
          self.reader.store(input.register, text)?;
        }
      }
      // The focused pane owns text motions and selections.
      Action::Motion(_)
      | Action::Visual(_)
      | Action::Object(..)
      | Action::Reselect
      | Action::WordSearch
      | Action::Cancel => {}
      Action::Peek if panel.kind == PanelKind::Annotations => {
        if let Some(Destination::Annotation { position, annotation }) =
          panel.rows.get(panel.selected).map(|row| row.destination.clone())
        {
          self.preview_annotation(position, &annotation);
        }
      }
      Action::Follow => self.panel_follow(Action::Follow)?,
      // The recent view opens with Enter alone.
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
    if mouse.kind == MouseEventKind::Down(MouseButton::Middle) {
      return self.panel_split(mouse);
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
  fn panel_split(&mut self, mouse: MouseEvent) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(index) = panel.row_at(mouse.column, mouse.row) else {
      return Ok(());
    };
    let request = match panel.rows[index].destination.clone() {
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

  /// PDF metadata, document facts, reader statistics, terminal caps.
  pub fn info(&self) -> String {
    let mut lines = metadata_lines(&self.doc.metadata());
    lines.push(String::new());
    let place = format!("Path: {}", self.path.display());
    let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
    lines.extend(document_lines(place, bytes, self.doc.pages.len()));
    let page = self.navigation_position().page;
    let bounds = self.doc.pages[page.0].bounds;
    let (width, height) = (bounds.width(), bounds.height());
    let outline = if self.doc.inferred { " (inferred)" } else { "" };
    lines.extend([
      format!(
        "Page {}: {width:.0} x {height:.0} pt, {:.0} x {:.0} mm",
        self.doc.label(page),
        width / 72.0 * 25.4,
        height / 72.0 * 25.4
      ),
      format!("Outline: {} entries{outline}", self.doc.outline.len()),
      format!("Marks: {}", self.marks.len()),
      String::new(),
      "# Reader".into(),
      format!("Zoom: {:.0}%", self.layout.zoom * 100.0),
      format!("Rotation: {}°", self.layout.rotation),
      format!(
        "Margins: {}",
        if self.layout.crop.is_none() { "shown" } else { "cropped" }
      ),
      format!(
        "Search: {} hits for {:?}",
        self.search.hits.len(),
        self.search.query.text
      ),
      format!("Text cache: {} KiB", self.doc.cache_bytes() / 1024),
      format!("Images: {} KiB resident", self.graphics.bytes() / 1024),
      format!("Transferred: {} KiB", self.graphics.uploaded / 1024),
      format!("Last tile: {:.3} ms", self.render_ms),
      format!(
        "Cell: {}x{} px",
        self.screen.cell_width, self.screen.cell_height
      ),
      format!("Screen: {}x{} cells", self.screen.columns, self.screen.rows),
      format!(
        "Presentation: {}",
        if self.text_mode { "text" } else { "images" }
      ),
      crate::caps::get()
        .map_or_else(|| "Terminal: not probed".into(), ToString::to_string),
    ]);
    lines.join("\n")
  }
}

/// `:info`'s metadata section, shared with recent document cards.
pub fn metadata_lines(metadata: &[(String, String)]) -> Vec<String> {
  let fields = metadata.iter().map(|(k, v)| format!("{k}: {v}"));
  std::iter::once("# Metadata".to_owned()).chain(fields).collect()
}

/// `:info`'s file facts, shared with recent document cards; `place`
/// names where the file lives.
pub fn document_lines(place: String, bytes: u64, pages: usize) -> Vec<String> {
  vec![
    "# Document".into(),
    place,
    format!("Size: {:.1} MiB", bytes as f64 / 1_048_576.0),
    format!("Pages: {pages}"),
  ]
}

pub(crate) fn help() -> Vec<String> {
  let keys = include_str!("help.txt").lines().map(str::to_owned);
  let commands = crate::cmdline::commands().map(|spec| {
    let usage = if spec.usage.is_empty() { "" } else { " " };
    format!(":{}{usage}{}: {}", spec.name, spec.usage, spec.help)
  });
  keys.chain([String::new(), "# Commands".into()]).chain(commands).collect()
}

/// Recent documents that still exist, newest first.
pub fn recent_rows(entries: &[Entry]) -> Vec<Row> {
  let home = std::env::var_os("HOME").map(PathBuf::from);
  entries
    .iter()
    .filter(|entry| entry.path.exists())
    .map(|entry| {
      let path = match home.as_deref().map(|h| entry.path.strip_prefix(h)) {
        Some(Ok(rest)) => Path::new("~").join(rest),
        _ => entry.path.clone(),
      };
      Row {
        style: Style::Path,
        ..Row::new(
          path.display().to_string(),
          format!("p{}", entry.cursor.page.0 + 1),
          Destination::File(entry.path.clone()),
        )
      }
    })
    .collect()
}

/// Last outline entry starting at or before `page`.
pub fn section(outline: &[Heading], page: PageId, y: f32) -> Option<usize> {
  outline.iter().rposition(|heading| match heading.target {
    Target::Page(start) => start <= page,
    Target::Point { page: start, y: top, .. } => {
      start < page || start == page && top <= y
    }
    Target::Uri(_) => false,
  })
}

impl App {
  /// Outline entry containing the cursor.
  pub fn section(&mut self) -> Option<usize> {
    let cursor = self.navigation_position();
    if self.reader.viewport_mode() && !self.text_mode {
      let y = self
        .layout
        .unproject(cursor.page, (0.0, 0.0), self.reader.scroll)
        .map_or(0.0, |(_, y)| y);
      return section(&self.doc.outline, cursor.page, y);
    }
    let y = self
      .doc
      .text(cursor.page)
      .ok()
      .and_then(|text| text.glyph(cursor).map(|g| g.bounds.y1))
      .unwrap_or(f32::MAX);
    section(&self.doc.outline, cursor.page, y)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn paths_keep_the_file_name() -> Result<()> {
    let row = Row {
      style: Style::Path,
      ..Row::new("~/books/ml/deep.pdf".into(), String::new(), Destination::None)
    };
    // (room, visible text)
    for (room, want) in [
      (40, "~/books/ml/deep.pdf"),
      (14, "…s/ml/deep.pdf"),
      (9, "…deep.pdf"),
      (8, "deep.pdf"),
      (5, "deep."),
    ] {
      let mut out = Vec::new();
      let width = row.write(&mut out, theme::RESET, room, 0, "")?;
      let out = String::from_utf8(out)?;
      let text: String = out
        .split('\x1b')
        .map(|s| s.split_once('m').map_or(s, |(_, t)| t))
        .collect();
      assert_eq!(text, want, "{room}");
      assert_eq!(width, want.width(), "{room}");
      assert!(out.contains(&format!("{}deep", theme::FILE)), "{room}");
    }
    Ok(())
  }
  #[test]
  fn presses_beyond_the_list_miss_its_rows() -> Result<()> {
    let rows = ["a", "b", "c"].map(Row::parse).into();
    let mut panel = Panel::new(PanelKind::Recent, rows);
    panel.list(&mut Vec::new(), terminal::At::default(), 20, 10)?;
    let press = |column| MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column,
      row: 2,
      modifiers: crossterm::event::KeyModifiers::NONE,
    };
    assert_eq!(panel.pointer(press(25)), None);
    assert_eq!(panel.pointer(press(5)), Some(1));
    Ok(())
  }
}

#[cfg(test)]
mod focus_tests {
  use super::*;
  use crossterm::event::KeyModifiers;

  #[test]
  fn wheel_moves_viewport_and_keys_reveal_rows() -> Result<()> {
    let mut panel = Panel::new(
      PanelKind::Help,
      (0..80).map(|row| Row::parse(&format!("row {row}"))).collect(),
    );
    panel.focused = false;
    panel.list(&mut Vec::new(), terminal::At::default(), 40, 12)?;
    let wheel = MouseEvent {
      kind: MouseEventKind::ScrollDown,
      column: 2,
      row: 4,
      modifiers: KeyModifiers::NONE,
    };
    panel.pointer(wheel);
    panel.list(&mut Vec::new(), terminal::At::default(), 40, 12)?;
    assert_eq!((panel.selected, panel.top, panel.focused), (0, 3, false));
    panel.selected = 2;
    panel.list(&mut Vec::new(), terminal::At::default(), 40, 12)?;
    assert_eq!(panel.top, 3, "tracking does not undo wheel scrolling");
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 12);
    assert_eq!((panel.selected, panel.top), (3, 3));
    panel.selected = 0;
    panel.begin_search(false);
    panel.preview("row 40");
    assert_eq!(panel.selected, 40);
    assert_eq!(panel.top, 31, "search reveals the matching row");
    Ok(())
  }
}

#[cfg(test)]
mod help_tests {
  use super::*;
  use crossterm::event::KeyModifiers;

  #[test]
  fn scrolling_can_enter_wrapped_rows() -> Result<()> {
    let rows =
      ["# Move", "j: one two three four five six seven eight nine ten"]
        .map(Row::parse)
        .into();
    let mut panel = Panel::new(PanelKind::Help, rows);
    panel.list(&mut Vec::new(), terminal::At::default(), 12, 4)?;
    panel.selected = 1;
    for top in 1..=3 {
      panel.navigate(Input::plain(Action::Rows(1)), 4);
      panel.list(&mut Vec::new(), terminal::At::default(), 12, 4)?;
      assert_eq!((panel.top, panel.selected), (top, 1));
    }
    Ok(())
  }

  #[test]
  fn wrapped_rows_keep_their_source() -> Result<()> {
    let rows =
      ["# Move", "j: one two three four five", "k: up"].map(Row::parse).into();
    let mut panel = Panel::new(PanelKind::Help, rows);
    panel.list(&mut Vec::new(), terminal::At::default(), 15, 6)?;
    let press = |row| MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 5,
      row,
      modifiers: KeyModifiers::NONE,
    };
    assert_eq!(panel.pointer(press(3)), Some(1));
    assert_eq!(panel.pointer(press(4)), Some(1));
    panel.begin_search(false);
    panel.preview("three four");
    assert_eq!(panel.selected, 1);
    assert_eq!(panel.count(), Some((1, 1)));
    assert_eq!(panel.rows[panel.selected].label, "j: one two three four five");
    panel.finish_search("three four");
    panel.selected = 2;
    panel.reveal();
    assert_eq!(panel.top, 1);
    panel.list(&mut Vec::new(), terminal::At::default(), 40, 6)?;
    assert_eq!(panel.selected, 2);
    assert_eq!(panel.top, 0);
    assert_eq!(panel.pointer(press(3)), Some(2));
    assert_eq!(panel.pointer(press(4)), None);
    Ok(())
  }
}
