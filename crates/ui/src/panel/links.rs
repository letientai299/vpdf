use super::layout::Layout;
use super::{Destination, Row, Target};
#[cfg(test)]
use crate::presentation::*;
use crate::text::{PageId, Position, Text};
use vpdf_model::document::{Heading, Link};
use vpdf_reader::reader::{JumpDescription, TextSource};

/// Whether `anchor` cites what `context` starts: a number, a bracket,
/// or the author the entry starts with.
fn cites(anchor: &str, context: &str) -> bool {
  let numeric = anchor.chars().filter(|ch| ch.is_alphabetic()).take(3).count();
  numeric < 3
    || anchor.starts_with('[') && anchor.ends_with(']')
    || leads(anchor, context)
}

/// Whether both start with the same word.
fn leads(anchor: &str, context: &str) -> bool {
  let word = |text: &str| -> String {
    text.chars().take_while(|ch| ch.is_alphabetic()).collect()
  };
  let first = word(anchor);
  !first.is_empty() && first == word(context)
}

fn excerpt(text: &Text, target: &Target) -> String {
  let start = match target {
    Target::Point { page, x, y } => {
      below(text, *x, *y).unwrap_or_else(|| text.nearest(*page, *x, *y).line)
    }
    _ => 0,
  };
  let context: String = text
    .lines
    .iter()
    .skip(start)
    .take(3)
    .enumerate()
    .take_while(|(index, line)| *index == 0 || !line.paragraph)
    .flat_map(|(_, line)| line.glyphs.iter().map(|glyph| glyph.ch).chain([' ']))
    .take(240)
    .collect();
  context.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The line nearest a point, among those not mostly above it:
/// reference targets sit in the gap above their entry, often nearer
/// the heading over the first one.
fn below(text: &Text, x: f32, y: f32) -> Option<usize> {
  let lines = text.lines.iter().enumerate();
  let scored = lines.filter_map(|(index, line)| {
    // Anchors can sit inside the line above, not past its middle.
    let span = line.span().filter(|span| span.y0 + span.y1 >= 2.0 * y)?;
    let dx = (span.x0 - x).max(0.0).max(x - span.x1);
    let dy = (span.y0 - y).max(0.0);
    Some((index, dx * dx + 4.0 * dy * dy))
  });
  scored.min_by(|a, b| a.1.total_cmp(&b.1)).map(|(index, _)| index)
}

/// `name` gives a page's label.
pub fn link_row<'a>(
  link: Link,
  anchor: String,
  context: String,
  name: impl Fn(PageId) -> Option<&'a str>,
) -> Row {
  let (label, detail) = match &link.target {
    Target::Uri(uri) => {
      let raw = anchor.is_empty()
        || ["http://", "https://", "mailto:", "//"]
          .iter()
          .any(|prefix| anchor.starts_with(prefix));
      (if raw { uri.clone() } else { anchor }, String::new())
    }
    Target::Page(page) | Target::Point { page, .. } => {
      let label = name(*page)
        .map_or_else(|| page.0.saturating_add(1).to_string(), str::to_owned);
      let cited = cited(&anchor, context);
      let text = if cited.is_empty() { anchor } else { cited };
      if text.is_empty() {
        (format!("Page {label}"), String::new())
      } else {
        (text, format!("p. {label}"))
      }
    }
  };
  Row::new(label, detail, Destination::Link(link.target))
}

/// A reference's anchor joined with the `context` it cites; empty
/// without context.
pub fn cited(anchor: &str, context: String) -> String {
  if context.is_empty() {
    return context;
  }
  if anchor.is_empty()
    || context.starts_with(&format!("[{anchor}]"))
    || leads(anchor, &context)
  {
    context
  } else if let Some(rest) = context
    .strip_prefix(anchor)
    .filter(|rest| !rest.starts_with(char::is_numeric))
  {
    format!("{anchor} {}", rest.trim_start()).trim_end().to_owned()
  } else {
    format!("{anchor} {context}").trim().to_owned()
  }
}

/// `→ p. 43 · Section`, `cited` in place of the section, or the
/// address.
pub fn destination<'a>(
  target: &Target,
  outline: &[Heading],
  pages: usize,
  name: impl Fn(PageId) -> &'a str,
  cited: &str,
) -> String {
  let (page, y) = match target {
    Target::Uri(uri) => return format!("→ {uri}"),
    Target::Page(page) => (*page, first_heading(outline, *page)),
    Target::Point { page, y, .. } => (*page, *y),
  };
  let page = PageId(page.0.min(pages.saturating_sub(1)));
  let name = name(page);
  if !cited.is_empty() {
    return format!("→ p. {name} · {cited}");
  }
  match vpdf_model::document::section(outline, page, y) {
    Some(entry) => format!("→ p. {name} · {}", outline[entry].title.trim()),
    None => format!("→ p. {name}"),
  }
}

/// First heading top on `page`, else 0.
fn first_heading(outline: &[Heading], page: PageId) -> f32 {
  let tops = outline.iter().filter_map(|heading| match heading.target {
    Target::Point { page: on, y, .. } if on == page => Some(y),
    _ => None,
  });
  tops.reduce(f32::min).unwrap_or(0.0)
}

/// A link's text on `page`, and for references, the text they cite.
pub fn context(
  page: PageId,
  link: &Link,
  text: Option<&Text>,
  source: &mut impl TextSource,
) -> (String, String) {
  let anchor = text.map_or_else(String::new, |text| link.anchor(text, page));
  let context = match link.target {
    Target::Page(page) | Target::Point { page, .. } => source
      .page_text(page)
      .ok()
      .map_or_else(String::new, |text| excerpt(&text, &link.target)),
    Target::Uri(_) => String::new(),
  };
  if !cites(&anchor, &context) {
    return (anchor, String::new());
  }
  (anchor, context)
}

pub fn description(
  target: &Target,
  position: Position,
  text: Option<&Text>,
  outline: &[Heading],
) -> JumpDescription {
  let Some(text) = text else {
    return JumpDescription::Link(String::new());
  };
  for heading in outline {
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
  JumpDescription::Link(excerpt(text, target))
}

impl Layout {
  pub fn links(rows: &[Row], columns: usize, selected: usize) -> Self {
    let mut layout = Self { columns, lines: Vec::new(), starts: Vec::new() };
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      layout.label(source, &row.label, &row.detail, selected);
      if let Destination::Link(Target::Uri(uri)) = &row.destination
        && *uri != row.label
      {
        layout.secondary(source, uri, selected);
      }
    }
    layout.starts.push(layout.lines.len());
    layout
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Bounds, Glyph, Line as TextLine};
  use crate::{
    Result,
    keys::{Action, Input, PanelKind},
    panel::{Panel, Style},
  };
  use unicode_width::UnicodeWidthStr;
  use vpdf_model::input::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use vpdf_reader::motion::Motion;
  #[test]
  fn failed_context_keeps_anchor() {
    struct Missing {
      reads: usize,
    }
    impl TextSource for Missing {
      fn page_count(&self) -> usize {
        1
      }
      fn page_text(&mut self, _: PageId) -> Result<std::sync::Arc<Text>> {
        self.reads += 1;
        anyhow::bail!("Text unavailable")
      }
    }
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
    let text = Text {
      lines: vec![TextLine {
        paragraph: false,
        glyphs: vec![Glyph { ch: '1', bounds }],
      }],
      truncated: false,
    };
    let mut source = Missing { reads: 0 };
    let page = PageId(0);
    let mut link = Link::new(bounds, Target::Page(page));
    assert_eq!(
      context(page, &link, Some(&text), &mut source),
      ("1".into(), String::new())
    );
    assert_eq!(source.reads, 1);
    link.target = Target::Uri("https://example.org".into());
    assert_eq!(
      context(page, &link, Some(&text), &mut source),
      ("1".into(), String::new())
    );
    assert_eq!(source.reads, 1);
  }

  #[test]
  fn labels_distinguish_targets() {
    let pages = |_| Some("iv");
    for (anchor, context, expected) in [
      ("[12]", "Attention Is All You Need", "[12] Attention Is All You Need"),
      ("†", "Equal contribution", "† Equal contribution"),
      ("4", "4To illustrate the result", "4 To illustrate the result"),
      ("2", "2 Code is available online", "2 Code is available online"),
      ("12", "[12] Reference title", "[12] Reference title"),
      ("", "Introduction", "Introduction"),
      ("", "", "Page iv"),
    ] {
      let target = Target::Point { page: PageId(0), x: 12.0, y: 34.0 };
      let link = Link::new(Bounds::default(), target);
      let row = link_row(link, anchor.into(), context.into(), pages);
      assert_eq!(row.label, expected);
      assert!(matches!(
        row.destination,
        Destination::Link(Target::Point { x: 12.0, y: 34.0, .. })
      ));
    }
    let uri = "https://example.com/paper";
    for (anchor, expected) in
      [("Read more", "Read more"), ("", uri), (uri, uri)]
    {
      let link = Link::new(Bounds::default(), Target::Uri(uri.into()));
      let row = link_row(link, anchor.into(), String::new(), pages);
      assert_eq!(row.label, expected);
      assert_eq!(row.detail, "");
      assert!(matches!(row.destination,
        Destination::Link(Target::Uri(target)) if target == uri));
    }
    for (anchor, expected) in [
      ("[12]", true),
      ("*", true),
      ("a", true),
      ("[Smith, 2020]", true),
      ("Introduction", false),
    ] {
      assert_eq!(cites(anchor, "Unrelated text"), expected);
    }
  }

  #[test]
  fn excerpts_follow_destinations() {
    let text = Text {
      lines: [
        "Header",
        "† Equal contribution",
        "Authors listed together",
        "Unrelated paragraph",
      ]
      .into_iter()
      .enumerate()
      .map(|(index, value)| TextLine {
        paragraph: index != 2,
        glyphs: value
          .chars()
          .enumerate()
          .map(|(glyph, ch)| Glyph {
            ch,
            bounds: Bounds {
              x0: glyph as f32 * 10.0,
              y0: index as f32 * 20.0,
              x1: (glyph + 1) as f32 * 10.0,
              y1: index as f32 * 20.0 + 10.0,
            },
          })
          .collect(),
      })
      .collect(),
      truncated: false,
    };
    let target = Target::Point { page: PageId(0), x: 0.0, y: 20.0 };
    assert_eq!(
      excerpt(&text, &target),
      "† Equal contribution Authors listed together"
    );
    assert_eq!(excerpt(&Text::default(), &target), "");
    // An anchor inside the line above starts below it.
    let inside = Target::Point { page: PageId(0), x: 0.0, y: 8.0 };
    assert_eq!(
      excerpt(&text, &inside),
      "† Equal contribution Authors listed together"
    );
  }

  /// U21: numbers, brackets, and leading authors quote their entry.
  #[test]
  fn citations_quote_their_entry() {
    let vaswani = "Vaswani, A., Shazeer, N. (2017). Attention is all you need.";
    // (anchor, excerpt) -> label text, or None for the section
    let cases = [
      ("13", "[13] Sepp Hochreiter.", Some("[13] Sepp Hochreiter.")),
      ("[2]", "Jimmy Ba.", Some("[2] Jimmy Ba.")),
      ("Vaswani et al., 2017", vaswani, Some(vaswani)),
      ("Section 3", "3 Model Architecture", None),
      ("See the results", "Results heading", None),
      (
        "Ba",
        "Ba, J. Layer normalization.",
        Some("Ba, J. Layer normalization."),
      ),
    ];
    for (anchor, context, label) in cases {
      let quoted =
        cites(anchor, context).then(|| cited(anchor, context.to_owned()));
      assert_eq!(quoted.as_deref(), label, "{anchor}");
    }
  }

  fn row(label: &str, uri: &str) -> Row {
    Row::new(
      label.into(),
      String::new(),
      Destination::Link(Target::Uri(uri.into())),
    )
  }
  #[test]
  fn unselected_long_labels_clip() {
    let long = "A label far wider than the panel";
    let rows = [row(long, long), row(long, long), row("Short", "Short")];
    let layout = Layout::links(&rows, 20, 0);
    // (source) -> its label line clipped
    let cases = [(0, false), (1, true), (2, false)];
    for (source, want) in cases {
      let label = &layout.lines[layout.starts[source]];
      assert_eq!(label.clipped, want, "row {source}");
    }
    assert!(layout.lines.iter().filter(|line| line.source == 0).count() > 1);
  }
  #[test]
  fn selected_urls_expand() {
    let uri = "https://example.com/long/path?query=needle";
    let rows = [row("Project page", uri), row(uri, uri)];
    for columns in [4, 12, 30, 50] {
      let layout = Layout::links(&rows, columns, 0);
      let secondary: String = layout
        .lines
        .iter()
        .filter(|line| line.source == 0 && line.row.style == Style::Muted)
        .map(|line| line.row.label.trim_start())
        .collect();
      assert_eq!(secondary, uri);
      assert!(layout.lines.iter().all(|line| line.row.label.width()
        + line.row.detail.width()
        <= columns));
      assert_eq!(
        layout.lines.iter().filter(|line| line.source == 1).count(),
        1
      );
      let layout = Layout::links(&rows, columns, 1);
      let raw: String = layout
        .lines
        .iter()
        .filter(|line| line.source == 1)
        .map(|line| line.row.label.as_str())
        .collect();
      assert_eq!(raw, uri);
    }
  }
  #[test]
  fn expanded_links_keep_targets() -> Result<()> {
    let uri = "https://example.com/long/path?query=needle";
    let mut panel = Panel::new(
      PanelKind::Links,
      vec![row("Project page", uri), row("Other", "https://example.org")],
    );
    panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 24, 20)?;
    let press = |line| MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 4,
      row: line,
      modifiers: KeyModifiers::NONE,
    };
    assert_eq!(panel.pointer(press(2)), Some(0));
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 20);
    panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 24, 20)?;
    assert_eq!(panel.selected, 1);
    panel.begin_search(false);
    panel.preview("needle");
    assert_eq!(panel.selected, 0);
    assert_eq!(panel.count(), Some((1, 1)));
    panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 12, 20)?;
    assert_eq!(panel.pointer(press(3)), Some(0));
    Ok(())
  }
  #[test]
  fn wheel_scrolls_expanded_links() -> Result<()> {
    let uri = format!("https://example.com/{}", "long-path/".repeat(20));
    let mut panel = Panel::new(
      PanelKind::Links,
      vec![row(&uri, &uri), row("Other", "https://example.org")],
    );
    panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 20, 6)?;
    panel.pointer(MouseEvent {
      kind: MouseEventKind::ScrollDown,
      column: 4,
      row: 2,
      modifiers: KeyModifiers::NONE,
    });
    panel.list(&mut Vec::new(), vpdf_model::geometry::At::default(), 20, 6)?;
    assert_eq!((panel.selected, panel.top), (0, 3));
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 6);
    let mut out = Vec::new();
    panel.list(&mut out, vpdf_model::geometry::At::default(), 20, 6)?;
    assert_eq!(panel.selected, 1);
    assert!(String::from_utf8(out)?.contains("Other"));
    Ok(())
  }
}
