use super::View;
use crate::Result;
use crate::page::Point;
use crate::source::Pages;
use crate::text::{Bounds, Position};
use std::cmp::Ordering;
use vpdf_model::document::Target;
use vpdf_reader::reader::TextSource;

pub struct Sections {
  points: Vec<Point>,
}

impl Sections {
  pub fn new<'a>(
    targets: impl Iterator<Item = &'a Target>,
    pages: usize,
  ) -> Self {
    let mut points: Vec<_> = targets
      .filter_map(Point::from_target)
      .filter(|point| point.page.0 < pages)
      .collect();
    points.sort_by(|a, b| a.compare(*b));
    points.dedup_by(|a, b| a.compare(*b).is_eq());
    Self { points }
  }
}

pub struct Section {
  point: Point,
  normal: bool,
}

impl Section {
  pub fn target(&self) -> Target {
    self.point.target()
  }
}

impl<S: TextSource + Pages> View<'_, S> {
  pub fn section(
    &mut self,
    sections: Sections,
    forward: bool,
    count: usize,
  ) -> Result<Option<Section>> {
    let landed = self.target.is_some_and(|target| self.matches_landing(target));
    let normal = self.reader.viewport_mode();
    let position = self.navigation();
    let mut headings = sections.points;
    let landing =
      if normal { self.section_landing(&headings, landed)? } else { None };
    if !forward {
      headings.reverse();
    }
    let mut destinations = Vec::new();
    for point @ Point { page, x, y } in headings {
      let order = if let Some(landing) = landing {
        point.compare(landing)
      } else if normal && !self.frame.pane.text {
        self.section_view(point).0.total_cmp(&self.reader.scroll)
      } else if page != position.page {
        page.cmp(&position.page)
      } else {
        self.source.page_text(page)?.nearest(page, x, y).cmp(&position)
      };
      if order == if forward { Ordering::Greater } else { Ordering::Less } {
        destinations.push(point);
      }
    }
    let Some(&point) = destinations
      .get(count.saturating_sub(1).min(destinations.len().saturating_sub(1)))
    else {
      return Ok(None);
    };
    Ok(Some(Section { point, normal }))
  }

  pub fn land_section(&mut self, section: &Section) -> Result<()> {
    if section.normal {
      if self.frame.pane.text {
        let cursor = self.reader.cursor;
        self.text().restore(Position { glyph: 0, ..cursor })?;
      } else {
        let (scroll, pan) = self.section_view(section.point);
        self.reader.scroll = self.layout.clamp_scroll(scroll);
        self.layout.pan_to(pan);
      }
    }
    self.landing.record(self.landed_view(self.reader.cursor));
    Ok(())
  }

  fn section_view(&self, Point { page, x, y }: Point) -> (f32, f32) {
    let bounds = Bounds { x0: x, x1: x, y0: y, y1: y };
    let point = self.layout.project(page, bounds, 0.0);
    let local = point.x0 - self.layout.origin(page, 0.0).0;
    let pan = (local - f32::from(self.frame.pane.screen.cell_width))
      .clamp(0.0, self.layout.pan_limit(page));
    (point.y0, pan)
  }

  fn section_landing(
    &mut self,
    headings: &[Point],
    landed: bool,
  ) -> Result<Option<Point>> {
    let Some(target) = *self.target else { return Ok(None) };
    if self.animation.active() || !self.visible(target)? {
      return Ok(None);
    }
    for &point @ Point { page, x, y } in headings {
      if page != target.page
        || self.source.page_text(page)?.nearest(page, x, y) != target
      {
        continue;
      }
      let landed = if landed {
        true
      } else if self.frame.pane.text {
        self.text().clamp(Position { glyph: 0, ..target })? == self.text.top()
      } else {
        let (scroll, pan) = self.section_view(point);
        self.layout.clamp_scroll(scroll).total_cmp(&self.reader.scroll).is_eq()
          && pan.total_cmp(&self.layout.pan()).is_eq()
      };
      if landed {
        return Ok(Some(point));
      }
    }
    Ok(None)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::PageId;

  #[test]
  fn headings_filter_sort_and_deduplicate() {
    let targets = [
      Target::Point { page: PageId(0), x: 30.0, y: 20.0 },
      Target::Point { page: PageId(0), x: 50.0, y: 10.0 },
      Target::Page(PageId(9)),
      Target::Uri("https://example.invalid".into()),
      Target::Page(PageId(0)),
      Target::Point { page: PageId(0), x: 30.0, y: 20.0 },
      Target::Point { page: PageId(0), x: -0.0, y: 0.0 },
    ];
    let sections = Sections::new(targets.iter(), 1);
    assert_eq!(sections.points.len(), 4);
    assert_eq!(sections.points[0].x.to_bits(), (-0.0_f32).to_bits());
    assert_eq!(sections.points[1].x.to_bits(), 0.0_f32.to_bits());
    assert_eq!(sections.points[2].target(), targets[1]);
    assert_eq!(sections.points[3].target(), targets[0]);
  }
}
