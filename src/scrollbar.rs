use crate::Result;
use crate::app::App;
use std::time::Instant;
use vpdf_ui::pointer::{Hit, Route};
use vpdf_ui::scrollbar::{Input, Ticks, View};

impl App {
  fn has_bar(&self) -> bool {
    vpdf_ui::scrollbar::available(
      &self.layout,
      self.text_mode,
      self.overlays.overview().is_some(),
      self.covered(),
    )
  }

  pub(crate) fn on_bar(&self, column: u16) -> bool {
    vpdf_ui::scrollbar::hit(
      column,
      self.screen.columns,
      self.has_bar(),
      self.overlaid(),
    )
  }

  pub fn bar_until(&self) -> Option<Instant> {
    self.bar.deadline()
  }

  pub fn bar_expired(&mut self, now: Instant) -> bool {
    self.bar.expire(now)
  }

  pub(crate) fn drop_scrub(&mut self) -> bool {
    self.reader.release_bar(&mut self.bar)
  }

  pub(crate) fn scrub_row(&self) -> Option<u16> {
    self.bar.row()
  }

  pub(crate) fn bar_mouse(&mut self, route: &Route) -> Result<()> {
    let frame = self.frame(self.uncovered());
    let view = View {
      layout: &self.layout,
      scroll: self.reader.scroll,
      frame,
      available: self.has_bar(),
      hovered: self.hover.hit() == Hit::Scrollbar,
    };
    let scroll = match self.bar.input(&view, route) {
      Input::None => None,
      Input::Release => {
        self.drop_scrub();
        None
      }
      Input::Press(press) => {
        let origin = self.location()?;
        self.reader.press_bar(&mut self.bar, press, origin)
      }
      Input::Scroll(scroll) => Some(scroll),
    };
    if let Some(scroll) = scroll {
      self.reading_view().scrub_scroll(scroll);
    }
    Ok(())
  }

  pub(crate) fn scene_bar<'a>(
    &mut self,
    scene: &mut impl vpdf_model::scene::Draw<'a>,
  ) {
    let now = Instant::now();
    let frame = self.frame(self.uncovered());
    let view = View {
      layout: &self.layout,
      scroll: self.reader.scroll,
      frame,
      available: self.has_bar(),
      hovered: self.hover.hit() == Hit::Scrollbar,
    };
    let ticks = Ticks { outline: &self.doc.outline, search: &self.search };
    if let Some(label) = self.bar.scene(view, ticks, now, scene) {
      let text = self.page_line(label.page());
      label.scene(&text, scene);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::text::{PageId, Position};
  use vpdf_search::search::Hit as Found;
  use vpdf_ui::theme;
  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn ticks_mark_headings_and_pages_with_matches() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = crate::screen::cells(120, 40);
    let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
    app.doc.load_outline()?;
    let height = app.layout.screen().height();
    let headings = Ticks { outline: &app.doc.outline, search: &app.search }
      .positions(&app.layout, height);
    let top = app.doc.outline.iter().filter(|item| item.depth == 0).count();
    assert!(!headings.is_empty() && headings.len() <= top);
    assert!(headings.iter().all(|(_, color)| *color == theme::HEADING_TICK));
    let found = |page| {
      let at = Position { page: PageId(page), line: 0, glyph: 0 };
      Found { first: at, last: at, excerpt: String::new(), matched: 0..0 }
    };
    // Three matches on two pages: two ticks.
    let query = vpdf_search::query::Query::default();
    app.search.show(query, vec![found(2), found(2), found(9)]);
    let matches = Ticks { outline: &app.doc.outline, search: &app.search }
      .positions(&app.layout, height)
      .into_iter();
    let matches = matches.filter(|(_, color)| *color == theme::MATCH_TICK);
    let ys: Vec<_> = matches.map(|(y, _)| y).collect();
    assert_eq!(ys.len(), 2);
    assert!(ys.iter().all(|y| (0.0..height).contains(y)));
    app.search.set_hidden(true);
    assert_eq!(
      Ticks { outline: &app.doc.outline, search: &app.search }
        .positions(&app.layout, height),
      headings,
      "hidden matches mark nothing"
    );
    Ok(())
  }
}
