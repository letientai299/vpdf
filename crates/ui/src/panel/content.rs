use super::{Destination, Row, Style};
use crate::text::{Line, PageId, Position, Text};
use std::path::Path;
use vpdf_model::document::Heading;
use vpdf_reader::reader::JumpDescription;
use vpdf_search::search::Hit;

impl Row {
  pub fn jump(
    position: Position,
    description: Option<JumpDescription>,
    line: Option<&Line>,
    name: &str,
  ) -> Self {
    let label = description.as_ref().map_or_else(
      || {
        line
          .map(Line::text)
          .filter(|text| !text.trim().is_empty())
          .unwrap_or_else(|| "No text".into())
      },
      super::jumps::label,
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

    let page = if name.is_empty() || name == page.to_string() {
      page.to_string()
    } else {
      format!("{name} (#{page})")
    };
    Self::new(
      label.split_whitespace().collect::<Vec<_>>().join(" "),
      format!(
        "p. {page} · L{} C{} · word {word}",
        position.line + 1,
        position.glyph + 1
      ),
      Destination::Jump { position, description },
    )
  }

  pub fn outline(item: &Heading, label: String) -> Self {
    Self {
      style: if item.depth == 0 { Style::Heading } else { Style::Plain },
      ..Self::new(
        format!("{}{}", "  ".repeat(item.depth.min(12)), item.title),
        label,
        Destination::Link(item.target.clone()),
      )
    }
  }

  pub fn result(hit: &Hit, label: String) -> Self {
    Self::new(hit.excerpt.clone(), label, Destination::Position(hit.first))
  }

  pub fn local_mark(
    name: char,
    position: Position,
    text: Option<&Text>,
  ) -> Self {
    let line = text.and_then(|text| text.lines.get(position.line));
    let line =
      line.map_or_else(String::new, |line| line.text().trim().to_owned());
    Self::new(
      format!("{name}  {line}"),
      format!("p{}", position.page.0 + 1),
      Destination::Position(position),
    )
  }

  pub fn global_mark(name: char, position: Position, path: &Path) -> Self {
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    Self::new(
      format!("{name}  {file}"),
      format!("p{}", position.page.0 + 1),
      Destination::Mark(name),
    )
  }

  pub fn recent(path: &Path, page: PageId, home: Option<&Path>) -> Self {
    let label = match home.map(|home| path.strip_prefix(home)) {
      Some(Ok(rest)) => Path::new("~").join(rest),
      _ => path.to_path_buf(),
    };
    Self {
      style: Style::Path,
      ..Self::new(
        label.display().to_string(),
        format!("p{}", page.0 + 1),
        Destination::File(path.to_path_buf()),
      )
    }
  }
}
