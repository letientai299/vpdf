use crate::Result;
use crate::app::{App, InputOwner, Tabline};
use crate::chrome::Chrome;
use crate::find::Phase;
use crate::grid;
use crate::layout::Screen;
use crate::pointer::Hit;
use crate::prompt::Prompt;
use crate::reader::{Mode, SelectionKind};
use crate::terminal::{self, At};
use crate::text::PageId;
use crate::theme::{self, Color};
use std::fmt::Write as _;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

impl App {
  fn mode(&self) -> (&'static str, Color) {
    match self.input_owner() {
      InputOwner::Prompt => return ("PROMPT", Color::Yellow),
      InputOwner::Hints => return ("HINT", Color::Yellow),
      InputOwner::Context => return ("MENU", Color::Yellow),
      InputOwner::Peek => return ("PEEK", Color::Yellow),
      InputOwner::Region => return ("REGION", Color::Yellow),
      InputOwner::Panel => {
        if let Some(panel) = &self.panel {
          return (panel.kind.title(), Color::Cyan);
        }
      }
      InputOwner::Overview => return ("PAGES", Color::Cyan),
      InputOwner::Content => {}
    }
    match self.reader.mode() {
      Mode::Visual(SelectionKind::Character) => ("VISUAL", Color::Magenta),
      Mode::Visual(SelectionKind::Line) => ("V-LINE", Color::Magenta),
      _ if self.reader.operator.is_some() => ("YANK", Color::Yellow),
      Mode::Cursor => ("CURSOR", Color::Cyan),
      Mode::Normal => ("NORMAL", Color::Blue),
    }
  }

  /// Match under or before the cursor, like Neovim's searchcount.
  fn search_count(&self) -> String {
    if let Some(panel) = self.panel.as_ref().filter(|panel| panel.focused) {
      return panel
        .count()
        .map_or_else(String::new, |(i, n)| format!("[{i}/{n}]  "));
    }
    let hits = &self.search.hits;
    if self.search.hidden || hits.is_empty() {
      return String::new();
    }
    let more =
      if matches!(self.search.phase, Phase::Running { .. }) { "+" } else { "" };
    if self.reader.viewport_mode() {
      return format!("[{}{more} matches]  ", hits.len());
    }
    let cursor = self.reader.cursor;
    let index = hits.partition_point(|hit| hit.first <= cursor);
    format!("[{index}/{}{more}]  ", hits.len())
  }

  /// Share of the document above the viewport's bottom edge.
  fn progress(&self) -> u32 {
    let fraction = if self.text_mode {
      (self.navigation_position().page.0 + 1) as f32
        / self.doc.pages.len() as f32
    } else {
      let bottom = self.reader.scroll + self.layout.screen.height();
      bottom / self.layout.total.max(1.0)
    };
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
  }

  /// Document title for the terminal window.
  pub fn title(&self) -> String {
    let name = self.name();
    if self.doc.title.is_empty() {
      return name;
    }
    format!("{} — {name}", self.doc.title)
  }

  /// Tab names that fit in `room` beside `detail` columns, with their
  /// display width; else the active file's name.
  fn tabs(&self, room: usize, detail: usize) -> Tabs {
    let Tabline { names, active } = &self.tabline;
    // Narrow panes cut the name so the status line stays in the pane.
    let file = || {
      let label = format!(" {}:{} ", active + 1, self.name());
      let label = terminal::clean(&label, room.saturating_sub(1));
      let gap = if room > 0 { " " } else { "" };
      let width = label.width() + gap.width();
      let text = format!("{}{label}{}{gap}", theme::STATUS_FILE, theme::BAR);
      Tabs { text, width, labels: Vec::new() }
    };
    if names.len() < 2 {
      return file();
    }
    let labels: Vec<_> = names
      .iter()
      .enumerate()
      .map(|(index, name)| format!(" {}:{name} ", index + 1))
      .collect();
    let width = labels.iter().map(|label| label.width()).sum::<usize>() + 1;
    // Names yield to the hover detail.
    if width + detail > room {
      return file();
    }
    let mut text = String::new();
    for (index, label) in labels.iter().enumerate() {
      let style =
        if index == *active { theme::TAB_ACTIVE } else { theme::TAB_INACTIVE };
      text.extend([style, label.as_str(), theme::BAR]);
    }
    let labels = labels.iter().map(|label| label.width()).collect();
    Tabs { text: text + " ", width, labels }
  }

  /// Status-line text for the hover.
  pub(crate) fn hover_detail(&mut self) -> Option<String> {
    match self.hover {
      Hit::Note { page, index, .. } => {
        self.note_line(page, index).ok().flatten()
      }
      Hit::Row(_) => self.panel.as_ref()?.hover_title.clone(),
      Hit::Card(row) => self.panel.as_ref()?.shelf.as_ref()?.detail(row),
      Hit::Thumbnail(page) => Some(self.page_line(page)),
      Hit::Link(..) => self.hover_label(),
      Hit::Preview { .. } => self.preview_label(),
      Hit::Nothing
      | Hit::Text
      | Hit::Item(..)
      | Hit::Edge { .. }
      | Hit::Chrome
      | Hit::Scrollbar => None,
    }
  }

  /// A page's label and the section the reader names at its top.
  pub(crate) fn page_line(&mut self, page: PageId) -> String {
    let section = self.top_section(page);
    let label = self.doc.label(page);
    match section.map(|index| self.doc.outline[index].title.trim()) {
      Some(title) => format!("p. {label} · {title}"),
      None => format!("p. {label}"),
    }
  }

  /// Mode, tabs, message, then pending keys and position.
  pub fn status(&self, out: &mut impl Write, row: u16) -> Result<()> {
    self.status_line(out, row, None)
  }

  /// `status`, `detail` replacing the section.
  pub fn status_line(
    &self,
    out: &mut impl Write,
    row: u16,
    detail: Option<&str>,
  ) -> Result<()> {
    let line = self.line(detail)?;
    write!(out, "{}", self.at.goto(row, 0u16))?;
    let Some(tabs) = &line.tabs else {
      write!(out, "{}{}{}", theme::BAR, line.right, theme::RESET)?;
      // Plain frames skip clears; erase the rest.
      let rest =
        usize::from(self.screen.columns).saturating_sub(line.right.width());
      if rest > 0 {
        write!(out, "\x1b[{rest}X")?;
      }
      return Ok(());
    };
    let badge =
      if self.focused { theme::badge(line.color) } else { theme::BAR.into() };
    write!(
      out,
      "{badge}{}{}{}{}{}{}{}{}{}",
      line.left,
      theme::BAR,
      tabs.text,
      line.style,
      line.message,
      theme::BAR,
      " ".repeat(line.fill),
      line.right,
      theme::RESET
    )?;
    Ok(())
  }

  /// The status line's parts.
  fn line(&self, detail: Option<&str>) -> Result<Line> {
    let columns = usize::from(self.screen.columns);
    let (mode, color) = self.mode();
    let page = self.navigation_position().page.0;
    let mut right = self.keys.pending();
    if !right.is_empty() {
      right.push_str("  ");
    }
    let (mut pages, mut zoom) = (0..0, 0..0);
    // The recent view's document is closed; it shows no position.
    let covered = self.covered();
    if !covered {
      right += &self.search_count();
      let start = right.width();
      write!(
        right,
        "{}/{} [{}] {}% ",
        page + 1,
        self.doc.pages.len(),
        self.doc.label(PageId(page)),
        self.progress(),
      )?;
      pages = start..right.width();
    }
    if !covered && (self.layout.zoom - 1.0).abs() > 0.005 {
      let start = right.width();
      write!(right, " zoom {:.0}% ", self.layout.zoom * 100.0)?;
      zoom = start..right.width();
    }
    let left = format!(" {mode} ");
    let mut line = Line {
      left,
      color,
      tabs: None,
      message: String::new(),
      style: theme::BAR,
      section: false,
      fill: 0,
      right,
      pages,
      zoom,
    };
    let Some(room) =
      columns.checked_sub(line.left.width() + line.right.width() + 1)
    else {
      line.right = terminal::clean(&line.right, columns);
      return Ok(line);
    };
    let section =
      self.current_section.map(|index| self.doc.outline[index].title.trim());
    let detail = detail.filter(|_| self.shows_detail());
    let (message, style) = match (&self.error, section) {
      (Some(error), _) => (error.as_str(), theme::ERROR),
      (None, _) if let Some(detail) = detail => (detail, theme::BAR),
      (None, _) if covered => ("Enter open  :e file  q close", theme::BAR),
      (None, Some(title)) if self.reader.message.is_empty() => {
        line.section = true;
        (title, theme::STATUS_SECTION)
      }
      (None, _) => (self.reader.message.as_str(), theme::BAR),
    };
    let tabs = self.tabs(room, detail.map_or(0, UnicodeWidthStr::width));
    let room = room.saturating_sub(tabs.width);
    // A cut detail says so; `K` shows notes whole.
    line.message = match detail {
      Some(_) => terminal::clipped(message, room).0,
      None => terminal::clean(message, room),
    };
    line.fill = room.saturating_sub(line.message.width()) + 1;
    line.style = style;
    line.tabs = Some(tabs);
    Ok(line)
  }

  /// Whether a detail would show: messages and errors stay until a
  /// key or click.
  pub(crate) fn shows_detail(&self) -> bool {
    self.error.is_none() && (self.covered() || self.reader.message.is_empty())
  }

  /// The focused pane's status item there.
  pub(crate) fn status_item(&self, column: u16) -> Option<Chrome> {
    // Overlays take the press first.
    if !self.focused || self.overlaid() {
      return None;
    }
    // A pointer on this row hovers chrome, which has no detail.
    let line = self.line(None).ok()?;
    let column = usize::from(column);
    let columns = usize::from(self.screen.columns);
    let start = if line.tabs.is_some() {
      columns.saturating_sub(line.right.width())
    } else {
      0
    };
    let at = column.checked_sub(start);
    // Thumbnails take neither; text has none.
    let paged = self.overview.is_none();
    if paged && !self.text_mode && at.is_some_and(|at| line.pages.contains(&at))
    {
      return Some(Chrome::Pages);
    }
    if paged && at.is_some_and(|at| line.zoom.contains(&at)) {
      return Some(Chrome::Zoom);
    }
    let tabs = line.tabs?;
    let mut edge = line.left.width();
    for (index, width) in tabs.labels.iter().enumerate() {
      if (edge..edge + width).contains(&column) {
        return Some(Chrome::Tab(index));
      }
      edge += width;
    }
    let message = line.left.width() + tabs.width;
    let section = message..message + line.message.width();
    (line.section && section.contains(&column)).then_some(Chrome::Section)
  }
}

/// Listed tabs, or the active file's name.
#[derive(Debug)]
struct Tabs {
  text: String,
  width: usize,
  /// Listed tabs' widths; empty for the file.
  labels: Vec<usize>,
}

/// Status line parts; spans in columns.
struct Line {
  left: String,
  color: Color,
  /// None when only `right` fits.
  tabs: Option<Tabs>,
  message: String,
  style: &'static str,
  /// The message is the section name.
  section: bool,
  /// Blanks between the message and `right`.
  fill: usize,
  right: String,
  /// The page counter within `right`.
  pages: std::ops::Range<usize>,
  /// The zoom label within `right`.
  zoom: std::ops::Range<usize>,
}

/// Prompt on the last row, its list above, a ghost suggestion, and a
/// block cursor; the `pointed` completion shows as chosen.
pub fn prompt(
  out: &mut impl Write,
  prompt: &Prompt,
  screen: Screen,
  at: At,
  pointed: Option<usize>,
) -> Result<()> {
  let row = screen.rows - 1;
  if let Some(menu) = &prompt.menu {
    let layout = menu.grid(screen);
    let bottom = (at, row - 1);
    let shown = pointed.or(menu.selected);
    grid::draw(out, bottom, &menu.labels, &layout, shown, 0, screen)?;
  }
  let (text, ghost, column) = prompt.display();
  let room = usize::from(screen.columns).saturating_sub(text.width());
  write!(
    out,
    "{}{}",
    at.goto(row, 0u16),
    terminal::clean(&text, usize::from(screen.columns))
  )?;
  let ghost = terminal::clean(ghost, room);
  let pad = " ".repeat(room.saturating_sub(ghost.width()));
  write!(out, "{}{ghost}{}{pad}", theme::MUTED, theme::RESET)?;
  let column = column.min(usize::from(screen.columns) - 1);
  write!(out, "{}\x1b[2 q\x1b[?25h", at.goto(row, column))?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  use std::path::PathBuf;
  impl Tabs {
    fn pair(self) -> (String, usize) {
      (self.text, self.width)
    }
  }

  fn app() -> Result<App> {
    open("paper.pdf", true)
  }

  fn open(name: &str, text_mode: bool) -> Result<App> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    App::with_screen(
      &root.join(name),
      "",
      text_mode,
      Screen {
        columns: 120,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
        origin: (0, 0),
      },
    )
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn filenames_keep_style_and_number() -> Result<()> {
    let mut app = app()?;
    let single = format!("{} 1:paper.pdf {} ", theme::STATUS_FILE, theme::BAR);
    assert_eq!(app.tabs(80, 0).pair(), (single.clone(), 14));
    app.tabline = Tabline { names: vec!["paper.pdf".into()], active: 0 };
    assert_eq!(app.tabs(80, 0).pair(), (single, 14));
    app.tabline.names.insert(0, "book.pdf".into());
    for active in 0..2 {
      app.tabline.active = active;
      let styles = if active == 0 {
        [theme::TAB_ACTIVE, theme::TAB_INACTIVE]
      } else {
        [theme::TAB_INACTIVE, theme::TAB_ACTIVE]
      };
      let expected = format!(
        "{} 1:book.pdf {}{} 2:paper.pdf {} ",
        styles[0],
        theme::BAR,
        styles[1],
        theme::BAR
      );
      assert_eq!(app.tabs(80, 0).pair(), (expected, 26));
    }
    let narrow = format!("{} 2:paper.pdf {} ", theme::STATUS_FILE, theme::BAR);
    assert_eq!(app.tabs(14, 0).pair(), (narrow, 14));
    app.path = "論文.pdf".into();
    for room in 0..30 {
      let (tabs, width) = app.tabs(room, 0).pair();
      let plain = tabs
        .replace(theme::STATUS_FILE, "")
        .replace(theme::TAB_INACTIVE, "")
        .replace(theme::BAR, "");
      assert_eq!(width, plain.width(), "{room}");
      assert!(width <= room, "{room}");
    }
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn section_has_separate_style() -> Result<()> {
    let mut app = app()?;
    app.doc.load_outline()?;
    app.current_section = Some(0);
    let title = app.doc.outline[0].title.trim();
    let mut out = Vec::new();
    app.status(&mut out, 23)?;
    let out = String::from_utf8(out)?;
    assert!(out.contains(&format!("{} 1:paper.pdf ", theme::STATUS_FILE)));
    let section = format!("{}{title}{}", theme::STATUS_SECTION, theme::BAR);
    assert!(out.contains(&section));
    Ok(())
  }

  /// U20: thumbnails name the section the reader shows at their top,
  /// and landings name the heading they land on.
  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn sections_agree_with_the_reader() -> Result<()> {
    use crate::document::Target;
    // (file, text mode, Cursor mode)
    let cases = ["paper.pdf", "book.pdf"].map(|file| {
      [(false, false), (false, true), (true, false)]
        .map(|(text, cursor)| (file, text, cursor))
    });
    for (file, text_mode, cursor) in cases.into_iter().flatten() {
      let name = format!("{file} text={text_mode} cursor={cursor}");
      let mut app = open(file, text_mode)?;
      app.doc.load_outline()?;
      if cursor {
        app.reader.enter_cursor();
      }
      for page in (0..app.doc.pages.len()).map(PageId) {
        app.go_to_page(page)?;
        app.track_outline();
        let line = app.page_line(page);
        let shown = line.split_once(" · ").map(|(_, title)| title);
        let section = app.current_section;
        let title = section.map(|index| app.doc.outline[index].title.trim());
        assert_eq!(shown, title, "{name} p. {}", page.0 + 1);
      }
      let outline = app.doc.outline.clone();
      for (index, heading) in outline.iter().enumerate() {
        // Page targets go to the page top; nothing lands.
        let Target::Point { .. } = heading.target else { continue };
        app.follow(&heading.target)?;
        app.track_outline();
        let Some(section) = app.current_section else {
          panic!("{name} {index}: no section");
        };
        // Duplicate entries share a target.
        let target = &outline[section].target;
        assert_eq!(*target, heading.target, "{name} {index}: {section}");
      }
    }
    Ok(())
  }
}
