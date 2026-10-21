use super::layout::Layout;
use super::{JumpDescription, Row};
pub(super) fn label(description: &JumpDescription) -> String {
  match description {
    JumpDescription::Heading(title) => format!("Heading: {title}"),
    JumpDescription::Link(excerpt) => {
      if excerpt.is_empty() {
        "Link".into()
      } else {
        format!("Link: {excerpt}")
      }
    }
    JumpDescription::Search { query, ordinal } => {
      let delimiter = if query.backward { '?' } else { '/' };
      let count = ordinal.map_or_else(
        || "counting…".into(),
        |(index, total)| format!("{index}/{total}"),
      );
      format!("Search [{count}] {delimiter}{}{delimiter}", query.text)
    }
  }
}

impl Layout {
  pub(crate) fn jumps(rows: &[Row], columns: usize, selected: usize) -> Self {
    let mut layout = Self { columns, lines: Vec::new(), starts: Vec::new() };
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      layout.label(source, &row.label, "", selected);
      layout.secondary(source, &row.detail, selected);
    }
    layout.starts.push(layout.lines.len());
    layout
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::keys::{Action, Input, PanelKind};
  use crate::panel::{Destination, Panel, Position, Style};
  use unicode_width::UnicodeWidthStr;
  use vpdf_model::input::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use vpdf_reader::motion::Motion;
  use vpdf_search::query::Query;
  use vpdf_search::search::Hit;

  #[test]
  fn search_counts_survive_clipping() {
    let description = JumpDescription::Search {
      query: Query { text: "long search query ".repeat(8), ..Query::default() },
      ordinal: Some((2, 97)),
    };
    let rows = [
      Row::new("Other".into(), String::new(), Destination::None),
      Row::new(label(&description), String::new(), Destination::None),
    ];
    let layout = Layout::jumps(&rows, 28, 0);
    let label = &layout.lines[layout.starts[1]].row.label;
    assert!(label.contains("[2/97]"), "{label}");
  }

  #[test]
  fn completed_search_numbers_history() {
    use crate::text::PageId;
    use vpdf_reader::reader::Reader;
    let query = Query { text: "needle".into(), ..Query::default() };
    let hits: Vec<_> = [0, 2, 4]
      .into_iter()
      .map(|page| {
        let position = Position { page: PageId(page), ..Position::default() };
        Hit { first: position, last: position, excerpt: "needle".into() }
      })
      .collect();
    let mut reader = Reader::default();
    for hit in &hits[1..] {
      reader.cursor = hit.first;
      reader.describe_jump(Some(JumpDescription::Search {
        query: query.clone(),
        ordinal: None,
      }));
      if hit.first == hits[1].first {
        reader.remember();
      }
    }
    reader.history(false);
    let labels = |reader: &Reader| -> Vec<String> {
      reader
        .described_jumps()
        .map(|(_, description)| label(description.unwrap()))
        .collect()
    };
    let other = Query { text: "other".into(), ..query.clone() };
    reader.finish_search_jumps(&other, &hits);
    assert_eq!(labels(&reader), ["Search [counting…] /needle/"]);
    reader.finish_search_jumps(&query, &hits);
    assert_eq!(labels(&reader), ["Search [3/3] /needle/"]);
    reader.history(true);
    assert_eq!(labels(&reader), ["Search [2/3] /needle/"]);
  }

  #[test]
  fn expanded_jumps_keep_context() {
    let label = "A line containing café and 世界";
    let detail = "p. iv (#4) · L12 C8 · word “café”";
    let rows = || {
      vec![
        Row::new(label.into(), detail.into(), Destination::None),
        Row::new("Other".into(), "p. 5 · L1 C1".into(), Destination::None),
      ]
    };
    let compact = |value: &str| -> String {
      value.chars().filter(|ch| !ch.is_whitespace()).collect()
    };
    for columns in [8, 16, 32, 64] {
      let layout = Layout::jumps(&rows(), columns, 0);
      for (style, expected) in [(Style::Plain, label), (Style::Muted, detail)] {
        let shown: String = layout
          .lines
          .iter()
          .filter(|line| line.source == 0 && line.row.style == style)
          .map(|line| line.row.label.as_str())
          .collect();
        assert_eq!(compact(&shown), compact(expected));
      }
      assert!(
        layout.lines.iter().all(|line| { line.row.label.width() <= columns })
      );
      assert_eq!(
        layout.lines.iter().filter(|line| line.source == 1).count(),
        2
      );
    }
    let mut panel = Panel::new(PanelKind::Jumps, rows());
    panel.scene_list(
      &mut Vec::new(),
      vpdf_model::geometry::At::default(),
      32,
      20,
    );
    let press = MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 4,
      row: 3,
      modifiers: KeyModifiers::NONE,
    };
    assert_eq!(panel.pointer(press), Some(0));
    panel.navigate(
      Input {
        action: Action::Motion(Motion::Down),
        count: 1,
        explicit: false,
        register: '"',
      },
      20,
    );
    assert_eq!(panel.selected, 1);
    panel.begin_search(false);
    panel.preview("café");
    assert_eq!(panel.selected, 0);
  }
}
