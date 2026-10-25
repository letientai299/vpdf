use crate::panel::{Destination, Row, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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
  pub(crate) fn plain(rows: &[Row], columns: usize) -> Self {
    let mut layout = Self { columns, lines: Vec::new(), starts: Vec::new() };
    let keys = super::key_width(rows, columns);
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      let detail = vpdf_model::text::clean(&row.detail, columns / 3);
      let room = columns
        .saturating_sub(detail.width() + usize::from(!detail.is_empty()));
      layout.lines.push(Line {
        source,
        row: row.clone(),
        keys,
        clipped: row.label.width() > room,
      });
    }
    layout.starts.push(layout.lines.len());
    layout
  }

  pub(crate) fn project(
    mut self,
    projection: &super::tree::Projection,
  ) -> Self {
    if projection.sources().len() == projection.source_count() {
      return self;
    }
    self.lines.retain(|line| projection.position(line.source).is_some());
    self.starts.fill(0);
    for line in &self.lines {
      self.starts[line.source + 1] += 1;
    }
    for source in 1..self.starts.len() {
      self.starts[source] += self.starts[source - 1];
    }
    self
  }

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
      let (label, cut) = vpdf_model::text::clipped(label, room);
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
      vec![vpdf_model::text::clipped(text, room).0]
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
