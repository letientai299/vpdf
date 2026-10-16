use crate::Result;
use crate::document::MIB;
use crate::motion::range;
use crate::reader::{
  Operator, Reader, Selection, SelectionKind, TextSource, Yank,
};
use crate::text::{PageId, Position, class};
use anyhow::ensure;

impl Reader {
  pub fn yank(
    &mut self,
    count: usize,
    register: char,
    source: &mut impl TextSource,
  ) -> Result<()> {
    if let Some(selection) = self.selection {
      return self.yank_range(
        selection.anchor,
        self.cursor,
        selection.kind == SelectionKind::Line,
        register,
        source,
      );
    }
    if let Some(op) = self.operator.take() {
      let first = self.cursor;
      for _ in 1..count.saturating_mul(op.count).min(10_000) {
        self.step(1, true, source)?;
      }
      self.yank_range(first, self.cursor, true, op.register, source)?;
      self.cursor = first;
    } else {
      self.operator = Some(Operator { count, register });
      self.message = "y …".into();
    }
    Ok(())
  }

  pub fn yank_range(
    &mut self,
    first: Position,
    last: Position,
    linewise: bool,
    register: char,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let (first, last) = range(first, last);
    let mut output = String::new();
    for page in first.page.0..=last.page.0 {
      let text = source.page_text(PageId(page))?;
      for (line, row) in text.lines.iter().enumerate() {
        if page == first.page.0 && line < first.line {
          continue;
        }
        if page == last.page.0 && line > last.line {
          break;
        }
        if !output.is_empty() {
          output.push('\n');
        }
        for (glyph, ch) in row.glyphs.iter().enumerate() {
          let pos = Position { page: PageId(page), line, glyph };
          if linewise || (pos >= first && pos <= last) {
            ensure!(
              output.len() + ch.ch.len_utf8() < 8 * MIB,
              "Selection exceeds 8 MiB"
            );
            output.push(ch.ch);
          }
        }
      }
    }
    if linewise {
      output.push('\n');
    }
    self.store(register, output)?;
    self.end_selection();
    Ok(())
  }

  /// Saves yanked text to `register` and queues clipboard delivery.
  pub fn store(&mut self, register: char, text: String) -> Result<()> {
    self.save_register(register, &text)?;
    self.message = format!("Yanked {} characters", text.chars().count());
    self.pending_yank = Some(Yank { text, register });
    Ok(())
  }

  fn save_register(&mut self, register: char, text: &str) -> Result<()> {
    let name = register.to_ascii_lowercase();
    let output = if register.is_ascii_uppercase() {
      let old = self.registers.get(&name).cloned().unwrap_or_default();
      ensure!(old.len() + text.len() < 8 * MIB, "Register exceeds 8 MiB");
      old + text
    } else {
      text.to_owned()
    };
    for key in [name, '"', '0'] {
      self.registers.insert(key, output.clone(), output.len());
    }
    Ok(())
  }

  pub fn object(
    &mut self,
    around: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let text = source.page_text(self.cursor.page)?;
    let Some(line) = text.lines.get(self.cursor.line) else { return Ok(()) };
    let Some(glyph) = line.glyphs.get(self.cursor.glyph) else { return Ok(()) };
    let kind = class(glyph.ch);
    let mut a = self.cursor.glyph;
    let mut b = a;
    while a > 0 && class(line.glyphs[a - 1].ch) == kind {
      a -= 1;
    }
    while b + 1 < line.glyphs.len() && class(line.glyphs[b + 1].ch) == kind {
      b += 1;
    }
    if around {
      while b + 1 < line.glyphs.len() && class(line.glyphs[b + 1].ch) == 0 {
        b += 1;
      }
    }
    let first = Position { glyph: a, ..self.cursor };
    let last = Position { glyph: b, ..self.cursor };
    if let Some(op) = self.operator.take() {
      self.yank_range(first, last, false, op.register, source)?;
    } else {
      self.selection =
        Some(Selection { anchor: first, kind: SelectionKind::Character });
      self.cursor = last;
    }
    Ok(())
  }
}
