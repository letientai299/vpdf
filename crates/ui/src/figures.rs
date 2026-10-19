use crate::Result;
use crate::source::Figures;
use vpdf_model::document::{Figure, FigureSource};
use vpdf_model::text::{Bounds, PageId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
  pub page: PageId,
  pub x: f32,
  pub y: f32,
}

pub fn advance(
  source: &mut impl Figures,
  current: Point,
  forward: bool,
  count: usize,
) -> Result<Option<(PageId, Bounds)>> {
  let mut remaining = count.max(1);
  let mut chosen = None;
  let pages = source.count();
  for offset in 0..pages {
    let page = if forward {
      current.page.0.checked_add(offset).filter(|p| *p < pages)
    } else {
      current.page.0.checked_sub(offset)
    };
    let Some(page) = page.map(PageId) else { break };
    let figures = source.figures(page)?;
    for offset in 0..figures.len() {
      let index = if forward { offset } else { figures.len() - offset - 1 };
      let bounds = figures[index].bounds;
      let order = compare(
        (page, bounds.y0, bounds.x0),
        (current.page, current.y, current.x),
      );
      if order
        != if forward {
          std::cmp::Ordering::Greater
        } else {
          std::cmp::Ordering::Less
        }
      {
        continue;
      }
      chosen = Some((page, bounds));
      remaining -= 1;
      if remaining == 0 {
        break;
      }
    }
    if remaining == 0 {
      break;
    }
  }
  Ok(chosen)
}

pub fn at(source: &mut impl Figures, point: Point) -> Result<Option<Figure>> {
  let figures = source.figures(point.page)?;
  let under =
    figures.iter().filter(|figure| figure.bounds.contains(point.x, point.y));
  // Graphics win over lone captions.
  Ok(under.min_by_key(|figure| figure.source != FigureSource::Graphic).cloned())
}

pub fn nearest(
  source: &mut impl Figures,
  point: Point,
) -> Result<Option<Bounds>> {
  let figures = source.figures(point.page)?;
  Ok(
    figures
      .iter()
      .min_by(|a, b| {
        distance(a.bounds, point.x, point.y)
          .total_cmp(&distance(b.bounds, point.x, point.y))
      })
      .map(|figure| figure.bounds),
  )
}

fn compare(a: (PageId, f32, f32), b: (PageId, f32, f32)) -> std::cmp::Ordering {
  a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2))
}

fn distance(bounds: Bounds, x: f32, y: f32) -> f32 {
  let dx = (bounds.x0 - x).max(x - bounds.x1).max(0.0);
  let dy = (bounds.y0 - y).max(y - bounds.y1).max(0.0);
  dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::source::Pages;
  use std::sync::Arc;

  struct Source {
    pages: Vec<Arc<Vec<Figure>>>,
    requested: Vec<PageId>,
    fail: Option<PageId>,
  }

  impl Pages for Source {
    fn count(&self) -> usize {
      self.pages.len()
    }
    fn bounds(&self, _page: PageId) -> Bounds {
      unreachable!()
    }
    fn label(&self, _page: PageId) -> &str {
      unreachable!()
    }
  }

  impl Figures for Source {
    fn figures(&mut self, page: PageId) -> Result<Arc<Vec<Figure>>> {
      self.requested.push(page);
      anyhow::ensure!(self.fail != Some(page), "Figure load failed");
      Ok(Arc::clone(&self.pages[page.0]))
    }
  }

  fn figure(x: f32, y: f32) -> Figure {
    Figure {
      bounds: Bounds { x0: x, y0: y, x1: x + 10.0, y1: y + 10.0 },
      caption: String::new(),
      source: FigureSource::Graphic,
    }
  }

  fn source() -> Source {
    Source {
      pages: vec![
        Arc::new(vec![
          figure(10.0, 10.0),
          figure(30.0, 10.0),
          figure(10.0, 30.0),
        ]),
        Arc::default(),
        Arc::new(vec![figure(5.0, 5.0)]),
      ],
      requested: Vec::new(),
      fail: None,
    }
  }

  #[test]
  fn counts_cross_empty_pages() -> Result<()> {
    let first = Point { page: PageId(0), x: 10.0, y: 10.0 };
    let last = Point { page: PageId(2), x: 5.0, y: 5.0 };
    for (point, forward, count, expected, requested) in [
      (first, true, 0, Some((0, 30.0, 10.0)), vec![0]),
      (first, true, 2, Some((0, 10.0, 30.0)), vec![0]),
      (first, true, 3, Some((2, 5.0, 5.0)), vec![0, 1, 2]),
      (first, true, 99, Some((2, 5.0, 5.0)), vec![0, 1, 2]),
      (first, false, 1, None, vec![0]),
      (last, true, 1, None, vec![2]),
      (last, false, 1, Some((0, 10.0, 30.0)), vec![2, 1, 0]),
      (last, false, 99, Some((0, 10.0, 10.0)), vec![2, 1, 0]),
    ] {
      let mut source = source();
      let selected = advance(&mut source, point, forward, count)?;
      assert_eq!(selected.map(|(page, b)| (page.0, b.x0, b.y0)), expected);
      assert_eq!(
        source.requested,
        requested.into_iter().map(PageId).collect::<Vec<_>>()
      );
    }
    Ok(())
  }

  #[test]
  fn failures_stop_lazy_navigation() -> Result<()> {
    let mut source = source();
    source.fail = Some(PageId(1));
    let point = Point { page: PageId(0), x: 10.0, y: 10.0 };
    assert!(advance(&mut source, point, true, 1)?.is_some());
    source.requested.clear();
    let error = advance(&mut source, point, true, 3).unwrap_err();
    assert_eq!(error.to_string(), "Figure load failed");
    assert_eq!(source.requested, vec![PageId(0), PageId(1)]);
    Ok(())
  }

  #[test]
  fn graphics_win_pointer_overlap() -> Result<()> {
    let mut source = source();
    let mut caption = figure(10.0, 10.0);
    caption.source = FigureSource::Caption;
    source.pages[0] = Arc::new(vec![caption, figure(5.0, 5.0)]);
    let point = Point { page: PageId(0), x: 15.0, y: 15.0 };
    assert_eq!(at(&mut source, point)?.unwrap().source, FigureSource::Graphic);
    assert_eq!(nearest(&mut source, point)?, Some(figure(10.0, 10.0).bounds));
    let outside = Point { x: 100.0, y: 100.0, ..point };
    assert!(at(&mut source, outside)?.is_none());
    assert!(nearest(&mut source, outside)?.is_some());
    let empty = Point { page: PageId(1), ..point };
    assert!(at(&mut source, empty)?.is_none());
    assert!(nearest(&mut source, empty)?.is_none());
    Ok(())
  }
}
