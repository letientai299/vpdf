use crate::Result;
use crate::app::{App, Tabline};
use crate::find::Phase;
use crate::grid;
use crate::layout::Screen;
use crate::prompt::Prompt;
use crate::reader::SelectionKind;
use crate::terminal::{self, At};
use crate::theme::{self, Color};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

impl App {
  fn mode(&self) -> (&'static str, Color) {
    if let Some(panel) = self.panel.as_ref().filter(|panel| panel.focused) {
      return (panel.kind.title(), Color::Cyan);
    }
    if self.hints.is_some() {
      return ("HINT", Color::Yellow);
    }
    if self.overview.is_some() {
      return ("PAGES", Color::Cyan);
    }
    if self.preview.is_some() {
      return ("TEXT", Color::Green);
    }
    match self.reader.selection.map(|selection| selection.kind) {
      Some(SelectionKind::Character) => ("VISUAL", Color::Magenta),
      Some(SelectionKind::Line) => ("V-LINE", Color::Magenta),
      None if self.reader.operator.is_some() => ("YANK", Color::Yellow),
      None => ("NORMAL", Color::Blue),
    }
  }

  /// Match under or before the cursor, like Neovim's searchcount.
  fn search_count(&self) -> String {
    if let Some(panel) = self.panel.as_ref().filter(|panel| panel.focused) {
      return panel
        .count()
        .map_or_else(String::new, |(i, n)| format!("[{i}/{n}]  "));
    }
    let hits = &self.search.hits;
    if self.search.hidden || hits.is_empty() {
      return String::new();
    }
    let cursor = self.reader.cursor;
    let index = hits.partition_point(|hit| hit.first <= cursor);
    let more =
      if matches!(self.search.phase, Phase::Running { .. }) { "+" } else { "" };
    format!("[{index}/{}{more}]  ", hits.len())
  }

  /// Share of the document above the viewport's bottom edge.
  fn progress(&self) -> u32 {
    let fraction = if self.text_mode {
      (self.reader.cursor.page.0 + 1) as f32 / self.doc.pages.len() as f32
    } else {
      let bottom = self.reader.scroll + self.layout.screen.height();
      bottom / self.layout.total.max(1.0)
    };
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
  }

  /// Document title for the terminal window.
  pub fn title(&self) -> String {
    let name = self.name();
    if self.doc.title.is_empty() {
      return name;
    }
    format!("{} — {name}", self.doc.title)
  }

  /// Tab names that fit in `room`, with their display width.
  fn tabs(&self, room: usize) -> (String, usize) {
    let Tabline { names, active } = &self.tabline;
    // Narrow panes cut the name so the status line stays in the pane.
    let plain = |text: String| {
      let text = terminal::clean(&text, room);
      let width = text.width();
      (text, width)
    };
    if names.len() < 2 {
      return plain(format!(" {}  ", self.name()));
    }
    let labels: Vec<_> = names
      .iter()
      .enumerate()
      .map(|(index, name)| format!(" {}:{name} ", index + 1))
      .collect();
    let width = labels.iter().map(|label| label.width()).sum::<usize>() + 1;
    if width > room {
      return plain(format!(
        " {}/{} {}  ",
        active + 1,
        names.len(),
        self.name()
      ));
    }
    let mut text = String::new();
    for (index, label) in labels.iter().enumerate() {
      let style = if index == *active { theme::TAB } else { theme::BAR };
      text += &format!("{style}{label}{}", theme::BAR);
    }
    (text + " ", width)
  }

  /// Mode, tabs, message, then pending keys and position.
  pub fn status(&self, out: &mut impl Write, row: u16) -> Result<()> {
    let columns = usize::from(self.screen.columns);
    let (mode, color) = self.mode();
    let page = self.reader.cursor.page.0;
    let mut right = self.keys.pending();
    if !right.is_empty() {
      right.push_str("  ");
    }
    right += &self.search_count();
    right += &format!(
      "{}/{} [{}] {}% ",
      page + 1,
      self.doc.pages.len(),
      self.doc.pages[page].label,
      self.progress(),
    );
    if (self.layout.zoom - 1.0).abs() > 0.005 {
      right += &format!(" zoom {:.0}% ", self.layout.zoom * 100.0);
    }
    let left = format!(" {mode} ");
    write!(out, "{}", self.at.goto(row, 0u16))?;
    let Some(room) = columns.checked_sub(left.width() + right.width() + 1)
    else {
      let right = terminal::clean(&right, columns);
      write!(out, "{}{right}{}", theme::BAR, theme::RESET)?;
      return Ok(());
    };
    let (tabs, width) = self.tabs(room);
    let section =
      self.current_section.map(|index| self.doc.outline[index].title.trim());
    let (message, style) = match (&self.error, section) {
      (Some(error), _) => (error.as_str(), theme::ERROR),
      (None, Some(title)) if self.reader.message.is_empty() => {
        (title, theme::MUTED_BAR)
      }
      (None, _) => (self.reader.message.as_str(), theme::BAR),
    };
    let message = terminal::clean(message, room.saturating_sub(width));
    let fill = room.saturating_sub(width + message.width()) + 1;
    let badge =
      if self.focused { theme::badge(color) } else { theme::BAR.into() };
    write!(
      out,
      "{badge}{left}{}{tabs}{style}{message}{}{}{right}{}",
      theme::BAR,
      theme::BAR,
      " ".repeat(fill),
      theme::RESET
    )?;
    Ok(())
  }
}

/// Prompt on the last row, its list above, a ghost suggestion, and a
/// block cursor.
pub fn prompt(
  out: &mut impl Write,
  prompt: &Prompt,
  screen: Screen,
  at: At,
) -> Result<()> {
  let row = screen.rows - 1;
  if let Some(menu) = &prompt.menu {
    let layout = menu.layout(usize::from(screen.columns), grid::limit(screen));
    grid::draw(out, (at, row - 1), &layout, menu.selected, 0, screen)?;
  }
  let (text, ghost, column) = prompt.display();
  let room = usize::from(screen.columns).saturating_sub(text.width());
  write!(
    out,
    "{}{}",
    at.goto(row, 0u16),
    terminal::clean(&text, usize::from(screen.columns))
  )?;
  let ghost = terminal::clean(ghost, room);
  let pad = " ".repeat(room.saturating_sub(ghost.width()));
  write!(out, "{}{ghost}{}{pad}", theme::MUTED, theme::RESET)?;
  let column = column.min(usize::from(screen.columns) - 1);
  write!(out, "{}\x1b[2 q\x1b[?25h", at.goto(row, column))?;
  Ok(())
}
