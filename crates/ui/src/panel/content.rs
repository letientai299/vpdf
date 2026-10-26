use super::{Destination, Row, RowKind, Style};
use crate::text::{Line, PageId, Position, Text};
use std::ops::Range;
use std::path::{Path, PathBuf};
use vpdf_model::caption::{Caption, Kind};
use vpdf_model::document::{Figure, Heading, Target};
use vpdf_reader::reader::JumpDescription;
use vpdf_search::search::Hit;

impl Row {
  pub fn jump(
    position: Position,
    description: Option<JumpDescription>,
    line: Option<&Line>,
    name: &str,
  ) -> Self {
    let (kind, label) = description.as_ref().map_or_else(
      || {
        let text = line
          .map(Line::text)
          .filter(|text| !text.trim().is_empty())
          .unwrap_or_else(|| "No text".into());
        (RowKind::Mention, text)
      },
      super::jumps::title,
    );
    let number = (position.page.0 + 1).to_string();
    let page = Self::page(if name.is_empty() { &number } else { name });
    let row = Self::new(
      label.split_whitespace().collect::<Vec<_>>().join(" "),
      format!("{page} L{} C{}", position.line + 1, position.glyph + 1),
      Destination::Jump { position, description },
    );
    Self { kind, ..row }
  }

  /// A page as list panels show it: `p4`, or `piv` by its label.
  pub fn page(label: &str) -> String {
    format!("p{label}")
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

  pub fn figure(
    figure: &Figure,
    page: PageId,
    index: usize,
    label: String,
  ) -> Self {
    let caption =
      figure.caption.split_whitespace().collect::<Vec<_>>().join(" ");
    let destination = Destination::Link(Target::Point {
      page,
      x: figure.bounds.x0,
      y: figure.bounds.y0,
    });
    if caption.is_empty() {
      let row = Self::new(format!("Figure {index}"), label, destination);
      return Self { kind: RowKind::Figure, ..row };
    }
    let Some((kind, key, rest)) = caption_key(&caption) else {
      let row = Self::new(caption, label, destination);
      return Self { kind: RowKind::Mention, style: Style::Muted, ..row };
    };
    Self { key, kind, ..Self::new(rest.to_owned(), label, destination) }
  }

  /// Hits under a heading per page.
  pub fn results(hits: &[Hit], label: impl Fn(PageId) -> String) -> Vec<Self> {
    let rows = hits.iter().map(|hit| {
      let Range { start, end } = hit.matched;
      let destination = Destination::Position(hit.first);
      let row = Self::new(hit.excerpt.clone(), String::new(), destination);
      (hit.first.page, Self { style: Style::Excerpt { start, end }, ..row })
    });
    Self::paged(rows, label)
  }

  /// Rows under a heading per page, in page order as given.
  pub fn paged(
    rows: impl IntoIterator<Item = (PageId, Self)>,
    label: impl Fn(PageId) -> String,
  ) -> Vec<Self> {
    let mut paged = Vec::new();
    let mut last = None;
    for (page, row) in rows {
      if last != Some(page) {
        last = Some(page);
        let heading = Self::page(&label(page));
        paged.push(Self::new(heading, String::new(), Destination::None));
      }
      paged.push(Self { depth: 1, ..row });
    }
    paged
  }

  /// A mark keyed `’a`, labeled with its line; `page` is the page's
  /// label.
  pub fn local_mark(
    name: char,
    position: Position,
    text: Option<&Text>,
    page: &str,
  ) -> Self {
    let line = text.and_then(|text| text.lines.get(position.line));
    let line =
      line.map_or_else(String::new, |line| line.text().trim().to_owned());
    let page = Self::page(page);
    let row = Self::new(line, page, Destination::Position(position));
    Self { key: mark_key(name), depth: 1, ..row }
  }

  /// A mark keyed `’A`, labeled with its file; another file's page
  /// labels are unknown, so its page reads by number.
  pub fn global_mark(name: char, position: Position, path: &Path) -> Self {
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    let page = Self::page(&(position.page.0 + 1).to_string());
    let row = Self::new(file.into_owned(), page, Destination::Mark(name));
    Self { key: mark_key(name), depth: 1, style: Style::Document, ..row }
  }

  /// Marks under `Local` and `Global` headings; the header counts
  /// them, and a folded heading its hidden rows.
  pub fn marks(local: Vec<Self>, global: Vec<Self>) -> Vec<Self> {
    let mut rows = Vec::new();
    for (title, marks) in [("Local", local), ("Global", global)] {
      if marks.is_empty() {
        continue;
      }
      rows.push(Self::new(title.into(), String::new(), Destination::None));
      rows.extend(marks);
    }
    rows
  }

  /// The mark a row names, from its `’a` key.
  pub fn mark_name(&self) -> Option<char> {
    let mut key = self.key.strip_prefix(MARK)?.chars();
    key.next().filter(|_| key.next().is_none())
  }

  pub fn recent(path: &Path, page: PageId, home: Option<&Path>) -> Self {
    let label = tilde(path, home);
    Self {
      style: Style::Path,
      ..Self::new(
        label.display().to_string(),
        Self::page(&(page.0 + 1).to_string()),
        Destination::File(path.to_path_buf()),
      )
    }
  }
}

/// `path` with the `home` folder written `~`.
pub(super) fn tilde(path: &Path, home: Option<&Path>) -> PathBuf {
  match home.map(|home| path.strip_prefix(home)) {
    Some(Ok(rest)) => Path::new("~").join(rest),
    _ => path.to_path_buf(),
  }
}

/// Marks read `’a`, as Vim's jump to a mark.
const MARK: char = '’';

fn mark_key(name: char) -> String {
  format!("{MARK}{name}")
}

/// Short and long names of caption labels, as in `Fig 3` and `Figure 3`.
const NAMES: [(Kind, RowKind, &str, &str); 2] = [
  (Kind::Figure, RowKind::Figure, "Fig", "Figure"),
  (Kind::Table, RowKind::Table, "Tab", "Table"),
];

/// Splits `Figure 3: Text` into its kind, `Fig 3` and `Text`.
fn caption_key(caption: &str) -> Option<(RowKind, String, &str)> {
  let Caption { kind, number, rest } = vpdf_model::caption::parse(caption)?;
  let (_, row, short, _) = NAMES.into_iter().find(|name| name.0 == kind)?;
  Some((row, format!("{short} {number}"), rest))
}

impl Row {
  /// The key and label as one text, also with the long label name, so
  /// `/fig 3` and `/figure 3` both find `Fig 3`.
  pub(super) fn keyed_texts(&self) -> impl Iterator<Item = String> + '_ {
    let joined =
      (!self.key.is_empty()).then(|| format!("{} {}", self.key, self.label));
    let long = NAMES.into_iter().find_map(|(_, kind, short, long)| {
      let number =
        self.key.strip_prefix(short).filter(|_| kind == self.kind)?;
      Some(format!("{long}{number} {}", self.label))
    });
    joined.into_iter().chain(long)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::document::FigureSource;
  use vpdf_model::text::Bounds;

  #[test]
  fn figure_rows_split_their_keys() {
    use RowKind::{Figure as F, Mention as M, Table as T};
    // (caption, kind, key, label)
    let cases = [
      (" \n ", F, "", "Figure 3"),
      ("Figure 2.4:   A plot", F, "Fig 2.4", "A plot"),
      (
        "FIGURE 2.3 Execution of a program.",
        F,
        "Fig 2.3",
        "Execution of a program.",
      ),
      ("Figure 12(a)", F, "Fig 12(a)", ""),
      ("Table 1: Maximum path lengths", T, "Tab 1", "Maximum path lengths"),
      ("Table 2. caption", T, "Tab 2", "caption"),
      (
        "Table 2 summarizes our results",
        M,
        "",
        "Table 2 summarizes our results",
      ),
      ("As seen in the plot", M, "", "As seen in the plot"),
    ];
    for (caption, kind, key, label) in cases {
      let figure = Figure {
        bounds: Bounds { x0: 10.0, y0: 20.0, x1: 110.0, y1: 80.0 },
        caption: caption.into(),
        source: FigureSource::Graphic,
      };
      let row = Row::figure(&figure, PageId(7), 3, "viii".into());
      assert_eq!(
        (row.kind, row.key.as_str(), row.label.as_str()),
        (kind, key, label)
      );
      assert_eq!(row.style == Style::Muted, kind == M, "{caption}");
      assert_eq!(row.detail, "viii");
      assert!(matches!(
        row.destination,
        Destination::Link(Target::Point { page: PageId(7), x: 10.0, y: 20.0 })
      ));
    }
  }
}
