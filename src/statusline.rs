use crate::Result;
use crate::app::{App, InputOwner, Tabline};
use crate::find::Phase;
use crate::grid;
use crate::layout::Screen;
use crate::pointer::Hit;
use crate::prompt::Prompt;
use crate::reader::{Mode, SelectionKind};
use crate::terminal::{self, At};
use crate::text::PageId;
use crate::theme::{self, Color};
use std::fmt::Write as _;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

impl App {
  fn mode(&self) -> (&'static str, Color) {
    match self.input_owner() {
      InputOwner::Prompt => return ("PROMPT", Color::Yellow),
      InputOwner::Hints => return ("HINT", Color::Yellow),
      InputOwner::Context => return ("MENU", Color::Yellow),
      InputOwner::Peek => return ("PEEK", Color::Yellow),
      InputOwner::Region => return ("REGION", Color::Yellow),
      InputOwner::Panel => {
        if let Some(panel) = &self.panel {
          return (panel.kind.title(), Color::Cyan);
        }
      }
      InputOwner::Overview => return ("PAGES", Color::Cyan),
      InputOwner::Content => {}
    }
    match self.reader.mode() {
      Mode::Visual(SelectionKind::Character) => ("VISUAL", Color::Magenta),
      Mode::Visual(SelectionKind::Line) => ("V-LINE", Color::Magenta),
      _ if self.reader.operator.is_some() => ("YANK", Color::Yellow),
      Mode::Cursor => ("CURSOR", Color::Cyan),
      Mode::Normal => ("NORMAL", Color::Blue),
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
    let more =
      if matches!(self.search.phase, Phase::Running { .. }) { "+" } else { "" };
    if self.reader.viewport_mode() {
      return format!("[{}{more} matches]  ", hits.len());
    }
    let cursor = self.reader.cursor;
    let index = hits.partition_point(|hit| hit.first <= cursor);
    format!("[{index}/{}{more}]  ", hits.len())
  }

  /// Share of the document above the viewport's bottom edge.
  fn progress(&self) -> u32 {
    let fraction = if self.text_mode {
      (self.navigation_position().page.0 + 1) as f32
        / self.doc.pages.len() as f32
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
    let file = || {
      let label = format!(" {}:{} ", active + 1, self.name());
      let label = terminal::clean(&label, room.saturating_sub(1));
      let gap = if room > 0 { " " } else { "" };
      let width = label.width() + gap.width();
      (format!("{}{label}{}{gap}", theme::STATUS_FILE, theme::BAR), width)
    };
    if names.len() < 2 {
      return file();
    }
    let labels: Vec<_> = names
      .iter()
      .enumerate()
      .map(|(index, name)| format!(" {}:{name} ", index + 1))
      .collect();
    let width = labels.iter().map(|label| label.width()).sum::<usize>() + 1;
    if width > room {
      return file();
    }
    let mut text = String::new();
    for (index, label) in labels.iter().enumerate() {
      let style =
        if index == *active { theme::TAB_ACTIVE } else { theme::TAB_INACTIVE };
      text.extend([style, label.as_str(), theme::BAR]);
    }
    (text + " ", width)
  }

  /// Status-line text for the hover.
  pub(crate) fn hover_detail(&mut self) -> Option<String> {
    match self.hover {
      Hit::Note { page, index, .. } => {
        self.note_line(page, index).ok().flatten()
      }
      Hit::Row(_) => self.panel.as_ref()?.hover_title.clone(),
      Hit::Card(row) => self.panel.as_ref()?.shelf.as_ref()?.detail(row),
      Hit::Thumbnail(page) => Some(self.page_line(page)),
      Hit::Link(..) => self.hover_label(),
      Hit::Nothing | Hit::Text | Hit::Preview { .. } | Hit::Item(..)
      | Hit::Edge { .. } => None,
    }
  }

  /// A page's label and section.
  fn page_line(&self, page: PageId) -> String {
    let label = self.doc.label(page);
    let section = crate::panel::section(&self.doc.outline, page, f32::MAX);
    match section.map(|index| self.doc.outline[index].title.trim()) {
      Some(title) => format!("p. {label} · {title}"),
      None => format!("p. {label}"),
    }
  }

  /// Mode, tabs, message, then pending keys and position.
  pub fn status(&self, out: &mut impl Write, row: u16) -> Result<()> {
    self.status_line(out, row, None)
  }

  /// `status`, `detail` replacing the section.
  pub fn status_line(
    &self,
    out: &mut impl Write,
    row: u16,
    detail: Option<&str>,
  ) -> Result<()> {
    let columns = usize::from(self.screen.columns);
    let (mode, color) = self.mode();
    let page = self.navigation_position().page.0;
    let mut right = self.keys.pending();
    if !right.is_empty() {
      right.push_str("  ");
    }
    // The recent view's document is closed; it shows no position.
    let covered = self.covered();
    if !covered {
      right += &self.search_count();
      write!(
        right,
        "{}/{} [{}] {}% ",
        page + 1,
        self.doc.pages.len(),
        self.doc.label(PageId(page)),
        self.progress(),
      )?;
    }
    if !covered && (self.layout.zoom - 1.0).abs() > 0.005 {
      write!(right, " zoom {:.0}% ", self.layout.zoom * 100.0)?;
    }
    let left = format!(" {mode} ");
    write!(out, "{}", self.at.goto(row, 0u16))?;
    let Some(room) = columns.checked_sub(left.width() + right.width() + 1)
    else {
      let right = terminal::clean(&right, columns);
      write!(out, "{}{right}{}", theme::BAR, theme::RESET)?;
      // Plain frames skip the row clear, so erase what follows.
      let rest = columns.saturating_sub(right.width());
      if rest > 0 {
        write!(out, "\x1b[{rest}X")?;
      }
      return Ok(());
    };
    let (tabs, width) = self.tabs(room);
    let section =
      self.current_section.map(|index| self.doc.outline[index].title.trim());
    let quiet = covered || self.reader.message.is_empty();
    let (message, style) = match (&self.error, section) {
      (Some(error), _) => (error.as_str(), theme::ERROR),
      (None, _) if let Some(detail) = detail.filter(|_| quiet) => {
        (detail, theme::BAR)
      }
      (None, _) if covered => ("Enter open  :e file  q close", theme::BAR),
      (None, Some(title)) if self.reader.message.is_empty() => {
        (title, theme::STATUS_SECTION)
      }
      (None, _) => (self.reader.message.as_str(), theme::BAR),
    };
    let room = room.saturating_sub(width);
    // A cut detail says so; `K` shows notes whole.
    let message = match detail {
      Some(detail) if message == detail => terminal::clipped(message, room).0,
      _ => terminal::clean(message, room),
    };
    let fill = room.saturating_sub(message.width()) + 1;
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
    let bottom = (at, row - 1);
    grid::draw(out, bottom, &menu.labels, &layout, menu.selected, 0, screen)?;
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

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn app() -> Result<App> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    App::with_screen(
      &root.join("paper.pdf"),
      "",
      true,
      Screen {
        columns: 120,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
        origin: (0, 0),
      },
    )
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn filenames_keep_style_and_number() -> Result<()> {
    let mut app = app()?;
    let single = format!("{} 1:paper.pdf {} ", theme::STATUS_FILE, theme::BAR);
    assert_eq!(app.tabs(80), (single.clone(), 14));
    app.tabline = Tabline { names: vec!["paper.pdf".into()], active: 0 };
    assert_eq!(app.tabs(80), (single, 14));
    app.tabline.names.insert(0, "book.pdf".into());
    for active in 0..2 {
      app.tabline.active = active;
      let styles = if active == 0 {
        [theme::TAB_ACTIVE, theme::TAB_INACTIVE]
      } else {
        [theme::TAB_INACTIVE, theme::TAB_ACTIVE]
      };
      let expected = format!(
        "{} 1:book.pdf {}{} 2:paper.pdf {} ",
        styles[0],
        theme::BAR,
        styles[1],
        theme::BAR
      );
      assert_eq!(app.tabs(80), (expected, 26));
    }
    let narrow = format!("{} 2:paper.pdf {} ", theme::STATUS_FILE, theme::BAR);
    assert_eq!(app.tabs(14), (narrow, 14));
    app.path = "論文.pdf".into();
    for room in 0..30 {
      let (tabs, width) = app.tabs(room);
      let plain = tabs
        .replace(theme::STATUS_FILE, "")
        .replace(theme::TAB_INACTIVE, "")
        .replace(theme::BAR, "");
      assert_eq!(width, plain.width(), "{room}");
      assert!(width <= room, "{room}");
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn section_has_separate_style() -> Result<()> {
    let mut app = app()?;
    app.doc.load_outline()?;
    app.current_section = Some(0);
    let title = app.doc.outline[0].title.trim();
    let mut out = Vec::new();
    app.status(&mut out, 23)?;
    let out = String::from_utf8(out)?;
    assert!(out.contains(&format!("{} 1:paper.pdf ", theme::STATUS_FILE)));
    let section = format!("{}{title}{}", theme::STATUS_SECTION, theme::BAR);
    assert!(out.contains(&section));
    Ok(())
  }
}
