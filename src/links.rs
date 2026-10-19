use crate::Result;
use crate::app::App;
use crate::app::InputOwner;
use crate::document::{Document, Heading, Link, Target};
use crate::panel::layout::Layout;
use crate::panel::{Destination, Row};
use crate::pointer::{Hit, Hover};
use crate::preview::{Origin, Preview};
use crate::split::Rect;
use crate::text::{Bounds, PageId, Position, Text};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The hovered link's page, that page's links, and index.
type Hovered = (PageId, Arc<Vec<Link>>, usize);

/// Rest before a link previews.
pub const REST: Duration = Duration::from_millis(400);

/// A left press's link or note icon.
#[derive(Clone, Debug, Default)]
pub enum LinkPress {
  #[default]
  Idle,
  /// Followed on release.
  Held(Target),
  /// Note icon, press glyph; shows on release (U11).
  Note(PageId, usize, Option<Position>),
  /// Its click run is spent.
  Followed,
}

/// The link under the pointer, resting.
#[derive(Clone, Copy, Debug)]
pub struct Rest {
  pub page: PageId,
  /// Index into `Document::links`.
  pub index: usize,
  /// When the rest began; None once calmed.
  pub since: Option<Instant>,
}

impl App {
  /// Takes `hover`; new links beneath rest.
  pub fn point(&mut self, hover: Option<Hover>) {
    self.hover = hover.map_or_default(|hover| hover.hit);
    let beneath = hover.and_then(|hover| hover.beneath);
    if self.rest.map(|rest| (rest.page, rest.index)) == beneath {
      return;
    }
    self.rest = beneath.map(|(page, index)| Rest {
      page,
      index,
      since: Some(Instant::now()),
    });
  }

  fn hovered_links(&mut self) -> Option<Hovered> {
    let Hit::Link(page, index) = self.hover else { return None };
    // Hover is advisory; failures show nothing.
    let links = self.doc.links(page).ok()?;
    Some((page, links, index))
  }

  /// The hovered link and its page.
  pub fn hover_link(&mut self) -> Option<(PageId, Link)> {
    let (page, links, index) = self.hovered_links()?;
    Some((page, links.get(index)?.clone()))
  }

  /// The hovered link's boxes, across pages.
  pub fn hover_segments(&mut self) -> Vec<(PageId, Bounds)> {
    let Some((page, links, index)) = self.hovered_links() else {
      return Vec::new();
    };
    let Some(link) = links.get(index) else { return Vec::new() };
    let mut boxes: Vec<_> = link.boxes().map(|b| (page, b)).collect();
    boxes.extend(crate::wraps::page_parts(&mut self.doc, page, link));
    boxes
  }

  /// The hovered link's destination; citations show the reference.
  pub fn hover_label(&mut self) -> Option<String> {
    let (page, link) = self.hover_link()?;
    Some(self.link_label(page, &link))
  }

  /// A link's destination; citations show the reference.
  pub(crate) fn link_label(&mut self, page: PageId, link: &Link) -> String {
    if matches!(link.target, Target::Uri(_)) {
      return destination(&link.target, &self.doc, "");
    }
    // Hover is advisory; unreadable text reads as a bare reference.
    let text = self.doc.text(page).ok();
    let (anchor, context) = self.link_context(page, link, text.as_deref());
    let cited = cited(&anchor, context);
    destination(&link.target, &self.doc, &cited)
  }

  /// When the rested link previews.
  pub fn hover_due(&mut self) -> Option<Instant> {
    let rest = self.rest?;
    let since = rest.since?;
    // Hover follows the pointer across panes (B2).
    let free = self.input_owner() == InputOwner::Content;
    if !free || self.peek.is_some() {
      return None;
    }
    let link = self.link(rest.page, rest.index).ok()??;
    let internal = !matches!(link.target, Target::Uri(_));
    internal.then_some(since + REST)
  }

  /// Opens a due preview.
  pub fn hover_preview(&mut self, now: Instant) -> bool {
    if self.hover_due().is_none_or(|due| now < due) {
      return false;
    }
    self.calm_hover();
    let Some(Rest { page, index, .. }) = self.rest else { return false };
    let Ok(Some(link)) = self.link(page, index) else { return false };
    // Previews are advisory; failures show nothing.
    let Ok(text) = self.doc.text(page) else { return false };
    // Textless links have no anchor.
    let boxes: Vec<_> = link.boxes().collect();
    let Some(source) = text.in_areas(page, &boxes).0 else {
      return false;
    };
    if self.show_preview(link.target, source).is_err() {
      return false;
    }
    if let Some(peek) = &mut self.peek {
      peek.origin = Origin::Hover(page, index);
    }
    true
  }

  /// The open hover preview.
  pub(crate) fn hover_peek(&self) -> Option<&Preview> {
    self.peek.as_ref().filter(|peek| peek.hovered().is_some())
  }

  /// Motion on the link restarts its rest.
  pub(crate) fn rest_again(&mut self) {
    if let Some(since) = self.rest.as_mut().and_then(|r| r.since.as_mut()) {
      *since = Instant::now();
    }
  }

  /// Ends the rest until another link.
  pub(crate) fn calm_hover(&mut self) {
    if let Some(rest) = &mut self.rest {
      rest.since = None;
    }
  }

  /// Closes a hover preview the pointer left.
  pub fn hover_leave(&mut self, at: Option<(u16, u16)>) -> bool {
    let Some(peek) = self.hover_peek() else { return false };
    // A moved view leaves it stale.
    let view = self.view_key();
    let placement = peek.placement.filter(|at| at.view == view);
    let on = |cells: Rect| at.is_some_and(|(c, r)| cells.contains(c, r));
    let inside = placement.is_some_and(|at| on(at.area));
    let bridged = placement.is_some_and(|at| on(at.bridge));
    let rested = self.rest.map(|rest| (rest.page, rest.index));
    let on_link = rested.is_some_and(|link| peek.hovered() == Some(link));
    // Another link is never a gap.
    let elsewhere = rested.is_some() && !on_link;
    if inside || on_link || bridged && !elsewhere {
      return false;
    }
    self.peek = None;
    true
  }

  /// Closes a hover preview; whether it did.
  pub(crate) fn drop_hover_preview(&mut self) -> bool {
    let open = self.hover_peek().is_some();
    if open {
      self.peek = None;
    }
    self.calm_hover();
    open
  }

  /// Inside: left follows, middle splits.
  pub fn preview_press(&mut self, mouse: MouseEvent) -> Result<bool> {
    let Some(peek) = &self.peek else { return Ok(false) };
    let key = peek.origin == Origin::Key;
    if key && self.input_owner() != InputOwner::Peek {
      return Ok(false);
    }
    let inside = self.in_preview(mouse);
    let alt = mouse.modifiers.contains(KeyModifiers::ALT);
    let button = match mouse.kind {
      MouseEventKind::Down(button) if inside && !alt => Some(button),
      _ => None,
    };
    let follows = self.peek.as_ref().is_some_and(Preview::follows);
    match button {
      Some(MouseButton::Left) => {
        // Like `dismiss_click`, it cancels pending keys.
        self.disarm();
        self.follow_preview()?;
        return Ok(true);
      }
      // Its target in a split (C2).
      Some(MouseButton::Middle) if follows => {
        self.disarm();
        let target = self.peek.take().map(|peek| peek.target);
        if let Some(target) = target {
          self.open_split(&target)?;
        }
        return Ok(true);
      }
      _ => {}
    }
    if key {
      return Ok(false);
    }
    self.drop_hover_preview();
    if inside {
      self.disarm();
    }
    Ok(inside)
  }

  /// Whether a press lands on the drawn preview.
  pub(crate) fn in_preview(&self, mouse: MouseEvent) -> bool {
    let (column, row) = self.pane_cell(mouse);
    self.preview_cells().is_some_and(|at| at.contains(column, row))
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
      let (anchor, context) = self.link_context(page, &link, text.as_deref());
      rows.push(link_row(link, anchor, context, |page| {
        page_name(&self.doc, page)
      }));
    }
    Ok(rows)
  }

  /// A link's text on `page`, and for references, the text they cite.
  fn link_context(
    &mut self,
    page: PageId,
    link: &Link,
    text: Option<&Text>,
  ) -> (String, String) {
    let anchor = text.map_or_else(String::new, |text| link.anchor(text, page));
    let context = match link.target {
      Target::Page(page) | Target::Point { page, .. } => self
        .doc
        .text(page)
        .ok()
        .map_or_else(String::new, |text| excerpt(&text, &link.target)),
      Target::Uri(_) => String::new(),
    };
    if !cites(&anchor, &context) {
      return (anchor, String::new());
    }
    (anchor, context)
  }
}

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

pub(crate) fn excerpt(text: &Text, target: &Target) -> String {
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
fn link_row(
  link: Link,
  anchor: String,
  context: String,
  name: impl Fn(PageId) -> String,
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
      let label = name(*page);
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
fn cited(anchor: &str, context: String) -> String {
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
pub(crate) fn destination(
  target: &Target,
  doc: &Document,
  cited: &str,
) -> String {
  let outline = &doc.outline;
  let (page, y) = match target {
    Target::Uri(uri) => return format!("→ {uri}"),
    Target::Page(page) => (*page, first_heading(outline, *page)),
    Target::Point { page, y, .. } => (*page, *y),
  };
  let page = PageId(page.0.min(doc.pages.len().saturating_sub(1)));
  let name = doc.label(page);
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

/// A page's label; past the last page, its number.
fn page_name(doc: &Document, page: PageId) -> String {
  doc
    .get_label(page)
    .map_or_else(|| page.0.saturating_add(1).to_string(), str::to_owned)
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
  use crate::keys::{Action, Input, PanelKind};
  use crate::layout::Screen;
  use crate::panel::{Panel, Style};
  use crate::text::{Bounds, Glyph, Line as TextLine, PageId};
  use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use std::path::PathBuf;
  use unicode_width::UnicodeWidthStr;
  use vpdf_reader::motion::Motion;
  use vpdf_terminal::terminal;

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
    let pages = |_| "iv".to_owned();
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
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 20);
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
    panel.navigate(Input::plain(Action::Motion(Motion::Down)), 6);
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
    app.go_to_page(PageId(1))?;
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
    app.go_to_page(PageId(3))?;
    let rows = app.panel_rows(PanelKind::Links)?;
    assert!(rows[3].label.starts_with("4 To illustrate"));
    assert_eq!(rows[3].detail, "p. 4");

    app.go_to_page(PageId(10))?;
    app.open_panel(PanelKind::Links)?;
    let row = &app.panel.as_ref().unwrap().rows[0];
    assert_eq!(row.label, "arXiv:1610.02357");
    // U21: hovers quote what references cite, as the panel does.
    let text = app.doc.text(PageId(2))?;
    let first = app.doc.links(PageId(2))?.iter().position(|link| {
      let cites = matches!(link.target, Target::Point { page: PageId(9), .. });
      cites && link.anchor(&text, PageId(2)) == "1"
    });
    let cases = [
      (1, 0, "→ p. 11 · [13] Sepp Hochreiter and Jürgen Schmidhuber."),
      // The first entry sits right under the bibliography heading.
      (2, first.expect("[1] on p. 3"), "→ p. 10 · [1] Jimmy Lei Ba"),
      (3, 3, "→ p. 4 · 4 To illustrate"),
      (10, 0, "→ http://arxiv.org/abs/1610.02357"),
    ];
    for (page, index, label) in cases {
      app.hover = Hit::Link(PageId(page), index);
      let shown = app.hover_label().unwrap_or_default();
      assert!(shown.starts_with(label), "{shown:?}");
    }
    app.panel_input(Input { register: 'a', ..Input::plain(Action::Yank) })?;
    assert_eq!(
      app.reader.registers.get(&'a').map(|text| &**text),
      Some("http://arxiv.org/abs/1610.02357\n")
    );
    Ok(())
  }
}
