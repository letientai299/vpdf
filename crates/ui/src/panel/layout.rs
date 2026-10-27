use crate::panel::{Destination, Row, RowKind, Style};
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
    let glyph = row.glyph().filter(|_| keep(2, TITLE));
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
      && glyph.is_some() == row.glyph().is_some();
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

  pub(super) fn trail<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    plain: Role,
    used: usize,
    progress: bool,
  ) {
    let detail = if progress {
      self.page.strip_prefix(' ').unwrap_or(&self.page)
    } else {
      self.page.trim_start()
    };
    let gap = usize::from(!detail.is_empty());
    scene.extend([
      Op::Style(plain),
      Op::Spaces((self.room + gap).saturating_sub(used)),
    ]);
    if progress {
      super::recent::scene_progress(scene, detail, plain);
    } else {
      if plain == Role::Reset {
        scene.push(Op::Style(Role::Muted));
      }
      scene.text(detail);
    }
    scene.push(Op::Style(Role::Reset));
  }

  /// Cells the glyph and key take before the title.
  pub(crate) fn lead_cells(&self) -> usize {
    self.glyph.map_or(0, |(glyph, _)| glyph.width().unwrap_or(1) + 1)
      + self.key.width()
  }

  /// Paints the glyph and key in `base`, accented when unselected, with
  /// `query` marked in the key.
  pub(crate) fn lead<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    base: Role,
    query: &str,
  ) {
    let accent = |role| super::rows::accented(Some(base)).then_some(role);
    if let Some((glyph, role)) = self.glyph {
      super::rows::styles(scene, Some(base), accent(role));
      scene.text(&format!("{glyph} "));
    }
    if !self.key.is_empty() {
      super::rows::marked(
        scene,
        &self.key,
        query,
        Some(base),
        accent(Role::Mode),
      );
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
  /// The selected row the lines expand, when they depend on it.
  pub selected: Option<usize>,
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
  pub(crate) fn empty(columns: usize) -> Self {
    Self { columns, lines: Vec::new(), starts: Vec::new(), selected: None }
  }

  /// One line per row, for a projected list.
  pub(crate) fn plain(rows: &[Row], columns: usize) -> Self {
    let mut layout = Self::empty(columns);
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

  pub(crate) fn recent(rows: &[Row], columns: usize) -> Self {
    let mut layout = Self::empty(columns);
    for (source, row) in rows.iter().enumerate() {
      if source > 0 && matches!(row.destination, Destination::Directory(_)) {
        layout.lines.push(Line {
          source: source - 1,
          keys: 0,
          clipped: false,
          row: Row::new(String::new(), String::new(), Destination::None),
        });
      }
      layout.starts.push(layout.lines.len());
      layout.lines.push(Line {
        source,
        row: row.clone(),
        keys: 0,
        clipped: false,
      });
    }
    layout.starts.push(layout.lines.len());
    layout
  }

  /// Each row's label lines, then the dim text `second` gives it, which
  /// replaces the label's detail; the selected row wraps both.
  pub(crate) fn expanded(
    rows: &[Row],
    columns: usize,
    selected: usize,
    second: impl Fn(usize, &Row) -> Option<String>,
  ) -> Self {
    let mut layout = Self { selected: Some(selected), ..Self::empty(columns) };
    let keys = key_cells(rows, columns);
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      let Some(text) = second(source, row) else {
        layout.label(source, row, keys, selected);
        continue;
      };
      let label = Row { detail: String::new(), ..row.clone() };
      let lead = layout.label(source, &label, keys, selected);
      layout.secondary(source, &text, selected, lead);
    }
    layout.starts.push(layout.lines.len());
    layout
  }

  /// Annotations; the selected one shows its note after `✎`.
  pub(crate) fn annotations(
    rows: &[Row],
    columns: usize,
    selected: usize,
  ) -> Self {
    let glyph = RowKind::Note.glyph().map_or('✎', |(glyph, _)| glyph);
    Self::expanded(rows, columns, selected, |source, row| {
      let note = row.note().filter(|_| source == selected)?;
      Some(format!("{glyph} {note}"))
    })
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

  /// The row's label lines; the first keeps its glyph, key and page,
  /// and the rest start under the title. Rows indent by depth, as the
  /// tree paints them, outside the lines. Returns the indent and the
  /// title's column within the lines.
  pub(crate) fn label(
    &mut self,
    source: usize,
    row: &Row,
    key_column: usize,
    selected: usize,
  ) -> (usize, usize) {
    let meta = Meta::new(row, self.columns, key_column, row.depth, 0);
    let lead = meta.lead_cells();
    let Meta { room, page, .. } = meta;
    let (labels, cut) = if source == selected {
      (wrap(&row.label, room), false)
    } else {
      let (label, cut) = vpdf_model::text::clipped(&row.label, room);
      (vec![label], cut)
    };
    for (index, label) in labels.into_iter().enumerate() {
      let label =
        if index == 0 { label } else { format!("{}{label}", " ".repeat(lead)) };
      // Continuation lines keep the row's style, such as muted.
      let line = Row {
        style: row.style,
        ..Row::new(label, String::new(), Destination::None)
      };
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
    (meta.indent, lead)
  }

  /// Dim lines of `text`, `lead` cells in past a row's unpainted
  /// `indent`.
  pub(crate) fn secondary(
    &mut self,
    source: usize,
    text: &str,
    selected: usize,
    (indent, lead): (usize, usize),
  ) {
    let columns = self.columns.saturating_sub(indent);
    let indent = columns.saturating_sub(1).min(lead);
    let room = columns.saturating_sub(indent);
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
