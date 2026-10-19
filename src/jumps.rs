use crate::app::App;
use crate::document::Target;
use crate::text::{Line, Position};
use vpdf_reader::reader::JumpDescription;
use vpdf_ui::find::Phase;
use vpdf_ui::panel::jump_label as label;
use vpdf_ui::panel::{Destination, Row};

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
