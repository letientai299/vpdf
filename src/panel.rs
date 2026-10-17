use crate::Result;
use crate::app::{App, Request};
use crate::document::{Heading, Target};
use crate::keys::{Action, Input, PanelKind};
use crate::motion::Motion;
use crate::panel_search::RowSearch;
use crate::state::Entry;
use crate::terminal;
use crate::text::{PageId, Position};
use crate::theme::{self, Color};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::io::Write;
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthStr;

#[derive(Clone)]
pub enum Destination {
  Position(Position),
  Link(Target),
  File(PathBuf),
  /// A global mark, resolved by the session.
  Mark(char),
  None,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Style {
  Plain,
  Heading,
  /// `keys: description`, with the keys highlighted.
  Key,
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
}

impl Panel {
  pub fn new(kind: PanelKind, rows: Vec<Row>) -> Self {
    Self {
      kind,
      rows,
      selected: 0,
      focused: true,
      top: 0,
      search: RowSearch::default(),
      note: "",
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
    self.rows = rows;
    self.note = note;
  }

  /// Applies list motions; `page` rows per full-screen scroll.
  pub fn navigate(&mut self, input: Input, page: usize) -> bool {
    let last = self.rows.len().saturating_sub(1);
    let count = input.count;
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
        self.top = self.selected.saturating_sub((visible * fraction) as usize);
        self.selected
      }
      Action::Result(reverse) => {
        self.repeat(reverse, count);
        self.selected
      }
      _ => return false,
    }
    .min(last);
    true
  }

  /// Returns a clicked row; only presses focus.
  pub fn pointer(&mut self, mouse: MouseEvent) -> Option<usize> {
    let last = self.rows.len().saturating_sub(1);
    if matches!(mouse.kind, MouseEventKind::Down(_)) {
      self.focused = true;
    }
    match mouse.kind {
      MouseEventKind::ScrollDown => {
        self.selected = (self.selected + 3).min(last);
      }
      MouseEventKind::ScrollUp => {
        self.selected = self.selected.saturating_sub(3);
      }
      MouseEventKind::Down(MouseButton::Left) if mouse.row > 0 => {
        let index = self.top + usize::from(mouse.row) - 1;
        if index <= last && !self.rows.is_empty() {
          self.selected = index;
          return Some(index);
        }
      }
      _ => {}
    }
    None
  }

  fn reveal(&mut self, count: usize) {
    if self.selected < self.top {
      self.top = self.selected;
    } else if count > 0 && self.selected >= self.top + count {
      self.top = self.selected + 1 - count;
    }
  }

  /// Columns a panel of `kind` takes in a pane `columns` wide; none
  /// when the page area would keep fewer than 12.
  pub fn width(kind: PanelKind, columns: u16) -> u16 {
    let wide = matches!(kind, PanelKind::Help | PanelKind::Info);
    let width = if wide { columns / 2 } else { columns / 3 };
    width.clamp(1, if wide { 64 } else { 50 }).min(columns.saturating_sub(12))
  }

  /// Draws `width` columns; the last is the border.
  pub fn draw(
    &mut self,
    out: &mut impl Write,
    at: terminal::At,
    width: u16,
    height: u16,
  ) -> Result<()> {
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
    let keys = self
      .rows
      .iter()
      .filter(|row| row.style == Style::Key)
      .filter_map(|row| row.label.split_once(": "))
      .map(|(key, _)| key.width())
      .max()
      .unwrap_or(0)
      .min(inner / 2);
    let count = height.saturating_sub(2) as usize;
    self.reveal(count);
    let start = self.top;
    for (row, item) in self.rows.iter().skip(start).take(count).enumerate() {
      let base = match (start + row == self.selected, self.focused) {
        (true, true) => theme::SELECTED,
        (true, false) => theme::CURRENT,
        (false, true) => theme::RESET,
        (false, false) => theme::DIM,
      };
      write!(out, "{}{base}", at.goto(row + 1, 0u16))?;
      let detail = terminal::clean(&item.detail, inner / 3);
      let room = inner.saturating_sub(detail.width() + 1);
      let used = item.write(out, base, room, keys, &self.search.query)?;
      write!(
        out,
        "{}{}{detail}{}",
        " ".repeat(inner.saturating_sub(used + detail.width())),
        if base == theme::RESET { theme::MUTED } else { "" },
        theme::RESET
      )?;
    }
    Ok(())
  }
}

impl Row {
  /// Writes the label within `room` columns and returns its width.
  fn write(
    &self,
    out: &mut impl Write,
    base: &str,
    room: usize,
    keys: usize,
    query: &str,
  ) -> Result<usize> {
    let plain = base == theme::RESET || base == theme::DIM;
    match (self.style, self.label.split_once(": ")) {
      (Style::Key, Some((key, text))) => {
        let key = terminal::clean(key, room);
        let pad = (keys.max(key.width()) + 2 - key.width())
          .min(room.saturating_sub(key.width()));
        let text =
          terminal::clean(text, room.saturating_sub(key.width() + pad));
        let style = if plain { theme::KEY } else { "" };
        marked(out, &key, query, &format!("{base}{style}"))?;
        write!(out, "{base}{}", " ".repeat(pad))?;
        marked(out, &text, query, base)?;
        Ok(key.width() + pad + text.width())
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
  /// Finds the cursor's section and selects it in an idle outline.
  pub fn track_outline(&mut self) {
    self.current_section = self.section();
    let Some(panel) = &mut self.panel else { return };
    if panel.kind != PanelKind::Outline || panel.focused {
      return;
    }
    if let Some(index) = self.current_section {
      panel.selected = index;
    }
  }

  pub fn open_panel(&mut self, kind: PanelKind) -> Result<()> {
    if kind == PanelKind::Text {
      if self.preview.is_some() {
        return self.close_text_preview();
      }
      self.preview = Some(self.text_mode);
      self.text_mode = true;
      // The preview takes focus so its text motions work.
      if let Some(panel) = &mut self.panel {
        panel.focused = false;
      }
      return Ok(());
    }
    if let Some(panel) = &mut self.panel
      && panel.kind == kind
    {
      if kind == PanelKind::Outline {
        panel.focused = true;
      } else {
        self.close_panel()?;
      }
      return Ok(());
    }
    if matches!(kind, PanelKind::Recent | PanelKind::Marks) {
      self.request = Some(Request::Panel(kind));
      return Ok(());
    }
    let rows = self.panel_rows(kind)?;
    self.show_panel(kind, rows)
  }

  pub fn show_panel(&mut self, kind: PanelKind, rows: Vec<Row>) -> Result<()> {
    let mut panel = Panel::new(kind, rows);
    panel.note = self.panel_note(kind);
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

  pub fn close_panel(&mut self) -> Result<()> {
    self.panel = None;
    self.relayout()
  }

  /// Returns from `gx` to the view it replaced.
  pub fn close_text_preview(&mut self) -> Result<()> {
    if let Some(previous) = self.preview.take() {
      self.text_mode = previous;
    }
    self.requested.clear();
    // Text mode moves the cursor without the page view.
    self.ensure_cursor()
  }

  pub fn panel_rows(&mut self, kind: PanelKind) -> Result<Vec<Row>> {
    let label = |target: &Target| match target {
      Target::Page(page) | Target::Point { page, .. } => {
        self.doc.pages.get(page.0).map_or_else(String::new, |p| p.label.clone())
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
            self.doc.pages[hit.first.page.0].label.clone(),
            Destination::Position(hit.first),
          )
        })
        .collect(),
      PanelKind::Links => self
        .doc
        .links(self.reader.cursor.page)?
        .into_iter()
        .map(|link| {
          let text = match &link.target {
            Target::Uri(uri) => uri.clone(),
            _ => "Go to page".into(),
          };
          Row::new(text, label(&link.target), Destination::Link(link.target))
        })
        .collect(),
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
    match input.action {
      Action::Quit | Action::ClosePanel => {
        self.close_panel()?;
      }
      Action::Yank | Action::YankLine => {
        if let Some(row) = panel.rows.get(panel.selected) {
          let text = format!("{}\n", row.label.trim());
          self.reader.store(input.register, text)?;
        }
      }
      // The focused pane owns text motions and selections.
      Action::Motion(_)
      | Action::Visual(_)
      | Action::Object(_)
      | Action::Reselect
      | Action::WordSearch => {}
      Action::Cancel => {
        panel.focused = false;
      }
      Action::Follow | Action::Swap => {
        self.panel_follow(input.action)?;
      }
      _ => return Ok(false),
    }
    Ok(true)
  }

  pub fn panel_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
    let Some(panel) = &mut self.panel else { return Ok(()) };
    let Some(index) = panel.pointer(mouse) else { return Ok(()) };
    let moves = !matches!(panel.rows[index].destination, Destination::None);
    if moves && self.overview.is_some() {
      // Show the followed row, not the grid.
      self.close_overview();
    }
    self.panel_follow(Action::Swap)
  }

  fn panel_follow(&mut self, action: Action) -> Result<()> {
    let Some(panel) = &self.panel else { return Ok(()) };
    let Some(row) = panel.rows.get(panel.selected) else { return Ok(()) };
    let destination = row.destination.clone();
    let outline = panel.kind == PanelKind::Outline;
    match &destination {
      Destination::File(path) => {
        self.request = Some(Request::Choose(path.clone()));
        return self.close_panel();
      }
      Destination::Mark(name) => {
        self.request = Some(Request::GoMark(*name, true));
        return self.close_panel();
      }
      Destination::None => return Ok(()),
      Destination::Position(_) | Destination::Link(_) => {}
    }
    // Closing first, so the page top is measured in the final layout.
    if action == Action::Follow {
      if outline {
        if let Some(panel) = &mut self.panel {
          panel.focused = false;
        }
      } else {
        self.close_panel()?;
      }
    }
    match destination {
      Destination::Position(position) => self.go_to(position, true),
      Destination::Link(target) => self.follow(&target),
      _ => Ok(()),
    }
  }

  /// PDF metadata, document facts, reader statistics, terminal caps.
  pub fn info(&self) -> String {
    let mut lines = vec!["# Metadata".to_owned()];
    lines.extend(
      self.doc.metadata().into_iter().map(|(k, v)| format!("{k}: {v}")),
    );
    let page = &self.doc.pages[self.reader.cursor.page.0];
    let (width, height) = (page.bounds.width(), page.bounds.height());
    let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
    let outline = if self.doc.inferred { " (inferred)" } else { "" };
    lines.extend([
      String::new(),
      "# Document".into(),
      format!("Path: {}", self.path.display()),
      format!("Size: {:.1} MiB", bytes as f64 / 1_048_576.0),
      format!("Pages: {}", self.doc.pages.len()),
      format!(
        "Page {}: {width:.0} x {height:.0} pt, {:.0} x {:.0} mm",
        page.label,
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

fn help() -> Vec<String> {
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
      let name = entry.path.file_name().unwrap_or_default();
      let parent = entry.path.parent().unwrap_or(Path::new(""));
      let folder = match home.as_deref().map(|h| parent.strip_prefix(h)) {
        Some(Ok(rest)) => Path::new("~").join(rest),
        _ => parent.to_owned(),
      };
      Row::new(
        format!("{}  {}", name.to_string_lossy(), folder.display()),
        format!("p{}", entry.cursor.page.0 + 1),
        Destination::File(entry.path.clone()),
      )
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
    let cursor = self.reader.cursor;
    let y = self
      .doc
      .text(cursor.page)
      .ok()
      .and_then(|text| text.glyph(cursor).map(|g| g.bounds.y1))
      .unwrap_or(f32::MAX);
    section(&self.doc.outline, cursor.page, y)
  }
}
