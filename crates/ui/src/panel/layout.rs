use crate::panel::{Destination, Row, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vpdf_terminal::terminal;

pub(crate) struct Line {
  pub source: usize,
  pub row: Row,
  pub keys: usize,
  /// Cut short of its source row's label.
  pub clipped: bool,
}

pub(crate) struct Layout {
  pub columns: usize,
  pub lines: Vec<Line>,
  pub starts: Vec<usize>,
}

pub fn wrap(mut text: &str, columns: usize) -> Vec<String> {
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

impl Layout {
  pub(crate) fn label(
    &mut self,
    source: usize,
    label: &str,
    detail: &str,
    selected: usize,
  ) {
    let detail = vpdf_model::text::clean(detail, self.columns / 3);
    let room = self
      .columns
      .saturating_sub(detail.width() + usize::from(!detail.is_empty()));
    let (labels, cut) = if source == selected {
      (wrap(label, room), false)
    } else {
      let (label, cut) = terminal::clipped(label, room);
      (vec![label], cut)
    };
    for (index, label) in labels.into_iter().enumerate() {
      self.lines.push(Line {
        source,
        keys: 0,
        clipped: cut,
        row: Row::new(
          label,
          if index == 0 { detail.clone() } else { String::new() },
          Destination::None,
        ),
      });
    }
  }

  pub(crate) fn secondary(
    &mut self,
    source: usize,
    text: &str,
    selected: usize,
  ) {
    let indent = self.columns.saturating_sub(1).min(2);
    let room = self.columns.saturating_sub(indent);
    let lines = if source == selected {
      wrap(text, room)
    } else {
      vec![terminal::clipped(text, room).0]
    };
    for line in lines {
      self.lines.push(Line {
        source,
        keys: 0,
        clipped: false,
        row: Row {
          style: Style::Muted,
          ..Row::new(
            format!("{}{line}", " ".repeat(indent)),
            String::new(),
            Destination::None,
          )
        },
      });
    }
  }
}
