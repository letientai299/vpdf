use crate::panel::{Destination, Row, Style, key_width};
use crate::panel_layout::{Layout, Line, wrap};

impl Layout {
  pub fn help(rows: &[Row], columns: usize) -> Self {
    let mut layout = Self { columns, lines: Vec::new(), starts: Vec::new() };
    let mut start = 0;
    while start < rows.len() {
      let end = rows[start + 1..]
        .iter()
        .position(|row| row.style == Style::Heading)
        .map_or(rows.len(), |offset| start + 1 + offset);
      let keys =
        key_width(&rows[start..end], columns).min(columns.saturating_sub(3));
      for (index, row) in rows.iter().enumerate().take(end).skip(start) {
        layout.starts.push(layout.lines.len());
        layout.extend(index, row, keys);
      }
      start = end;
    }
    layout.starts.push(layout.lines.len());
    layout
  }

  fn extend(&mut self, source: usize, row: &Row, keys: usize) {
    let push = |lines: &mut Vec<Line>, label, style| {
      lines.push(Line {
        source,
        keys,
        clipped: false,
        row: Row {
          label,
          style,
          detail: String::new(),
          destination: Destination::None,
        },
      });
    };
    let Some((key, text)) =
      row.label.split_once(": ").filter(|_| row.style == Style::Key)
    else {
      for line in wrap(&row.label, self.columns) {
        push(&mut self.lines, line, row.style);
      }
      return;
    };
    let style = match key.split_whitespace().next() {
      Some("N" | "C" | "V" | "C/V" | "P") => Style::ModeKey,
      _ if key.starts_with(':') => Style::CommandKey,
      _ => Style::Key,
    };
    let indent = (keys + 2).min(self.columns.saturating_sub(1));
    let key_lines = wrap(key, keys.max(1));
    let text_lines = wrap(text, self.columns.saturating_sub(indent));
    if self.columns < 4 {
      for line in wrap(key, self.columns) {
        push(&mut self.lines, format!("{line}: "), Style::Key);
      }
      for line in wrap(text, self.columns) {
        push(&mut self.lines, line, Style::Plain);
      }
      return;
    }
    for index in 0..key_lines.len().max(text_lines.len()) {
      let text = text_lines.get(index).map_or("", String::as_str);
      if let Some(key) = key_lines.get(index) {
        let style =
          if index > 0 && style == Style::ModeKey { Style::Key } else { style };
        push(&mut self.lines, format!("{key}: {text}"), style);
      } else {
        push(
          &mut self.lines,
          format!("{}{text}", " ".repeat(indent)),
          Style::Plain,
        );
      }
    }
  }
}

pub(crate) fn key_styles(
  key: &str,
  style: Style,
) -> Vec<(usize, usize, &'static str)> {
  if style == Style::ModeKey {
    return vec![(0, key.find(' ').unwrap_or(key.len()), crate::theme::MODE)];
  }
  if style != Style::CommandKey {
    return Vec::new();
  }
  let mut spans = Vec::new();
  let mut start = None;
  for (index, ch) in key.char_indices() {
    match ch {
      '<' | '[' => start = Some((index, ch)),
      '>' | ']' => {
        if let Some((from, open)) = start.take()
          && matches!((open, ch), ('<', '>') | ('[', ']'))
        {
          spans.push((from, index + 1, crate::theme::ARGUMENT));
        }
      }
      _ => {}
    }
  }
  spans
}

#[cfg(test)]
mod tests {
  use super::*;
  use unicode_width::UnicodeWidthStr;

  #[test]
  fn sections_align_independently() {
    let rows =
      ["# A", "long key: first", "x: next", "# B", "y: last"].map(Row::parse);
    let layout = Layout::help(&rows, 60);
    let widths: Vec<_> = layout.lines.iter().map(|line| line.keys).collect();
    assert_eq!(widths, [8, 8, 8, 1, 1]);
  }

  #[test]
  fn descriptions_keep_their_indent() {
    let rows = ["# A", "j: one two three four five"].map(Row::parse);
    let layout = Layout::help(&rows, 14);
    let labels: Vec<_> =
      layout.lines.iter().map(|line| &*line.row.label).collect();
    assert_eq!(labels, ["A", "j: one two", "   three four", "   five"]);
    assert_eq!(layout.starts, [0, 1, 4]);
    assert!(layout.lines[1..].iter().all(|line| line.source == 1));
  }

  #[test]
  fn wrapping_preserves_words_and_unicode() {
    for (text, columns, want) in [
      ("alpha beta gamma", 10, vec!["alpha beta", "gamma"]),
      ("abcdefghij", 4, vec!["abcd", "efgh", "ij"]),
      ("界界界 界", 4, vec!["界界", "界", "界"]),
      ("e\u{301}cho words", 4, vec!["e\u{301}cho", "word", "s"]),
      ("", 4, vec![""]),
    ] {
      assert_eq!(wrap(text, columns), want, "{text}");
    }
  }

  #[test]
  fn mode_labels_differ_from_keys() -> crate::Result<()> {
    let rows =
      ["C: Cursor mode", "C/V h j: glyphs", "v V: select", "<C-x>: Ctrl"]
        .map(Row::parse);
    let layout = Layout::help(&rows, 48);
    for (index, line) in layout.lines.iter().enumerate() {
      let mut out = Vec::new();
      line.row.write(&mut out, crate::theme::RESET, 48, line.keys, "")?;
      let out = String::from_utf8(out)?;
      assert_eq!(out.contains(crate::theme::MODE), index < 2);
      if index == 1 {
        assert!(out.contains(&format!("{}C/V", crate::theme::MODE)));
        assert!(out.contains(&format!("{} h j", crate::theme::KEY)));
      }
    }
    Ok(())
  }

  #[test]
  fn arguments_differ_from_key_notation() -> crate::Result<()> {
    let rows = [
      ":edit <file>: open",
      ":split [file]: split",
      "<C-d><C-u>: scroll",
      "[count]gg: jump",
    ]
    .map(Row::parse);
    let layout = Layout::help(&rows, 48);
    for (index, line) in layout.lines.iter().enumerate() {
      let mut out = Vec::new();
      line.row.write(&mut out, crate::theme::RESET, 48, line.keys, "")?;
      let out = String::from_utf8(out)?;
      assert_eq!(out.contains(crate::theme::ARGUMENT), index < 2);
    }
    assert_eq!(
      key_styles(":edit <file>", Style::CommandKey),
      [(6, 12, crate::theme::ARGUMENT)]
    );
    assert_eq!(
      key_styles(":split [file]", Style::CommandKey),
      [(7, 13, crate::theme::ARGUMENT)]
    );
    Ok(())
  }

  #[test]
  fn help_fits_narrow_panes() {
    let rows: Vec<_> =
      crate::panel::help().iter().map(|s| Row::parse(s)).collect();
    for columns in [4, 10, 24, 40, 63] {
      let layout = Layout::help(&rows, columns);
      for line in &layout.lines {
        let width = match line.row.label.split_once(": ") {
          Some((key, text))
            if matches!(
              line.row.style,
              Style::Key | Style::ModeKey | Style::CommandKey
            ) =>
          {
            line.keys.max(key.width()) + 2 + text.width()
          }
          _ => line.row.label.width(),
        };
        assert!(width <= columns, "{columns}: {}", line.row.label);
      }
    }
  }
}
