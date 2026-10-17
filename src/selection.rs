use crate::Result;
use crate::document::MIB;
use crate::motion::{covers, range};
use crate::reader::{Operator, Reader, SelectionKind, TextSource, Yank};
use crate::text::{PageId, Position};
use anyhow::ensure;
use std::sync::Arc;

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

  /// `Y`: yanks `count` lines, or every line the selection touches.
  pub fn yank_lines(
    &mut self,
    count: usize,
    register: char,
    source: &mut impl TextSource,
  ) -> Result<()> {
    if let Some(selection) = self.selection {
      return self.yank_range(
        selection.anchor,
        self.cursor,
        true,
        register,
        source,
      );
    }
    self.operator = Some(Operator { count, register });
    self.yank(1, register, source)
  }

  pub fn yank_range(
    &mut self,
    first: Position,
    last: Position,
    linewise: bool,
    register: char,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let range = range(first, last);
    let (first, last) = range;
    let mut output = String::new();
    for page in first.page.0..=last.page.0 {
      let text = source.page_text(PageId(page))?;
      let skip = if page == first.page.0 { first.line } else { 0 };
      for (line, row) in text.lines.iter().enumerate().skip(skip) {
        if page == last.page.0 && line > last.line {
          break;
        }
        if !output.is_empty() {
          output.push('\n');
        }
        let at = |glyph| Position { page: PageId(page), line, glyph };
        let glyphs = row.glyphs.iter().enumerate();
        for (_, ch) in glyphs.filter(|(g, _)| covers(range, linewise, at(*g))) {
          ensure!(
            output.len() + ch.ch.len_utf8() < 8 * MIB,
            "Selection exceeds 8 MiB"
          );
          output.push(ch.ch);
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
    let output: Arc<str> = if register.is_ascii_uppercase() {
      let old = self.registers.get(&name).map_or("", |old| &**old);
      ensure!(old.len() + text.len() < 8 * MIB, "Register exceeds 8 MiB");
      format!("{old}{text}").into()
    } else {
      text.into()
    };
    for key in [name, '"', '0'] {
      self.registers.insert(key, Arc::clone(&output), output.len());
    }
    Ok(())
  }

  pub fn object(
    &mut self,
    around: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    self.select_object(crate::objects::Object::Word, around, 1, source)
  }
}
