//! The status line: drawing, and clicks on its items.

use crate::Result;
use crate::app::{App, Request};
use std::io::Write;
use vpdf_model::input::MouseButton;
use vpdf_model::text::PageId;
use vpdf_ui::chrome::{Bar, Document, Item, Names, Press, View};
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::pointer::Hit;
use vpdf_ui::window::TabMove;

impl App {
  /// The status item at a pane cell.
  pub fn chrome_at(&self, column: u16, row: u16) -> Option<Item> {
    self.chrome_view(&self.name()).hit(column, row, self.overlaid())
  }

  /// Acts on a pressed status item.
  pub(crate) fn chrome_press(
    &mut self,
    item: Item,
    button: MouseButton,
  ) -> Result<()> {
    let Some(press) = item.press(button) else { return Ok(()) };
    let action = match press {
      Press::Tab(index) => {
        self.request = Some(Request::Tab(TabMove::Go(index + 1)));
        return Ok(());
      }
      Press::CloseTab(index) => {
        self.request = Some(Request::TabClose(Some(index)));
        return Ok(());
      }
      Press::Section => return self.outline_here(),
      Press::Action(action) => action,
    };
    self.action(Input::plain(action))
  }

  /// The outline, focused on the current section.
  fn outline_here(&mut self) -> Result<()> {
    let action = Action::Panel(PanelKind::Outline);
    self.action(Input::plain(action))?;
    let section = self.outline.current();
    if let Some(panel) = &mut self.panel
      && let Some(index) = section
    {
      panel.view.select(index);
    }
    Ok(())
  }
}

impl App {
  /// Document title for the terminal window.
  pub fn title(&self) -> String {
    let name = self.name();
    if self.doc.title.is_empty() {
      return name;
    }
    format!("{} — {name}", self.doc.title)
  }

  /// Status-line text for the hover.
  pub(crate) fn hover_detail(&mut self) -> Option<String> {
    match self.hover.hit() {
      Hit::Note { page, index, .. } => {
        self.note_line(page, index).ok().flatten()
      }
      Hit::Row(_) | Hit::Label(_) => {
        self.panel.as_ref()?.view.hover_title().map(str::to_owned)
      }
      Hit::PanelControl(control) => {
        let panel = &self.panel.as_ref()?.view;
        let label = panel.control_tip(control)?;
        let action = panel.control_action(control)?;
        let bindings = self.settings.bindings();
        let context = vpdf_ui::bindings::Context::Panel;
        let key = match action {
          Action::Fold(fold) => {
            vpdf_ui::context::fold_shortcut(bindings, context, fold)
          }
          _ => bindings
            .shortcut(context, &vpdf_ui::bindings::Target::Reader(action)),
        };
        Some(if key.is_empty() { label } else { format!("{label}  {key}") })
      }
      Hit::Card(row) => self.panel.as_ref()?.view.shelf()?.detail(row),
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
    self.reading_view().page_line(page)
  }

  pub(crate) fn chrome_view<'a>(&'a self, file: &'a str) -> Bar<'a> {
    let page = self.navigation_position().page;
    Bar {
      view: View {
        reader: &self.reader,
        layout: &self.layout,
        search: &self.search,
        keys: &self.keys,
        panel: self.panel.as_ref().map(|panel| &panel.view),
        owner: if self.sweeping() {
          vpdf_ui::input::InputOwner::Region
        } else {
          self.input_owner()
        },
        capture: if self.sweeping() { None } else { self.overlays.region() },
        text: self.text_mode,
        overview: self.overlays.overview().is_some(),
      },
      document: Document {
        page,
        pages: self.doc.pages.len(),
        label: self.doc.label(page),
        section: self
          .outline
          .current()
          .map(|index| self.doc.outline[index].title.trim()),
      },
      names: Names {
        file,
        names: &self.tabline.names,
        active: self.tabline.active,
      },
      screen: self.screen,
      error: self.error.as_deref(),
    }
  }

  pub fn status(
    &self,
    out: &mut impl Write,
    row: u16,
    detail: Option<&str>,
  ) -> Result<()> {
    crate::presentation::try_draw(out, None, |scene| {
      self.chrome_view(&self.name()).scene(
        scene,
        self.at,
        row,
        self.focused,
        detail,
      )
    })
  }

  pub(crate) fn shows_detail(&self) -> bool {
    vpdf_ui::chrome::shows_detail(
      self.error.as_deref(),
      &self.reader.message,
      false,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::style::Role;
  use vpdf_terminal::style::sgr;

  use std::path::PathBuf;
  fn app() -> Result<App> {
    open("paper.pdf", true)
  }

  fn open(name: &str, text_mode: bool) -> Result<App> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    App::with_screen(
      &root.join(name),
      "",
      text_mode,
      crate::screen::cells(120, 24),
    )
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn section_has_separate_style() -> Result<()> {
    let mut app = app()?;
    app.doc.load_outline()?;
    let target = app.doc.outline[0].target.clone();
    app.follow(&target)?;
    app.track_outline();
    assert_eq!(app.outline.current(), Some(0));
    let title = app.doc.outline[0].title.trim();
    let mut out = Vec::new();
    app.status(&mut out, 23, None)?;
    let out = String::from_utf8(out)?;
    assert!(out.contains(&format!("{} 1:paper.pdf ", sgr(Role::StatusFile))));
    let section =
      format!("{}{title}{}", sgr(Role::StatusSection), sgr(Role::Bar));
    assert!(out.contains(&section));
    Ok(())
  }

  /// U20: thumbnails name the section the reader shows at their top,
  /// and landings name the heading they land on.
  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn sections_agree_with_the_reader() -> Result<()> {
    use vpdf_model::document::Target;
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
        let section = app.outline.current();
        let title = section.map(|index| app.doc.outline[index].title.trim());
        assert_eq!(shown, title, "{name} p. {}", page.0 + 1);
      }
      let outline = app.doc.outline.clone();
      for (index, heading) in outline.iter().enumerate() {
        // Page targets go to the page top; nothing lands.
        let Target::Point { .. } = heading.target else { continue };
        app.follow(&heading.target)?;
        app.track_outline();
        let Some(section) = app.outline.current() else {
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
