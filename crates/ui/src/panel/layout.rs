use crate::panel::{Destination, Row, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vpdf_model::scene::{Draw, Op};
use vpdf_model::style::Role;
use vpdf_model::text::clean;

/// Title cells a glyph, key, `+N` or indent must leave.
const TITLE: usize = 4;

/// One list row's columns around its title.
pub(crate) struct Meta {
  pub indent: usize,
  pub glyph: Option<(char, Role)>,
  /// The key padded to the key column and its gap.
  pub key: String,
  /// Cells left for the title.
  pub room: usize,
  /// Rows hidden under a folded heading.
  pub badge: String,
  /// The page behind its 1-cell gap.
  pub page: String,
}

impl Meta {
  /// Narrow rows drop `+N`, then the page, the glyph and the key.
  pub(crate) fn new(
    row: &Row,
    available: usize,
    key_column: usize,
    depth: usize,
    hidden: usize,
  ) -> Self {
    let mut room = available;
    let mut kept = true;
    let mut keep = |cells: usize, title: usize| {
      kept = kept && room >= cells + title;
      if kept {
        room -= cells;
      }
      kept
    };
    let key = if key_column > 0 && keep(key_column + 1, TITLE) {
      let key = clean(&row.key, key_column);
      format!("{key}{}", " ".repeat(key_column + 1 - key.width()))
    } else {
      String::new()
    };
    let glyph = row.kind.glyph().filter(|_| keep(2, TITLE));
    let label = clean(&row.detail, available);
    let page = if !label.is_empty()
      && label.width() == row.detail.width()
      && keep(label.width() + 1, 1)
    {
      format!(" {label}")
    } else {
      String::new()
    };
    let badge = format!(" +{hidden}");
    let lead = (key_column == 0 || !key.is_empty())
      && glyph.is_some() == row.kind.glyph().is_some();
    // Contents keeps `+N` even when the page drops.
    let badge = if hidden > 0 && lead && room >= badge.width() + TITLE {
      room -= badge.width();
      badge
    } else {
      String::new()
    };
    let indent =
      depth.min(12).saturating_mul(2).min(room.saturating_sub(TITLE));
    Self { indent, glyph, key, room: room - indent, badge, page }
  }

  /// Paints the glyph and key in `base`, accented when unselected.
  pub(crate) fn lead<'a>(&self, scene: &mut impl Draw<'a>, base: Role) {
    let accent = |role| super::rows::accented(Some(base)).then_some(role);
    if let Some((glyph, role)) = self.glyph {
      super::rows::styles(scene, Some(base), accent(role));
      scene.text(&format!("{glyph} "));
    }
    if !self.key.is_empty() {
      super::rows::styles(scene, Some(base), accent(Role::Mode));
      scene.text(&self.key);
    }
    scene.push(Op::Style(base));
  }
}

/// Widest row key, at most a third of `columns`.
pub(crate) fn key_cells(rows: &[Row], columns: usize) -> usize {
  rows.iter().map(|row| row.key.width()).max().unwrap_or(0).min(columns / 3)
}

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
    let key_column = key_cells(rows, columns);
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      let room = Meta::new(row, columns, key_column, 0, 0).room;
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

  /// The row's label lines; the first keeps its glyph, key and page.
  pub(crate) fn label(
    &mut self,
    source: usize,
    row: &Row,
    key_column: usize,
    selected: usize,
  ) {
    let Meta { room, page, .. } =
      Meta::new(row, self.columns, key_column, 0, 0);
    let (labels, cut) = if source == selected {
      (wrap(&row.label, room), false)
    } else {
      let (label, cut) = vpdf_model::text::clipped(&row.label, room);
      (vec![label], cut)
    };
    for (index, label) in labels.into_iter().enumerate() {
      let line = Row::new(label, String::new(), Destination::None);
      self.lines.push(Line {
        source,
        keys: 0,
        clipped: cut,
        row: if index == 0 {
          Row {
            detail: page.trim_start().to_owned(),
            key: row.key.clone(),
            kind: row.kind,
            ..line
          }
        } else {
          line
        },
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
