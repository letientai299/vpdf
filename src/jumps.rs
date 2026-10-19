use crate::app::App;
use crate::document::Target;
use crate::find::Phase;
use crate::panel::layout::Layout;
use crate::panel::{Destination, Row};
use crate::text::{Line, Position};
use vpdf_reader::reader::JumpDescription;

impl App {
  pub(crate) fn search_description(
    &self,
    position: Position,
  ) -> JumpDescription {
    let mut description = JumpDescription::Search {
      query: self.search.query().clone(),
      ordinal: None,
    };
    if matches!(self.search.phase(), Phase::Complete) {
      description.finish_search(
        self.search.query(),
        self.search.hits(),
        position,
      );
    }
    description
  }

  pub(crate) fn link_description(
    &mut self,
    target: &Target,
    position: Position,
  ) -> JumpDescription {
    let Ok(text) = self.doc.text(position.page) else {
      return JumpDescription::Link(String::new());
    };
    for heading in &self.doc.outline {
      let at = match heading.target {
        Target::Point { page, x, y } if page == position.page => {
          text.nearest(page, x, y)
        }
        Target::Page(page) if page == position.page => {
          Position { page, ..Position::default() }
        }
        _ => continue,
      };
      if at == position && !heading.title.trim().is_empty() {
        return JumpDescription::Heading(heading.title.trim().into());
      }
    }
    JumpDescription::Link(crate::links::excerpt(&text, target))
  }

  pub(crate) fn jump_rows(&mut self) -> Vec<Row> {
    let jumps: Vec<_> = self
      .reader
      .described_jumps()
      .map(|(position, description)| (position, description.cloned()))
      .collect();
    jumps
      .into_iter()
      .map(|(position, description)| {
        let text = self.doc.text(position.page).ok();
        let line = text.as_ref().and_then(|text| text.lines.get(position.line));
        let label = description.as_ref().map_or_else(
          || {
            line
              .map(Line::text)
              .filter(|text| !text.trim().is_empty())
              .unwrap_or_else(|| "No text".into())
          },
          label,
        );
        let word = line
          .and_then(|line| {
            let (first, last) = line.run(position.glyph)?;
            let word: String =
              line.glyphs[first..=last].iter().map(|glyph| glyph.ch).collect();
            Some(if word.trim().is_empty() {
              "whitespace".into()
            } else {
              format!("“{word}”")
            })
          })
          .unwrap_or_else(|| "—".into());
        let page = position.page.0 + 1;
        let name = self.doc.get_label(position.page).unwrap_or_default();
        let page = if name.is_empty() || name == page.to_string() {
          page.to_string()
        } else {
          format!("{name} (#{page})")
        };
        Row::new(
          label.split_whitespace().collect::<Vec<_>>().join(" "),
          format!(
            "p. {page} · L{} C{} · word {word}",
            position.line + 1,
            position.glyph + 1
          ),
          Destination::Jump { position, description },
        )
      })
      .collect()
  }
}

fn label(description: &JumpDescription) -> String {
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
  use crate::find::Query;
  use crate::keys::{Action, Input, PanelKind};
  use crate::panel::{Panel, Style};
  use crate::search::Hit;
  use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use unicode_width::UnicodeWidthStr;
  use vpdf_reader::motion::Motion;
  use vpdf_terminal::terminal;

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
  fn expanded_jumps_keep_context() -> crate::Result<()> {
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
    panel.list(&mut Vec::new(), terminal::At::default(), 32, 20)?;
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
    Ok(())
  }
}
