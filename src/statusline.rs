use crate::Result;
use crate::app::App;
use crate::reader::SelectionKind;
use crate::terminal;
use crate::theme::{self, Color};
use std::io::Write;
use unicode_width::UnicodeWidthStr;

impl App {
  fn mode(&self) -> (&'static str, Color) {
    if let Some(panel) = self.panel.as_ref().filter(|panel| panel.focused) {
      return (panel.kind.title(), Color::Cyan);
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

  /// Mode, file, message, then pending keys and position.
  pub fn status(&self, out: &mut impl Write, row: u16) -> Result<()> {
    let columns = usize::from(self.screen.columns);
    let (mode, color) = self.mode();
    let name = self.path.file_name().map_or_else(
      || self.path.display().to_string(),
      |name| name.to_string_lossy().into_owned(),
    );
    let page = self.reader.cursor.page.0;
    let mut right = self.keys.pending();
    if !right.is_empty() {
      right.push_str("  ");
    }
    right += &format!(
      "{}/{} [{}]  {:.0}% ",
      page + 1,
      self.doc.pages.len(),
      self.doc.pages[page].label,
      self.layout.zoom * 100.0,
    );
    let left = format!(" {mode} ");
    let middle = format!(" {name}  ");
    let (message, style) = match &self.error {
      Some(error) => (error.as_str(), theme::ERROR),
      None => (self.reader.message.as_str(), theme::BAR),
    };
    let room =
      columns.saturating_sub(left.width() + middle.width() + right.width() + 1);
    let message = terminal::clean(message, room);
    let fill = room - message.width();
    let line = format!(
      "{}{left}{}{middle}{style}{message}{}{}{right}",
      theme::badge(color),
      theme::BAR,
      theme::BAR,
      " ".repeat(fill + 1),
    );
    write!(out, "\x1b[{};1H", row + 1)?;
    if left.width() + middle.width() + right.width() < columns {
      write!(out, "{line}{}", theme::RESET)?;
    } else {
      write!(
        out,
        "{}{}{}",
        theme::BAR,
        terminal::clean(&right, columns),
        theme::RESET
      )?;
    }
    Ok(())
  }
}
