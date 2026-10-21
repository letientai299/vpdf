use crate::Result;
use crate::find::Search;
use crate::input::InputOwner;
use crate::keys::{Action, Keys, PanelKind};
use crate::layout::{Layout, Screen};
use crate::panel::Panel;
use crate::text::PageId;
use crate::theme::Color;
use std::fmt::Write as _;
use std::io::Write;
use unicode_width::UnicodeWidthStr;
use vpdf_model::input::MouseButton;
use vpdf_model::scene::{Draw, Op, Text};
use vpdf_model::style::Role;
use vpdf_reader::reader::{Mode, Reader, SelectionKind};

/// Error anchored to the terminal's last row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
  rows: u16,
  columns: u16,
  error: String,
}

impl Notice {
  pub fn new(rows: u16, columns: u16, error: String) -> Self {
    Self { rows, columns, error }
  }

  /// Preserve an open prompt's cursor.
  pub fn draw(&self, out: &mut impl Write) -> Result<()> {
    let mut painter = vpdf_terminal::painter::Painter::new(out);
    self.scene(&mut painter);
    painter.finish()?;
    Ok(())
  }

  pub fn scene<'a>(&self, scene: &mut impl Draw<'a>) {
    scene.extend([Op::SaveCursor, Op::Style(Role::Error)]);
    crate::paint::line(
      scene,
      vpdf_model::geometry::At::default(),
      self.rows.max(1) - 1,
      &self.error,
      self.columns,
    );
    scene.extend([Op::Style(Role::Reset), Op::RestoreCursor]);
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Item {
  Tab(usize),
  Section,
  Pages,
  Matches,
  Zoom,
}

pub enum Press {
  Tab(usize),
  CloseTab(usize),
  Section,
  Action(Action),
}

impl Item {
  pub fn press(self, button: MouseButton) -> Option<Press> {
    Some(match (button, self) {
      (MouseButton::Left, Self::Tab(index)) => Press::Tab(index),
      (MouseButton::Middle, Self::Tab(index)) => Press::CloseTab(index),
      (MouseButton::Left, Self::Section) => Press::Section,
      (MouseButton::Left, Self::Pages) => Press::Action(Action::Overview),
      (MouseButton::Left, Self::Matches) => {
        Press::Action(Action::Panel(PanelKind::Results))
      }
      (MouseButton::Left, Self::Zoom) => Press::Action(Action::Fit),
      _ => return None,
    })
  }
}

/// Borrowed state for status presentation.
pub struct View<'a> {
  pub reader: &'a Reader,
  pub layout: &'a Layout,
  pub search: &'a Search,
  pub keys: &'a Keys,
  pub panel: Option<&'a Panel>,
  pub owner: InputOwner,
  pub text: bool,
  pub overview: bool,
}

pub struct Document<'a> {
  pub page: PageId,
  pub pages: usize,
  pub label: &'a str,
  pub section: Option<&'a str>,
}

pub struct Names<'a> {
  pub file: &'a str,
  pub names: &'a [String],
  pub active: usize,
}

impl Names<'_> {
  /// Tab names that fit in `room` beside `detail` columns, with their
  /// display width; else the active file's name.
  fn fit(&self, room: usize, detail: usize) -> Tabs {
    let Names { file, names, active } = self;
    // Narrow panes cut the name so the status line stays in the pane.
    let file = || {
      let label = format!(" {}:{} ", active + 1, file);
      let label = vpdf_model::text::clean(&label, room.saturating_sub(1));
      let gap = room > 0;
      let width = label.width() + usize::from(gap);
      Tabs {
        spans: vec![(Role::StatusFile, Text::new(label))],
        gap,
        width,
        labels: Vec::new(),
      }
    };
    if names.len() < 2 {
      return file();
    }
    let labels: Vec<_> = names
      .iter()
      .enumerate()
      .map(|(index, name)| Text::new(format!(" {}:{name} ", index + 1)))
      .collect();
    let width =
      labels.iter().map(|label| label.as_str().width()).sum::<usize>() + 1;
    if width + detail > room {
      return file();
    }
    let widths = labels.iter().map(|label| label.as_str().width()).collect();
    let spans = labels
      .into_iter()
      .enumerate()
      .map(|(index, label)| {
        let role =
          if index == *active { Role::TabActive } else { Role::TabInactive };
        (role, label)
      })
      .collect();
    Tabs { spans, gap: true, width, labels: widths }
  }
}

pub struct Bar<'a> {
  pub view: View<'a>,
  pub document: Document<'a>,
  pub names: Names<'a>,
  pub screen: Screen,
  pub error: Option<&'a str>,
}

pub fn shows_detail(error: Option<&str>, message: &str, covered: bool) -> bool {
  error.is_none() && (covered || message.is_empty())
}

impl Bar<'_> {
  fn covered(&self) -> bool {
    self.view.panel.is_some_and(Panel::covers_page)
  }

  fn mode(&self) -> (&'static str, Color) {
    match self.view.owner {
      InputOwner::Prompt => return ("PROMPT", Color::Yellow),
      InputOwner::Hints => return ("HINT", Color::Yellow),
      InputOwner::Context => return ("MENU", Color::Yellow),
      InputOwner::Peek => return ("PEEK", Color::Yellow),
      InputOwner::Region => return ("REGION", Color::Yellow),
      InputOwner::Panel => {
        if let Some(panel) = self.view.panel {
          return (panel.kind().title(), Color::Cyan);
        }
      }
      InputOwner::Overview => return ("PAGES", Color::Cyan),
      InputOwner::Content => {}
    }
    match self.view.reader.mode() {
      Mode::Visual(SelectionKind::Character) => ("VISUAL", Color::Magenta),
      Mode::Visual(SelectionKind::Line) => ("V-LINE", Color::Magenta),
      _ if self.view.reader.operator.is_some() => ("YANK", Color::Yellow),
      Mode::Cursor => ("CURSOR", Color::Cyan),
      Mode::Normal => ("NORMAL", Color::Blue),
    }
  }

  /// Match under or before the cursor, like Neovim's searchcount.
  fn search_count(&self) -> String {
    if let Some(panel) = self.view.panel.filter(|panel| panel.focused()) {
      return panel
        .count()
        .map_or_else(String::new, |(i, n)| format!("[{i}/{n}]  "));
    }
    let hits = self.view.search.hits();
    if self.view.search.hidden() || hits.is_empty() {
      return String::new();
    }
    let more = if self.view.search.phase().running() { "+" } else { "" };
    if self.view.reader.viewport_mode() {
      return format!("[{}{more} matches]  ", hits.len());
    }
    let cursor = self.view.reader.cursor;
    let index = hits.partition_point(|hit| hit.first <= cursor);
    format!("[{index}/{}{more}]  ", hits.len())
  }

  /// Share of the document above the viewport's bottom edge.
  fn progress(&self) -> u32 {
    let fraction = if self.view.text {
      (self.document.page.0 + 1) as f32 / self.document.pages as f32
    } else {
      let bottom = self.view.reader.scroll + self.view.layout.screen().height();
      bottom / self.view.layout.total().max(1.0)
    };
    (fraction.clamp(0.0, 1.0) * 100.0).round() as u32
  }

  /// `status`, `detail` replacing the section.
  pub fn draw(
    &self,
    out: &mut impl Write,
    at: vpdf_model::geometry::At,
    row: u16,
    focused: bool,
    detail: Option<&str>,
  ) -> Result<()> {
    let mut painter = vpdf_terminal::painter::Painter::new(out);
    self.scene(&mut painter, at, row, focused, detail)?;
    painter.finish()?;
    Ok(())
  }

  pub fn scene<'a>(
    &self,
    scene: &mut impl Draw<'a>,
    at: vpdf_model::geometry::At,
    row: u16,
    focused: bool,
    detail: Option<&str>,
  ) -> Result<()> {
    let line = self.line(detail)?;
    crate::paint::move_to(scene, at, row, 0u16);
    let Some(tabs) = &line.tabs else {
      scene.push(Op::Style(Role::Bar));
      scene.text(&line.right);
      scene.push(Op::Style(Role::Reset));
      let rest =
        usize::from(self.screen.columns).saturating_sub(line.right.width());
      if rest > 0 {
        scene.push(Op::Erase(rest as u16));
      }
      return Ok(());
    };
    let badge = if focused { Role::Badge(line.color) } else { Role::Bar };
    scene.push(Op::Style(badge));
    scene.text(&line.left);
    scene.push(Op::Style(Role::Bar));
    tabs.scene(scene);
    scene.push(Op::Style(line.style));
    scene.text(&line.message);
    scene.extend([Op::Style(Role::Bar), Op::Spaces(line.fill)]);
    scene.text(&line.right);
    scene.push(Op::Style(Role::Reset));
    Ok(())
  }

  /// The status line's parts.
  fn line(&self, detail: Option<&str>) -> Result<Line> {
    let columns = usize::from(self.screen.columns);
    let (mode, color) = self.mode();
    let page = self.document.page.0;
    let mut right = self.view.keys.pending();
    if !right.is_empty() {
      right.push_str("  ");
    }
    let (mut pages, mut zoom, mut matches) = (0..0, 0..0, 0..0);
    // The recent view's document is closed; it shows no position.
    let covered = self.covered();
    if !covered {
      let start = right.width();
      right += &self.search_count();
      if !self.view.panel.is_some_and(Panel::focused) {
        matches = start..right.width();
      }
      let start = right.width();
      write!(
        right,
        "{}/{} [{}] {}% ",
        page + 1,
        self.document.pages,
        self.document.label,
        self.progress(),
      )?;
      pages = start..right.width();
    }
    if !covered && (self.view.layout.zoom() - 1.0).abs() > 0.005 {
      let start = right.width();
      write!(right, " zoom {:.0}% ", self.view.layout.zoom() * 100.0)?;
      zoom = start..right.width();
    }
    let left = format!(" {mode} ");
    let mut line = Line {
      left,
      color,
      tabs: None,
      message: String::new(),
      style: Role::Bar,
      section: false,
      fill: 0,
      right,
      pages,
      zoom,
      matches,
    };
    let Some(room) =
      columns.checked_sub(line.left.width() + line.right.width() + 1)
    else {
      line.right = vpdf_model::text::clean(&line.right, columns);
      return Ok(line);
    };
    let section = self.document.section;
    let detail = detail.filter(|_| self.shows_detail());
    let (message, style) = match (self.error, section) {
      (Some(error), _) => (error, Role::Error),
      (None, _) if let Some(detail) = detail => (detail, Role::Bar),
      (None, _) if covered => ("Enter open  :e file  q close", Role::Bar),
      (None, Some(title)) if self.view.reader.message.is_empty() => {
        line.section = true;
        (title, Role::StatusSection)
      }
      (None, _) => (self.view.reader.message.as_str(), Role::Bar),
    };
    let tabs = self.names.fit(room, detail.map_or(0, UnicodeWidthStr::width));
    let room = room.saturating_sub(tabs.width);
    // A cut detail says so; `K` shows notes whole.
    line.message = match detail {
      Some(_) => vpdf_model::text::clipped(message, room).0,
      None => vpdf_model::text::clean(message, room),
    };
    line.fill = room.saturating_sub(line.message.width()) + 1;
    line.style = style;
    line.tabs = Some(tabs);
    Ok(line)
  }

  /// Whether a detail would show: messages and errors stay until a
  /// key or click.
  fn shows_detail(&self) -> bool {
    shows_detail(self.error, &self.view.reader.message, self.covered())
  }

  /// The pane's status item there.
  pub fn hit(&self, column: u16, row: u16, blocked: bool) -> Option<Item> {
    // Overlays take the press first.
    if blocked || row + 1 != self.screen.rows {
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
    if at.is_some_and(|at| line.matches.contains(&at)) {
      return Some(Item::Matches);
    }
    // Thumbnails take neither; text has none.
    let paged = !self.view.overview;
    if paged && !self.view.text && at.is_some_and(|at| line.pages.contains(&at))
    {
      return Some(Item::Pages);
    }
    if paged && at.is_some_and(|at| line.zoom.contains(&at)) {
      return Some(Item::Zoom);
    }
    let tabs = line.tabs?;
    let mut edge = line.left.width();
    for (index, width) in tabs.labels.iter().enumerate() {
      if (edge..edge + width).contains(&column) {
        return Some(Item::Tab(index));
      }
      edge += width;
    }
    let message = line.left.width() + tabs.width;
    let section = message..message + line.message.width();
    (line.section && section.contains(&column)).then_some(Item::Section)
  }
}

/// Listed tabs, or the active file's name.
#[derive(Debug)]
struct Tabs {
  spans: Vec<(Role, Text<'static>)>,
  gap: bool,
  width: usize,
  /// Listed tabs' widths; empty for the file.
  labels: Vec<usize>,
}

impl Tabs {
  fn scene<'a>(&self, scene: &mut impl Draw<'a>) {
    for (role, text) in &self.spans {
      scene.push(Op::Style(*role));
      scene.text(text.as_str());
      scene.push(Op::Style(Role::Bar));
    }
    if self.gap {
      scene.text(" ");
    }
  }
}

/// Status line parts; spans in columns.
struct Line {
  left: String,
  color: Color,
  /// None when only `right` fits.
  tabs: Option<Tabs>,
  message: String,
  style: Role,
  /// The message is the section name.
  section: bool,
  /// Blanks between the message and `right`.
  fill: usize,
  right: String,
  /// The page counter within `right`.
  pages: std::ops::Range<usize>,
  matches: std::ops::Range<usize>,
  /// The zoom label within `right`.
  zoom: std::ops::Range<usize>,
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_terminal::style::sgr;

  #[test]
  fn unmeasured_resize_shows_on_the_last_row() -> Result<()> {
    let error = vpdf_terminal::terminal::NO_CELL.to_owned();
    let unmeasured = Notice::new(10, 12, error);
    let mut out = Vec::new();
    unmeasured.draw(&mut out)?;
    let want = format!(
      "\x1b7{}\x1b[10;1HTerminal rep{}\x1b8",
      sgr(Role::Error),
      sgr(Role::Reset)
    );
    assert_eq!(String::from_utf8(out)?, want);
    Ok(())
  }

  impl Tabs {
    fn pair(self) -> (String, usize) {
      let mut out = Vec::new();
      let mut painter = vpdf_terminal::painter::Painter::new(&mut out);
      self.scene(&mut painter);
      painter.finish().expect("buffer output");
      (String::from_utf8(out).expect("UTF-8 tabs"), self.width)
    }
  }

  #[test]
  fn tab_controls_are_not_drawing_commands() {
    let names = ["safe.pdf".into(), "bad\n\x1b[31m.pdf".into()];
    let tabs = Names { file: "safe.pdf", names: &names, active: 0 };
    let (output, width) = tabs.fit(200, 0).pair();
    assert!(!output.contains("\x1b[31m"));
    let plain = output
      .replace(sgr(Role::TabActive), "")
      .replace(sgr(Role::TabInactive), "")
      .replace(sgr(Role::Bar), "");
    assert_eq!(plain, " 1:safe.pdf  2:bad [31m.pdf  ");
    assert_eq!(width, plain.width());
  }

  #[test]
  fn filenames_keep_style_and_number() {
    let mut tabs = Names { file: "paper.pdf", names: &[], active: 0 };
    let single =
      format!("{} 1:paper.pdf {} ", sgr(Role::StatusFile), sgr(Role::Bar));
    assert_eq!(tabs.fit(80, 0).pair(), (single.clone(), 14));
    let names = ["paper.pdf".into()];
    tabs.names = &names;
    assert_eq!(tabs.fit(80, 0).pair(), (single, 14));
    let names = ["book.pdf".into(), "paper.pdf".into()];
    tabs.names = &names;
    for active in 0..2 {
      tabs.active = active;
      let styles = if active == 0 {
        [sgr(Role::TabActive), sgr(Role::TabInactive)]
      } else {
        [sgr(Role::TabInactive), sgr(Role::TabActive)]
      };
      let expected = format!(
        "{} 1:book.pdf {}{} 2:paper.pdf {} ",
        styles[0],
        sgr(Role::Bar),
        styles[1],
        sgr(Role::Bar)
      );
      assert_eq!(tabs.fit(80, 0).pair(), (expected, 26));
    }
    let narrow =
      format!("{} 2:paper.pdf {} ", sgr(Role::StatusFile), sgr(Role::Bar));
    assert_eq!(tabs.fit(14, 0).pair(), (narrow, 14));
    tabs.file = "論文.pdf";
    for room in 0..30 {
      let (tabs, width) = tabs.fit(room, 0).pair();
      let plain = tabs
        .replace(sgr(Role::StatusFile), "")
        .replace(sgr(Role::TabInactive), "")
        .replace(sgr(Role::Bar), "");
      assert_eq!(width, plain.width(), "{room}");
      assert!(width <= room, "{room}");
    }
  }
}
