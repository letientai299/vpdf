use crate::text::{Bounds, PageId};
pub use vpdf_model::geometry::{
  Crop, Crops, Screen, Transform, clip, page_matrix, screen_step, transform,
};

#[derive(Clone, Debug)]
pub struct PageLayout {
  pub top: f32,
  pub width: f32,
  pub height: f32,
  pub scale: u32,
  pub matrix: Transform,
}

pub struct Layout {
  pub pages: Vec<PageLayout>,
  pub total: f32,
  pub screen: Screen,
  pub zoom: f32,
  pub rotation: u16,
  pub pan: f32,
  pub crop: Crop,
  pub crops: std::sync::Arc<Crops>,
}

impl Layout {
  pub fn new(pages: impl IntoIterator<Item = Bounds>, screen: Screen) -> Self {
    let mut layout = Self {
      pages: Vec::new(),
      total: 0.0,
      screen,
      zoom: 1.0,
      rotation: 0,
      pan: 0.0,
      crop: Crop::default(),
      crops: std::sync::Arc::default(),
    };
    layout.reflow(pages);
    layout
  }

  pub fn reflow(&mut self, pages: impl IntoIterator<Item = Bounds>) {
    self.pages.clear();
    let mut top = 0.0;
    for (index, page) in pages.into_iter().enumerate() {
      let bounds = self.crop_for(PageId(index)).apply(page);
      let width = if self.rotation.is_multiple_of(180) {
        bounds.width()
      } else {
        bounds.height()
      };
      let scale = (self.screen.width() / width * self.zoom).clamp(0.01, 16.0);
      let scale = (scale * 1024.0).round() as u32;
      let matrix = page_matrix(bounds, scale, self.rotation);
      let rect = transform(bounds, &matrix);
      self.pages.push(PageLayout {
        top,
        width: rect.width().ceil(),
        height: rect.height().ceil(),
        scale,
        matrix,
      });
      top += rect.height().ceil() + f32::from(self.screen.cell_height);
    }
    self.total = top;
  }

  pub fn clamp_scroll(&self, scroll: f32) -> f32 {
    scroll.clamp(0.0, (self.total - self.screen.height()).max(0.0))
  }

  pub fn crop_for(&self, page: PageId) -> Crop {
    if self.crop.is_none() {
      return self.crop;
    }
    self.crops.get(page)
  }

  pub fn visible(&self, scroll: f32) -> std::ops::Range<usize> {
    let first = self.pages.partition_point(|p| p.top + p.height < scroll);
    let last =
      self.pages.partition_point(|p| p.top < scroll + self.screen.height());
    first..last
  }

  pub fn page_at(&self, y: f32) -> PageId {
    PageId(
      self
        .pages
        .partition_point(|p| p.top <= y)
        .saturating_sub(1)
        .min(self.pages.len().saturating_sub(1)),
    )
  }

  pub fn project(&self, page: PageId, bounds: Bounds, scroll: f32) -> Bounds {
    let layout = &self.pages[page.0];
    let (x, y) = self.origin(page, scroll);
    let mut rect = transform(bounds, &layout.matrix);
    rect.x0 += x;
    rect.x1 += x;
    rect.y0 += y;
    rect.y1 += y;
    rect
  }

  pub fn unproject(
    &self,
    page: PageId,
    (x, y): (f32, f32),
    scroll: f32,
  ) -> Option<(f32, f32)> {
    let (left, top) = self.origin(page, scroll);
    self.pages[page.0]
      .matrix
      .invert()
      .map(|m| m.transform_xy(x - left, y - top))
  }

  pub fn pan_limit(&self, page: PageId) -> f32 {
    let max = self.pages[page.0].width - self.screen.width();
    if max <= 1.0 { 0.0 } else { max }
  }

  /// Pan the page shows: none when it fits.
  pub fn shown_pan(&self, page: PageId) -> f32 {
    (-self.origin(page, 0.0).0).max(0.0)
  }

  /// Raster origin within the page pane.
  pub fn origin(&self, page: PageId, scroll: f32) -> (f32, f32) {
    let layout = &self.pages[page.0];
    let spare = self.screen.width() - layout.width;
    let x = if spare >= 0.0 {
      spare / 2.0
    } else {
      -self.pan.clamp(0.0, self.pan_limit(page))
    };
    (x, layout.top - scroll)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn projection_roundtrips() {
    let pages = [Bounds { x0: 10.0, y0: 20.0, x1: 210.0, y1: 320.0 }];
    let screen = Screen {
      columns: 100,
      rows: 30,
      cell_width: 8,
      cell_height: 16,
      origin: (15, 3),
    };
    let mut layout = Layout::new(pages, screen);
    let cases = [0, 90, 180, 270].into_iter().flat_map(|rotation| {
      [0.5, 1.0, 2.0].into_iter().flat_map(move |zoom| {
        [-100.0, 0.0, 180.0, 10_000.0].map(|pan| (rotation, zoom, pan))
      })
    });
    for (rotation, zoom, pan) in cases {
      layout.rotation = rotation;
      layout.zoom = zoom;
      layout.pan = pan;
      layout.crop = Crop([50, 100, 150, 50]);
      std::sync::Arc::make_mut(&mut layout.crops)
        .insert(PageId(0), layout.crop);
      layout.reflow(pages);
      let spare = screen.width() - layout.pages[0].width;
      let (left, top) = layout.origin(PageId(0), 27.0);
      assert!((top + 27.0).abs() < 0.001);
      if spare >= 0.0 {
        assert!((left - spare / 2.0).abs() < 0.001);
      } else if spare >= -1.0 {
        assert!(left.abs() < 0.001, "fit-width rounding cannot pan");
      } else {
        assert!((left + pan.clamp(0.0, -spare)).abs() < 0.001);
      }
      for (x, y) in [(30.0, 60.0), (100.0, 150.0), (170.0, 280.0)] {
        let rect = layout.project(
          PageId(0),
          Bounds { x0: x, y0: y, x1: x, y1: y },
          27.0,
        );
        let (px, py) =
          layout.unproject(PageId(0), (rect.x0, rect.y0), 27.0).unwrap();
        assert!((px - x).abs() < 0.001 && (py - y).abs() < 0.001);
      }
    }
  }
}
