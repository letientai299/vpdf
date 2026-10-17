use crate::Result;
use crate::app::App;
use crate::document::{Heading, Link, PageInfo, Target};
use crate::panel::{Destination, Row, Style};
use crate::panel_layout::{Layout, Line, wrap};
use crate::terminal;
use crate::text::{Bounds, PageId, Text};
use unicode_width::UnicodeWidthStr;

/// The link under the pointer.
#[derive(Clone, Debug)]
pub struct HoverLink {
  pub page: PageId,
  /// Index into `Document::links`.
  pub index: usize,
  pub bounds: Bounds,
  /// Destination for the status line.
  pub label: String,
}

/// A left press's link.
#[derive(Clone, Debug, Default)]
pub enum LinkPress {
  #[default]
  Idle,
  /// Followed on release.
  Held(Target),
  /// Its click run is spent.
  Followed,
}

impl App {
  /// Shows a page's link by index.
  pub fn point_link(&mut self, link: Option<(PageId, usize)>) {
    let shown = self.hover_link.as_ref().map(|link| (link.page, link.index));
    if shown == link {
      return;
    }
    // Hover is advisory; failures show nothing.
    self.hover_link = link.and_then(|(page, index)| {
      let links = self.doc.links(page).ok()?;
      let link = links.get(index)?;
      let label = destination(&link.target, &self.doc.pages, &self.doc.outline);
      Some(HoverLink { page, index, bounds: link.bounds, label })
    });
  }

  pub(crate) fn link_rows(&mut self) -> Result<Vec<Row>> {
    let page = self.navigation_position().page;
    let links = self.doc.links(page)?;
    if links.is_empty() {
      return Ok(Vec::new());
    }
    // Unreadable text must not hide destinations.
    let text = self.doc.text(page).ok();
    let mut rows = Vec::with_capacity(links.len());
    for link in links.iter().cloned() {
      let anchor = text
        .as_ref()
        .map_or_else(String::new, |text| text.in_areas(page, &[link.bounds]).1);
      let context = match link.target {
        Target::Page(page) | Target::Point { page, .. }
          if reference(&anchor) =>
        {
          self
            .doc
            .text(page)
            .ok()
            .map_or_else(String::new, |text| excerpt(&text, &link.target))
        }
        _ => String::new(),
      };
      rows.push(link_row(link, anchor, context, &self.doc.pages));
    }
    Ok(rows)
  }
}

fn reference(anchor: &str) -> bool {
  anchor.chars().filter(|ch| ch.is_alphabetic()).take(3).count() < 3
    || anchor.starts_with('[') && anchor.ends_with(']')
}

fn excerpt(text: &Text, target: &Target) -> String {
  let start = match target {
    Target::Point { page, x, y } => text.nearest(*page, *x, *y).line,
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

fn link_row(
  link: Link,
  anchor: String,
  context: String,
  pages: &[PageInfo],
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
      let label = page_name(pages, *page);
      let text = if context.is_empty() {
        anchor
      } else if anchor.is_empty() || context.starts_with(&format!("[{anchor}]"))
      {
        context
      } else if let Some(rest) = context
        .strip_prefix(&anchor)
        .filter(|rest| !rest.starts_with(char::is_numeric))
      {
        format!("{anchor} {}", rest.trim_start()).trim_end().to_owned()
      } else {
        format!("{anchor} {context}").trim().to_owned()
      };
      if text.is_empty() {
        (format!("Page {label}"), String::new())
      } else {
        (text, format!("p. {label}"))
      }
    }
  };
  Row::new(label, detail, Destination::Link(link.target))
}

/// `→ p. 43 · Section`, or the address.
fn destination(
  target: &Target,
  pages: &[PageInfo],
  outline: &[Heading],
) -> String {
  let (page, y) = match target {
    Target::Uri(uri) => return format!("→ {uri}"),
    Target::Page(page) => (*page, first_heading(outline, *page)),
    Target::Point { page, y, .. } => (*page, *y),
  };
  let page = PageId(page.0.min(pages.len().saturating_sub(1)));
  let name = page_name(pages, page);
  match crate::panel::section(outline, page, y) {
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

/// A page's label, else its number.
fn page_name(pages: &[PageInfo], page: PageId) -> String {
  pages
    .get(page.0)
    .map(|info| info.label.as_str())
    .filter(|label| !label.is_empty())
    .map_or_else(|| page.0.saturating_add(1).to_string(), str::to_owned)
}

impl Layout {
  pub fn links(rows: &[Row], columns: usize, selected: usize) -> Self {
    let mut layout = Self { columns, lines: Vec::new(), starts: Vec::new() };
    for (source, row) in rows.iter().enumerate() {
      layout.starts.push(layout.lines.len());
      let detail = terminal::clean(&row.detail, columns / 3);
      let room = columns
        .saturating_sub(detail.width() + usize::from(!detail.is_empty()));
      let (labels, cut) = if source == selected {
        (wrap(&row.label, room), false)
      } else {
        let (label, cut) = terminal::clipped(&row.label, room);
        (vec![label], cut)
      };
      for (index, label) in labels.into_iter().enumerate() {
        layout.lines.push(Line {
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
      if let Destination::Link(Target::Uri(uri)) = &row.destination
        && *uri != row.label
      {
        let indent = columns.saturating_sub(1).min(2);
        let room = columns.saturating_sub(indent);
        let lines = if source == selected {
          wrap(uri, room)
        } else {
          vec![terminal::clipped(uri, room).0]
        };
        for line in lines {
          layout.lines.push(Line {
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
    layout.starts.push(layout.lines.len());
    layout
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::keys::{Action, Input, PanelKind};
  use crate::layout::Screen;
  use crate::motion::Motion;
  use crate::panel::Panel;
  use crate::text::{Bounds, Glyph, Line as TextLine, PageId, Position};
  use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use std::path::PathBuf;

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
  fn labels_distinguish_targets() {
    let pages = [PageInfo { bounds: Bounds::default(), label: "iv".into() }];
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
      let link = Link { bounds: Bounds::default(), target };
      let row = link_row(link, anchor.into(), context.into(), &pages);
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
      let link =
        Link { bounds: Bounds::default(), target: Target::Uri(uri.into()) };
      let row = link_row(link, anchor.into(), String::new(), &pages);
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
      assert_eq!(reference(anchor), expected);
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
    panel.list(&mut Vec::new(), terminal::At::default(), 24, 20)?;
    let press = |line| MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column: 4,
      row: line,
      modifiers: KeyModifiers::NONE,
    };
    assert_eq!(panel.pointer(press(2)), Some(0));
    panel.navigate(
      Input {
        action: Action::Motion(Motion::Down),
        count: 1,
        explicit: false,
        register: '"',
      },
      20,
    );
    panel.list(&mut Vec::new(), terminal::At::default(), 24, 20)?;
    assert_eq!(panel.selected, 1);
    panel.begin_search(false);
    panel.preview("needle");
    assert_eq!(panel.selected, 0);
    assert_eq!(panel.count(), Some((1, 1)));
    panel.list(&mut Vec::new(), terminal::At::default(), 12, 20)?;
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
    panel.list(&mut Vec::new(), terminal::At::default(), 20, 6)?;
    panel.pointer(MouseEvent {
      kind: MouseEventKind::ScrollDown,
      column: 4,
      row: 2,
      modifiers: KeyModifiers::NONE,
    });
    panel.list(&mut Vec::new(), terminal::At::default(), 20, 6)?;
    assert_eq!((panel.selected, panel.top), (0, 3));
    panel.navigate(
      Input {
        action: Action::Motion(Motion::Down),
        count: 1,
        explicit: false,
        register: '"',
      },
      6,
    );
    let mut out = Vec::new();
    panel.list(&mut out, terminal::At::default(), 20, 6)?;
    assert_eq!(panel.selected, 1);
    assert!(String::from_utf8(out)?.contains("Other"));
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn real_links_resolve_text() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 50,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app = App::with_screen(&root.join("paper.pdf"), "", true, screen)?;
    app.go_to(Position { page: PageId(1), ..Position::default() }, false)?;
    let rows = app.panel_rows(PanelKind::Links)?;
    assert!(rows[0].label.starts_with("[13] Sepp Hochreiter"));
    assert!(rows[0].label.contains("Long short-term memory"));
    assert_eq!(rows[0].detail, "p. 11");
    for row in rows.iter().filter(|row| !row.detail.is_empty()).take(3) {
      let Destination::Link(target) = &row.destination else {
        panic!("Reference destination");
      };
      app.follow(target)?;
    }
    app.go_to(Position { page: PageId(3), ..Position::default() }, false)?;
    let rows = app.panel_rows(PanelKind::Links)?;
    assert!(rows[3].label.starts_with("4 To illustrate"));
    assert_eq!(rows[3].detail, "p. 4");

    app.go_to(Position { page: PageId(10), ..Position::default() }, false)?;
    app.open_panel(PanelKind::Links)?;
    let row = &app.panel.as_ref().unwrap().rows[0];
    assert_eq!(row.label, "arXiv:1610.02357");
    app.panel_input(Input {
      action: Action::Yank,
      count: 1,
      explicit: false,
      register: 'a',
    })?;
    assert_eq!(
      app.reader.registers.get(&'a').map(|text| &**text),
      Some("http://arxiv.org/abs/1610.02357\n")
    );
    Ok(())
  }
}
