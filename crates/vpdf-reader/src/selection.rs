use crate::Result;
use crate::motion::{covers, range};
use crate::reader::{
  Operator, Reader, SelectionKind, TextSource, YANK_LIMIT, Yank,
};
use anyhow::ensure;
use std::sync::Arc;
use vpdf_model::reflow::{Join, joins, reflow};
use vpdf_model::text::{PageId, Position};

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
      self.yank_count(op, count, true, source)?;
    } else {
      self.operator = Some(Operator { count, register });
      self.message = "y …".into();
    }
    Ok(())
  }

  /// `yY`: yanks lines as laid out, without joining them.
  pub fn yank_raw(
    &mut self,
    count: usize,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let Some(op) = self.operator.take() else { return Ok(()) };
    self.yank_count(op, count, false, source)
  }

  /// Yanks `count` times the operator's lines from the cursor.
  fn yank_count(
    &mut self,
    op: Operator,
    count: usize,
    joined: bool,
    source: &mut impl TextSource,
  ) -> Result<()> {
    let first = self.cursor;
    for _ in 1..count.saturating_mul(op.count).min(10_000) {
      self.step(1, true, source)?;
    }
    let last = self.cursor;
    self.cursor = first;
    if !joined {
      return self.yank_range(first, last, true, op.register, source);
    }
    let range = range(first, last);
    let lines = covered_lines(range, true, source)?;
    let text = reflow(&lines, &context(range, source)?);
    self.store(op.register, text + "\n")?;
    self.end_selection();
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
    let lines = covered_lines(range(first, last), linewise, source)?;
    let mut output =
      lines.into_iter().map(|(_, line)| line).collect::<Vec<_>>().join("\n");
    if linewise {
      output.push('\n');
    }
    self.store(register, output)?;
    self.end_selection();
    Ok(())
  }

  /// First-page selection text, joined like search.
  pub fn search_text(
    &self,
    source: &mut impl TextSource,
  ) -> Result<Option<String>> {
    let Some(selection) = self.selection else { return Ok(None) };
    let (first, mut last) = range(selection.anchor, self.cursor);
    if last.page != first.page {
      last = source.page_text(first.page)?.clamp(Position {
        line: usize::MAX,
        glyph: usize::MAX,
        ..first
      });
    }
    let linewise = selection.kind == SelectionKind::Line;
    let mut text = String::new();
    for (_, line) in covered_lines((first, last), linewise, source)? {
      text.push_str(&line);
      if vpdf_search::search::hyphenated(&text) {
        text.pop();
      } else {
        text.push(' ');
      }
    }
    let words: Vec<_> = text.split_whitespace().collect();
    Ok((!words.is_empty()).then(|| words.join(" ")))
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
      ensure!(
        old.len() + text.len() < YANK_LIMIT,
        "Register exceeds {} MiB",
        YANK_LIMIT >> 20
      );
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

/// Text of each line `range` covers, with how it joins the previous.
fn covered_lines(
  range: (Position, Position),
  linewise: bool,
  source: &mut impl TextSource,
) -> Result<Vec<(Join, String)>> {
  let (first, last) = range;
  let mut lines = Vec::new();
  let mut size = 0;
  for page in first.page.0..=last.page.0 {
    let text = source.page_text(PageId(page))?;
    let skip = if page == first.page.0 { first.line } else { 0 };
    let end = if page == last.page.0 {
      (last.line + 1).min(text.lines.len())
    } else {
      text.lines.len()
    };
    let joins = joins(&text, skip..end);
    for (line, row) in text.lines.iter().enumerate().take(end).skip(skip) {
      let at = |glyph| Position { page: PageId(page), line, glyph };
      let glyphs = row.glyphs.iter().enumerate();
      let output: String = glyphs
        .filter(|(g, _)| covers(range, linewise, at(*g)))
        .map(|(_, ch)| ch.ch)
        .collect();
      if lines.is_empty() && output.is_empty() {
        continue;
      }
      size += output.len() + 1;
      ensure!(size < YANK_LIMIT, "Selection exceeds {} MiB", YANK_LIMIT >> 20);
      lines.push((joins[line - skip], output));
    }
  }
  Ok(lines)
}

/// Text of the pages `range` touches, where split words show whole.
fn context(
  (first, last): (Position, Position),
  source: &mut impl TextSource,
) -> Result<String> {
  let mut context = String::new();
  for page in first.page.0..=last.page.0 {
    context += &source.page_text(PageId(page))?.plain();
    context.push('\n');
  }
  Ok(context)
}
