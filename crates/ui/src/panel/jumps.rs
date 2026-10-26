use super::layout::Layout;
use super::{JumpDescription, Row, RowKind};
/// A jump's kind and title: a heading, a link's excerpt, or a search
/// with its ordinal.
pub(super) fn title(description: &JumpDescription) -> (RowKind, String) {
  match description {
    JumpDescription::Heading(title) => (RowKind::Section, title.clone()),
    JumpDescription::Link(excerpt) if excerpt.is_empty() => {
      (RowKind::Link, "Link".into())
    }
    JumpDescription::Link(excerpt) => (RowKind::Link, excerpt.clone()),
    JumpDescription::Search { query, ordinal } => {
      let delimiter = if query.backward { '?' } else { '/' };
      let count = ordinal.map_or_else(
        || "counting…".into(),
        |(index, total)| format!("{index}/{total}"),
      );
      // A long query still leaves room for the ordinal.
      let text = vpdf_model::text::clipped(&query.text, 12).0;
      (RowKind::Search, format!("{delimiter}{text}{delimiter} {count}"))
    }
  }
}

/// The key for a stop `offset` jumps from the cursor: `now`, `−2`, `+1`.
pub fn key(offset: isize) -> String {
  match offset {
    0 => "now".into(),
    ..0 => format!("−{}", offset.unsigned_abs()),
    _ => format!("+{offset}"),
  }
}

impl Layout {
  /// Each row with its position on a dim second line.
  pub(crate) fn jumps(rows: &[Row], columns: usize, selected: usize) -> Self {
    Self::expanded(rows, columns, selected, |_, row| Some(row.detail.clone()))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::keys::{Action, Input, PanelKind};
  use crate::panel::{Destination, Panel, Position, Style};
  use unicode_width::UnicodeWidthStr;
  use vpdf_model::input::{MouseButton, MouseEvent, MouseEventKind};
  use vpdf_reader::motion::Motion;
  use vpdf_search::query::Query;
  use vpdf_search::search::Hit;

  #[test]
  fn search_counts_survive_clipping() {
    let description = JumpDescription::Search {
      query: Query { text: "long search query ".repeat(8), ..Query::default() },
      ordinal: Some((2, 97)),
    };
    let (kind, label) = title(&description);
    let search = Row {
      key: key(-10),
      kind,
      ..Row::new(label, String::new(), Destination::None)
    };
    let rows =
      [Row::new("Other".into(), String::new(), Destination::None), search];
    // A narrow panel's text column, past the border and marker.
    let layout = Layout::jumps(&rows, 26, 0);
    let label = &layout.lines[layout.starts[1]].row.label;
    assert!(label.contains("2/97"), "{label}");
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
        Hit {
          first: position,
          last: position,
          excerpt: "needle".into(),
          matched: 0..0,
        }
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
        .stops()
        .filter(|(offset, ..)| *offset != 0)
        .map(|(.., description)| title(description.unwrap()).1)
        .collect()
    };
    let other = Query { text: "other".into(), ..query.clone() };
    reader.finish_search_jumps(&other, &hits);
    assert_eq!(labels(&reader), ["/needle/ counting…"]);
    reader.finish_search_jumps(&query, &hits);
    assert_eq!(labels(&reader), ["/needle/ 3/3"]);
    reader.history(true);
    assert_eq!(labels(&reader), ["/needle/ 2/3"]);
  }

  #[test]
  fn expanded_jumps_keep_context() {
    let label = "A line containing café and 世界";
    let detail = "piv L12 C8";
    let rows = || {
      vec![
        Row::new(label.into(), detail.into(), Destination::None),
        Row::new("Other".into(), "p5 L1 C1".into(), Destination::None),
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
    let press = MouseEvent::new(MouseEventKind::Down(MouseButton::Left), 4, 3);
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
