use crate::Result;
use crate::find::Search;
use crate::input::InputOwner;
use crate::keys::{Action, Keys, PanelKind};
use crate::layout::{Layout, Screen};
use crate::panel::Panel;
use crate::region::Region;
use crate::text::PageId;
use crate::theme::Badge;
use std::fmt::Write as _;
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
  pub capture: Option<&'a Region>,
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
  pub fn width(&self, room: usize) -> usize {
    self.fit(room, 0).width
  }

  pub fn scene<'a>(&self, scene: &mut impl Draw<'a>, room: usize) -> usize {
    let tabs = self.fit(room, 0);
    tabs.scene(scene);
    tabs.width
  }

  pub fn item(&self, room: usize, column: usize) -> Option<Item> {
    let tabs = self.fit(room, 0);
    let mut edge = 0;
    for (index, width) in tabs.labels.iter().enumerate() {
      if (edge..edge + width).contains(&column) {
        return Some(Item::Tab(index));
      }
      edge += width;
    }
    None
  }

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
  fn mode(&self) -> (&'static str, Badge) {
    if self.view.keys.resizing() {
      return ("RESIZE", Badge::Transient);
    }
    match self.view.owner {
      InputOwner::Prompt => return ("PROMPT", Badge::Transient),
      InputOwner::Hints => return ("HINT", Badge::Transient),
      InputOwner::Context => return ("MENU", Badge::Transient),
      InputOwner::Peek => return ("PEEK", Badge::Transient),
      InputOwner::Region => {
        let label = if self.view.capture.is_some_and(Region::is_bar) {
          "ACTIONS"
        } else {
          "REGION"
        };
        return (label, Badge::Transient);
      }
      InputOwner::Panel => {
        if let Some(panel) = self.view.panel {
          return (panel.kind().title(), Badge::Cursor);
        }
      }
      InputOwner::Overview => return ("PAGES", Badge::Cursor),
      InputOwner::Content => {}
    }
    match self.view.reader.mode() {
      Mode::Visual(SelectionKind::Character) => ("VISUAL", Badge::Visual),
      Mode::Visual(SelectionKind::Line) => ("V-LINE", Badge::Visual),
      _ if self.view.reader.operator.is_some() => ("YANK", Badge::Transient),
      Mode::Cursor => ("CURSOR", Badge::Cursor),
      Mode::Normal => ("NORMAL", Badge::Normal),
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
    let badge = if focused { Role::Badge(line.badge) } else { Role::Bar };
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
    let (mode, badge) = self.mode();
    let page = self.document.page.0;
    let mut right = self.view.keys.pending();
    if !right.is_empty() {
      right.push_str("  ");
    }
    let pages;
    let (mut zoom, mut matches) = (0..0, 0..0);
    {
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
    if (self.view.layout.zoom() - 1.0).abs() > 0.005 {
      let start = right.width();
      write!(right, " zoom {:.0}% ", self.view.layout.zoom() * 100.0)?;
      zoom = start..right.width();
    }
    let mode = if columns < 120 { &mode[..1] } else { mode };
    let left = format!(" {mode} ");
    let mut line = Line {
      left,
      badge,
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
    shows_detail(self.error, &self.view.reader.message, false)
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
  badge: Badge,
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
  use crate::testing::{spans, text};
  impl Tabs {
    fn pair(self) -> (Vec<(Role, String)>, usize) {
      let mut scene = Vec::new();
      self.scene(&mut scene);
      (spans(&scene), self.width)
    }
  }
  #[test]
  fn narrow_modes_preserve_section() -> Result<()> {
    let mut reader = Reader::default();
    let screen = crate::testing::screen::cells(40, 24);
    let layout = Layout::new([], screen);
    let search = Search::default();
    let keys = Keys::default();
    for cursor in [false, true] {
      if cursor {
        reader.enter_cursor();
      }
      let full = if cursor { "CURSOR" } else { "NORMAL" };
      for columns in [20, 40, 80, 119, 120, 160] {
        let bar = Bar {
          view: View {
            reader: &reader,
            layout: &layout,
            search: &search,
            keys: &keys,
            panel: None,
            owner: InputOwner::Content,
            capture: None,
            text: true,
            overview: false,
          },
          document: Document {
            page: PageId(0),
            pages: 1,
            label: "1",
            section: Some("Introduction"),
          },
          names: Names { file: "p.pdf", names: &[], active: 0 },
          screen: Screen { columns, ..screen },
          error: None,
        };
        let line = bar.line(None)?;
        let label = if columns < 120 { &full[..1] } else { full };
        assert_eq!(line.left, format!(" {label} "));
        let mut scene = Vec::new();
        bar.scene(
          &mut scene,
          vpdf_model::geometry::At::default(),
          23,
          true,
          None,
        )?;
        let rows = crate::testing::lines(&scene, columns);
        assert_eq!(rows[&23].width(), usize::from(columns));
        if columns != 40 {
          continue;
        }
        assert_eq!(line.message, "Introduction");
        let start = line.left.width() + line.tabs.as_ref().unwrap().width;
        for column in start..start + line.message.width() {
          assert_eq!(bar.hit(column as u16, 23, false), Some(Item::Section));
        }
        assert_eq!(bar.hit((start - 1) as u16, 23, false), None);
        assert_eq!(
          bar.hit((start + line.message.width()) as u16, 23, false),
          None
        );
      }
    }
    Ok(())
  }

  #[test]
  fn unmeasured_resize_shows_on_the_last_row() {
    let mut scene = Vec::new();
    Notice::new(10, 12, "Terminal reports no cell size".into())
      .scene(&mut scene);
    assert_eq!(scene.first(), Some(&Op::SaveCursor));
    assert!(scene.contains(&Op::Move { row: 9, column: 0 }));
    assert_eq!(text(&scene), "Terminal rep");
    assert_eq!(scene.last(), Some(&Op::RestoreCursor));
  }
  #[test]
  fn tab_controls_are_not_drawing_commands() {
    let names = ["safe.pdf".into(), "bad\n\x1b[31m.pdf".into()];
    let tabs = Names { file: "safe.pdf", names: &names, active: 0 };
    let (spans, width) = tabs.fit(200, 0).pair();
    let plain: String = spans.iter().map(|(_, s)| s.as_str()).collect();
    assert_eq!(plain, " 1:safe.pdf  2:bad [31m.pdf  ");
    assert_eq!(width, plain.width());
  }
  #[test]
  fn filenames_keep_style_and_number() {
    let mut tabs = Names { file: "paper.pdf", names: &[], active: 0 };
    let single =
      vec![(Role::StatusFile, " 1:paper.pdf ".into()), (Role::Bar, " ".into())];
    assert_eq!(tabs.fit(80, 0).pair(), (single.clone(), 14));
    let names = ["paper.pdf".into()];
    tabs.names = &names;
    assert_eq!(tabs.fit(80, 0).pair(), (single, 14));
    let names = ["book.pdf".into(), "paper.pdf".into()];
    tabs.names = &names;
    for active in 0..2 {
      tabs.active = active;
      let styles = if active == 0 {
        [Role::TabActive, Role::TabInactive]
      } else {
        [Role::TabInactive, Role::TabActive]
      };
      assert_eq!(
        tabs.fit(80, 0).pair(),
        (
          vec![
            (styles[0], " 1:book.pdf ".into()),
            (styles[1], " 2:paper.pdf ".into()),
            (Role::Bar, " ".into())
          ],
          26
        )
      );
    }
    let narrow =
      vec![(Role::StatusFile, " 2:paper.pdf ".into()), (Role::Bar, " ".into())];
    assert_eq!(tabs.fit(14, 0).pair(), (narrow, 14));
    tabs.file = "論文.pdf";
    for room in 0..30 {
      let (spans, width) = tabs.fit(room, 0).pair();
      let plain: String = spans.iter().map(|(_, s)| s.as_str()).collect();
      assert_eq!(width, plain.width(), "{room}");
      assert!(width <= room);
    }
  }
}
