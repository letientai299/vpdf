use crate::panel::Row;
use unicode_width::UnicodeWidthChar;

pub(crate) struct Line {
  pub source: usize,
  pub row: Row,
  pub keys: usize,
}

pub(crate) struct Layout {
  pub columns: usize,
  pub lines: Vec<Line>,
  pub starts: Vec<usize>,
}

pub(crate) fn wrap(mut text: &str, columns: usize) -> Vec<String> {
  let mut lines = Vec::new();
  while !text.is_empty() {
    let mut width = 0;
    let end = text
      .char_indices()
      .take_while(|(_, ch)| {
        width += ch.width().unwrap_or(0);
        width <= columns.max(1)
      })
      .last()
      .map_or(text.len(), |(index, ch)| index + ch.len_utf8());
    if end == text.len() {
      lines.push(text.to_owned());
      break;
    }
    let split = if text[end..].starts_with(char::is_whitespace) {
      Some(end)
    } else {
      text[..end].rfind(' ').filter(|&index| index > 0)
    };
    let end = split
      .unwrap_or_else(|| text[..end].rfind('>').map_or(end, |index| index + 1));
    lines.push(text[..end].to_owned());
    text = text[end..].trim_start();
  }
  if lines.is_empty() {
    lines.push(String::new());
  }
  lines
}
