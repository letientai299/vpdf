use crate::Result;
use crate::reader::{Reader, TextSource};
use crate::text::{PageId, Position};

impl Reader {
  pub fn paragraph(
    &mut self,
    forward: bool,
    source: &mut impl TextSource,
  ) -> Result<bool> {
    let start = self.cursor;
    let direction = if forward { 1 } else { -1 };
    let mut page = start.page.0 as i64;
    while page >= 0 && (page as usize) < source.page_count() {
      let id = PageId(page as usize);
      let text = source.page_text(id)?;
      let eligible = |line: usize| {
        let pos = Position { page: id, line, glyph: 0 };
        text.lines[line].paragraph
          && if forward { pos > start } else { pos < start }
      };
      let found = if forward {
        (0..text.lines.len()).find(|&line| eligible(line))
      } else {
        (0..text.lines.len()).rev().find(|&line| eligible(line))
      };
      if let Some(line) = found {
        self.cursor = Position { page: id, line, glyph: 0 };
        return Ok(true);
      }
      page += direction;
    }
    for index in 0..source.page_count() {
      let page = if forward { source.page_count() - 1 - index } else { index };
      let text = source.page_text(PageId(page))?;
      if !text.lines.is_empty() {
        self.edge(PageId(page), forward, source)?;
        break;
      }
    }
    Ok(false)
  }
}
