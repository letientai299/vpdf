use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::text::{Bounds, PageId, Position};
use std::cmp::Ordering;

type Point = (PageId, f32, f32);

fn point(target: &Target) -> Option<Point> {
  match *target {
    Target::Page(page) => Some((page, 0.0, 0.0)),
    Target::Point { page, x, y } => Some((page, x, y)),
    Target::Uri(_) => None,
  }
}

fn compare(a: Point, b: Point) -> Ordering {
  a.0.cmp(&b.0).then(a.2.total_cmp(&b.2)).then(a.1.total_cmp(&b.1))
}

impl App {
  pub(crate) fn move_section(
    &mut self,
    forward: bool,
    count: usize,
  ) -> Result<()> {
    self.want_outline();
    self.outline_updates()?;
    let normal = self.reader.viewport_mode();
    let position = self.navigation_position();
    let mut headings: Vec<_> = self
      .doc
      .outline
      .iter()
      .filter_map(|heading| point(&heading.target))
      .filter(|(page, ..)| page.0 < self.doc.pages.len())
      .collect();
    headings.sort_by(|a, b| compare(*a, *b));
    headings.dedup_by(|a, b| compare(*a, *b).is_eq());
    let landing = if normal { self.section_landing(&headings)? } else { None };
    if !forward {
      headings.reverse();
    }
    let mut destinations = Vec::new();
    for point @ (page, x, y) in headings {
      let order = if let Some(landing) = landing {
        compare(point, landing)
      } else if normal && !self.text_mode {
        self.section_view(point).0.total_cmp(&self.reader.scroll)
      } else if page != position.page {
        page.cmp(&position.page)
      } else {
        self.doc.text(page)?.nearest(page, x, y).cmp(&position)
      };
      if order == if forward { Ordering::Greater } else { Ordering::Less } {
        destinations.push(point);
      }
    }
    let Some(&point @ (page, x, y)) = destinations
      .get(count.saturating_sub(1).min(destinations.len().saturating_sub(1)))
    else {
      return Ok(());
    };
    self.follow(&Target::Point { page, x, y })?;
    if normal {
      if self.text_mode {
        self.text_top =
          self.clamp_text(Position { glyph: 0, ..self.reader.cursor })?;
      } else {
        let (scroll, pan) = self.section_view(point);
        self.reader.scroll = self.layout.clamp_scroll(scroll);
        self.layout.pan = pan;
      }
    }
    self.landed = Some(self.landed_view(self.reader.cursor));
    Ok(())
  }

  fn section_view(&self, (page, x, y): Point) -> (f32, f32) {
    let bounds = Bounds { x0: x, x1: x, y0: y, y1: y };
    let point = self.layout.project(page, bounds, 0.0);
    let local = point.x0 - self.layout.origin(page, 0.0).0;
    let pan = (local - f32::from(self.screen.cell_width))
      .clamp(0.0, self.layout.pan_limit(page));
    (point.y0, pan)
  }

  fn section_landing(&mut self, headings: &[Point]) -> Result<Option<Point>> {
    let Some(target) = self.target else { return Ok(None) };
    if self.animation.is_some() || !self.visible_position(target)? {
      return Ok(None);
    }
    for &point @ (page, x, y) in headings {
      if page != target.page
        || self.doc.text(page)?.nearest(page, x, y) != target
      {
        continue;
      }
      let landed = if self.landed == Some(self.landed_view(target)) {
        true
      } else if self.text_mode {
        self.clamp_text(Position { glyph: 0, ..target })? == self.text_top
      } else {
        let (scroll, pan) = self.section_view(point);
        self.layout.clamp_scroll(scroll).total_cmp(&self.reader.scroll).is_eq()
          && pan.total_cmp(&self.layout.pan).is_eq()
      };
      if landed {
        return Ok(Some(point));
      }
    }
    Ok(None)
  }
}
