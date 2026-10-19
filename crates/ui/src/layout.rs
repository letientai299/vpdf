pub mod tiles;
use crate::text::{Bounds, PageId};
pub use vpdf_model::geometry::{
  Crop, Crops, Screen, Transform, clip, page_matrix, screen_step, transform,
};
use vpdf_model::tile::{TILE_HEIGHT, TILE_WIDTH, TileKey, cover};

#[derive(Clone, Debug)]
pub struct PageLayout {
  pub top: f32,
  pub width: f32,
  pub height: f32,
  pub scale: u32,
  pub matrix: Transform,
}

/// View options and the last reflowed projection.
pub struct Layout {
  pages: Vec<PageLayout>,
  total: f32,
  screen: Screen,
  zoom: f32,
  rotation: u16,
  pan: f32,
  crop: Crop,
  crops: std::sync::Arc<Crops>,
}

#[derive(Clone, Copy)]
pub enum Zoom {
  To(f32),
  Steps(f32),
  Width,
  Page(PageId),
}

/// Reflow identity for retaining an anchor.
#[derive(Debug, PartialEq)]
pub struct Checkpoint {
  page: PageId,
  size: (u16, u16, u16, u16),
  matrix: Transform,
  top: u32,
}

#[must_use]
pub struct Fit {
  page: PageId,
  bounds: Bounds,
  screen: Screen,
}

impl Fit {
  pub fn finish(self, layout: &mut Layout) -> f32 {
    let rect = layout.project(self.page, self.bounds, 0.0);
    let scroll = layout
      .clamp_scroll(rect.y0 - (self.screen.height() - rect.height()) / 2.0);
    layout.pan = (layout.pan + rect.x0
      - (self.screen.width() - rect.width()) / 2.0)
      .clamp(0.0, layout.pan_limit(self.page));
    scroll
  }
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

  pub fn pages(&self) -> &[PageLayout] {
    &self.pages
  }

  pub fn total(&self) -> f32 {
    self.total
  }

  pub fn screen(&self) -> Screen {
    self.screen
  }

  pub fn zoom(&self) -> f32 {
    self.zoom
  }

  pub fn rotation(&self) -> u16 {
    self.rotation
  }

  pub fn pan(&self) -> f32 {
    self.pan
  }

  pub fn crop(&self) -> Crop {
    self.crop
  }

  pub fn crops(&self) -> &Crops {
    &self.crops
  }

  /// Defers projection changes until reflow.
  pub fn request_zoom(&mut self, zoom: Zoom) {
    match zoom {
      Zoom::To(value) => self.zoom = value,
      Zoom::Steps(steps) => {
        self.zoom = (self.zoom * 1.15_f32.powf(steps)).clamp(0.1, 8.0);
      }
      Zoom::Width => self.zoom = 1.0,
      Zoom::Page(page) => {
        let page = &self.pages[page.0];
        self.zoom *= (self.screen.height() / page.height)
          .min(self.screen.width() / page.width);
      }
    }
  }

  /// Defers rotation until reflow.
  pub fn rotate(&mut self) {
    self.rotation = (self.rotation + 90) % 360;
  }

  /// Restores saved options before reflow.
  pub fn restore(&mut self, zoom: f32, rotation: u16, crop: Crop) {
    self.zoom = zoom.clamp(0.1, 8.0);
    self.rotation = rotation % 360 / 90 * 90;
    self.crop = crop;
  }

  /// Copies view options before reflow.
  pub fn copy_view(&mut self, other: &Self) {
    self.zoom = other.zoom;
    self.rotation = other.rotation;
    self.crop = other.crop;
    self.crops = std::sync::Arc::clone(&other.crops);
    self.pan = other.pan;
  }

  /// Requests cropping without rebuilding geometry.
  pub fn request_crop(&mut self) {
    self.crop = Crop([1, 0, 0, 0]);
  }

  /// Applies resolved margins at the next reflow.
  pub fn resolve_crops(&mut self, crops: std::sync::Arc<Crops>) {
    self.request_crop();
    self.crops = crops;
  }

  pub fn show_margins(&mut self) {
    self.crop = Crop::default();
  }

  /// Discards document-specific crops on reload.
  pub fn clear_crops(&mut self) {
    self.crops = std::sync::Arc::default();
  }

  /// Stores pan; projection clips each page.
  pub fn pan_to(&mut self, pan: f32) {
    self.pan = pan;
  }

  pub fn pan_by(&mut self, page: PageId, delta: f32) {
    self.pan = (self.pan + delta).clamp(0.0, self.pan_limit(page));
  }

  pub fn clamp_pan(&mut self, page: PageId) {
    self.pan = self.pan.clamp(0.0, self.pan_limit(page));
  }

  pub fn relocate(&mut self, origin: (u16, u16)) {
    self.screen.origin = origin;
  }

  pub fn resize(
    &mut self,
    screen: Screen,
    pages: impl IntoIterator<Item = Bounds>,
  ) {
    self.screen = screen;
    self.reflow(pages);
  }

  pub fn checkpoint(&self, page: PageId) -> Checkpoint {
    let screen = self.screen;
    let projected = &self.pages[page.0];
    Checkpoint {
      page,
      size: (
        screen.columns,
        screen.rows,
        screen.cell_width,
        screen.cell_height,
      ),
      matrix: projected.matrix,
      top: projected.top.to_bits(),
    }
  }

  /// Places a page point at viewport pixels.
  pub fn restore_at(
    &mut self,
    page: PageId,
    (x, y): (f32, f32),
    (left, top): (f32, f32),
  ) -> f32 {
    let rect = self.project(page, Bounds { x0: x, x1: x, y0: y, y1: y }, 0.0);
    let scroll = self.clamp_scroll(rect.y0 - top);
    let shown = self.shown_pan(page);
    let max = self.pan_limit(page);
    self.pan = (shown + rect.x0 - left).clamp(0.0, max);
    scroll
  }

  /// Reveals a rectangle projected at scroll zero.
  pub fn reveal(
    &mut self,
    page: PageId,
    rect: Bounds,
    mut scroll: f32,
    margin: f32,
  ) -> f32 {
    if rect.y0 < scroll + margin {
      scroll = rect.y0 - margin;
    }
    if rect.y1 > scroll + self.screen.height() - margin {
      scroll = rect.y1 - self.screen.height() + margin;
    }
    let pan = self.shown_pan(page);
    if rect.x0 < 0.0 {
      self.pan = (pan + rect.x0).max(0.0);
    }
    if rect.x1 > self.screen.width() {
      self.pan = pan + rect.x1 - self.screen.width();
    }
    self.clamp_scroll(scroll)
  }

  /// Requests fit; finish after reflow.
  pub fn fit_region(&mut self, page: PageId, bounds: Bounds) -> Fit {
    let rect = self.project(page, bounds, 0.0);
    let screen = self.screen;
    let margin = f32::from(screen.cell_height);
    self.zoom *= ((screen.width() - margin).max(1.0) / rect.width())
      .min((screen.height() - margin * 2.0).max(1.0) / rect.height());
    self.zoom = self.zoom.clamp(0.1, 8.0);
    Fit { page, bounds, screen }
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

  pub fn point_at(
    &self,
    (x, y): (f32, f32),
    scroll: f32,
  ) -> Option<(PageId, f32, f32)> {
    let page = self.page_at(y + scroll);
    let (x, y) = self.unproject(page, (x, y), scroll)?;
    Some((page, x, y))
  }

  pub fn anchor_at(&self, at: (f32, f32), scroll: f32) -> (PageId, (f32, f32)) {
    self.point_at(at, scroll).map_or_else(
      || (self.page_at(at.1 + scroll), (0.0, 0.0)),
      |(page, x, y)| (page, (x, y)),
    )
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

pub fn visible(layout: &Layout, scroll: f32) -> Vec<TileKey> {
  let mut tiles = Vec::new();
  for page in layout.visible(scroll) {
    let data = &layout.pages[page];
    let (x, y) = layout.origin(PageId(page), scroll);
    let left = ((-x).max(0.0) as i32 / TILE_WIDTH) * TILE_WIDTH;
    let top = ((-y).max(0.0) as i32 / TILE_HEIGHT) * TILE_HEIGHT;
    let right = (layout.screen.width() - x).min(data.width) as i32;
    let bottom =
      (layout.screen.height() - y + TILE_HEIGHT as f32).min(data.height) as i32;
    let whole = TileKey {
      page: PageId(page),
      scale: data.scale,
      rotation: layout.rotation,
      x: 0,
      y: 0,
      width: data.width as i32,
      height: data.height as i32,
      crop: layout.crop_for(PageId(page)),
    };
    tiles.extend(cover(whole, left..right, top..bottom));
  }
  tiles
}

#[cfg(test)]
mod tests {
  use super::*;

  fn screen() -> Screen {
    Screen {
      columns: 100,
      rows: 30,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    }
  }

  const PAPER: Bounds = Bounds { x0: 0.0, y0: 0.0, x1: 400.0, y1: 600.0 };

  #[test]
  fn requests_preserve_old_projection() {
    let mut layout = Layout::new([PAPER], screen());
    let before = layout.checkpoint(PageId(0));
    let rect = layout.project(PageId(0), PAPER, 0.0);
    layout.request_zoom(Zoom::Steps(2.0));
    layout.rotate();
    layout.request_crop();
    assert!(!layout.crop().is_none());
    assert_eq!(layout.rotation(), 90);
    assert_eq!(layout.checkpoint(PageId(0)), before);
    assert_eq!(layout.project(PageId(0), PAPER, 0.0), rect);
    let mut crops = Crops::default();
    crops.insert(PageId(0), Crop([100, 100, 100, 100]));
    layout.resolve_crops(std::sync::Arc::new(crops));
    assert_eq!(layout.checkpoint(PageId(0)), before);
    layout.reflow([PAPER]);
    assert_ne!(layout.checkpoint(PageId(0)), before);
    assert_ne!(layout.project(PageId(0), PAPER, 0.0), rect);
    let cropped = layout.checkpoint(PageId(0));
    layout.show_margins();
    assert_eq!(layout.checkpoint(PageId(0)), cropped);
    assert!(layout.crop_for(PageId(0)).is_none());
    assert_eq!(layout.crops().len(), 1);
    layout.reflow([PAPER]);
    assert_ne!(layout.checkpoint(PageId(0)), cropped);
  }

  #[test]
  fn copies_keep_local_screen() {
    let mut source = Layout::new([PAPER], screen());
    let mut crops = Crops::default();
    crops.insert(PageId(0), Crop([100, 0, 0, 0]));
    source.resolve_crops(std::sync::Arc::new(crops));
    source.request_zoom(Zoom::To(2.0));
    source.rotate();
    source.pan_to(100.0);
    let smaller = Screen { columns: 40, ..screen() };
    let mut layout = Layout::new([PAPER], smaller);
    let before = layout.checkpoint(PageId(0));
    layout.copy_view(&source);
    assert_eq!(layout.screen(), smaller);
    assert_eq!(layout.checkpoint(PageId(0)), before);
    assert_eq!(
      (layout.zoom(), layout.rotation(), layout.pan()),
      (2.0, 90, 100.0)
    );
    assert!(std::ptr::eq(layout.crops(), source.crops()));
    layout.reflow([PAPER]);
    assert_ne!(layout.checkpoint(PageId(0)), before);
    layout.clear_crops();
    assert!(!layout.crop().is_none());
    assert!(layout.crops().is_empty());
    assert_eq!(source.crops().len(), 1);
    layout.restore(99.0, 451, Crop::default());
    assert_eq!((layout.zoom(), layout.rotation()), (8.0, 90));
  }

  #[test]
  fn anchors_survive_reflow() {
    let mut layout = Layout::new([PAPER], screen());
    let before = layout.checkpoint(PageId(0));
    layout.relocate((12, 6));
    assert_eq!(layout.checkpoint(PageId(0)), before);
    let point = (180.0, 210.0);
    let at = (250.0, 100.0);
    layout.request_zoom(Zoom::To(2.0));
    layout.rotate();
    layout.reflow([PAPER]);
    let scroll = layout.restore_at(PageId(0), point, at);
    let restored = layout.unproject(PageId(0), at, scroll).unwrap();
    assert!((restored.0 - point.0).abs() < 0.001);
    assert!((restored.1 - point.1).abs() < 0.001);
    layout.pan_by(PageId(0), f32::MAX);
    assert_eq!(layout.pan(), layout.pan_limit(PageId(0)));
    layout.pan_by(PageId(0), -f32::MAX);
    assert_eq!(layout.pan(), 0.0);
  }

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
      let crop = Crop([50, 100, 150, 50]);
      let mut crops = Crops::default();
      crops.insert(PageId(0), crop);
      layout.resolve_crops(std::sync::Arc::new(crops));
      layout.restore(zoom, rotation, crop);
      layout.pan_to(pan);
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
