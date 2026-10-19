use super::{
  Destination, Result, Row, Style, UnicodeWidthChar, UnicodeWidthStr, Write,
  terminal, theme,
};

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

/// Widest `keys:` column of `rows`.
pub fn key_width(rows: &[Row], inner: usize) -> usize {
  rows
    .iter()
    .filter(|row| {
      matches!(row.style, Style::Key | Style::ModeKey | Style::CommandKey)
    })
    .filter_map(|row| row.label.split_once(": "))
    .map(|(key, _)| key.width())
    .max()
    .unwrap_or(0)
    .min(inner / 2)
}

impl Row {
  /// Writes the label within `room` columns and returns its width.
  pub fn write(
    &self,
    out: &mut impl Write,
    base: &str,
    room: usize,
    keys: usize,
    query: &str,
  ) -> Result<usize> {
    let plain = [theme::RESET, theme::DIM, theme::HOVER, theme::HOVER_DIM]
      .contains(&base);
    match (self.style, self.label.split_once(": ")) {
      (Style::Key | Style::ModeKey | Style::CommandKey, Some((key, text))) => {
        let key = terminal::clean(key, room);
        let pad = (keys.max(key.width()) + 2 - key.width())
          .min(room.saturating_sub(key.width()));
        let text =
          terminal::clean(text, room.saturating_sub(key.width() + pad));
        let style = if plain { theme::KEY } else { "" };
        let mut written = 0;
        for (start, end, token) in crate::help::key_styles(&key, self.style) {
          marked(out, &key[written..start], query, &format!("{base}{style}"))?;
          marked(out, &key[start..end], query, &format!("{base}{token}"))?;
          written = end;
        }
        marked(out, &key[written..], query, &format!("{base}{style}"))?;
        write!(out, "{base}{}", " ".repeat(pad))?;
        marked(out, &text, query, base)?;
        Ok(key.width() + pad + text.width())
      }
      (Style::Path, _) => {
        let split = self.label.rfind('/').map_or(0, |slash| slash + 1);
        let (parent, name) = self.label.split_at(split);
        // The name keeps its room; the parent loses its left end first.
        let name = terminal::clean(name, room);
        let parent = tail(parent, room.saturating_sub(name.width()));
        let muted = if plain { theme::MUTED } else { "" };
        marked(out, &parent, query, &format!("{base}{muted}"))?;
        marked(out, &name, query, &format!("{base}{}", theme::FILE))?;
        write!(out, "{base}")?;
        Ok(parent.width() + name.width())
      }
      (Style::Muted, _) => {
        let text = terminal::clean(&self.label, room);
        let muted = if plain { theme::MUTED } else { "" };
        marked(out, &text, query, &format!("{base}{muted}"))?;
        Ok(text.width())
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

/// `text` within `columns`, cut from the left behind an ellipsis.
fn tail(text: &str, columns: usize) -> String {
  let text: String = crate::text::printable(text).collect();
  if text.width() <= columns {
    return text;
  }
  let Some(room) = columns.checked_sub(1) else { return String::new() };
  let mut width = 0;
  let kept: Vec<char> = text
    .chars()
    .rev()
    .take_while(|ch| {
      width += ch.width().unwrap_or(0);
      width <= room
    })
    .collect();
  std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

/// Writes `text` in `style` with query matches highlighted.
fn marked(
  out: &mut impl Write,
  text: &str,
  query: &str,
  style: &str,
) -> Result<()> {
  write!(out, "{style}")?;
  let text =
    if style.starts_with(theme::HOVER) || style.starts_with(theme::HOVER_DIM) {
      let title = text.trim_start_matches(' ');
      let indent = &text[..text.len() - title.len()];
      write!(out, "\x1b[24m{indent}{style}")?;
      title
    } else {
      text
    };
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
