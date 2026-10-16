use crate::Result;
use crate::app::App;
use crate::reader::SelectionKind;
use crate::text::Position;
use std::io::Write;
use unicode_width::UnicodeWidthChar;

impl App {
  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let height = self.screen.rows.saturating_sub(1) as usize;
    let start = self.reader.cursor.line.saturating_sub(height / 2);
    let columns = self.screen.columns.saturating_sub(self.left) as usize;
    let horizontal = self.reader.cursor.glyph.saturating_sub(columns / 2);
    for (row, line) in text.lines.iter().skip(start).take(height).enumerate() {
      write!(out, "\x1b[{};{}H", row + 1, self.left + 1)?;
      let mut width = 0;
      for (glyph, ch) in line.glyphs.iter().enumerate().skip(horizontal) {
        width += ch.ch.width().unwrap_or(0);
        if width > columns {
          break;
        }
        let pos =
          Position { page: self.reader.cursor.page, line: start + row, glyph };
        let style = self.text_style(pos);
        write!(out, "\x1b[{style}m{}", ch.ch)?;
      }
      write!(out, "\x1b[0m")?;
    }
    Ok(())
  }

  fn text_style(&self, position: Position) -> &'static str {
    if position == self.reader.cursor {
      return "7;34";
    }
    if let Some(selection) = self.reader.selection {
      let (first, last) =
        crate::motion::range(selection.anchor, self.reader.cursor);
      let selected = if selection.kind == SelectionKind::Line {
        (position.page, position.line) >= (first.page, first.line)
          && (position.page, position.line) <= (last.page, last.line)
      } else {
        position >= first && position <= last
      };
      if selected {
        return "44;37";
      }
    }
    if !self.search.hidden
      && self
        .search
        .hits
        .iter()
        .any(|hit| position >= hit.first && position <= hit.last)
    {
      return "43;30";
    }
    "0"
  }
}
