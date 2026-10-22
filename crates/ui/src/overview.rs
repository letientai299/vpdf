use crate::keys::{Action, Input};
use crate::layout::tiles::Visible;
use crate::layout::{Crop, Screen, page_matrix, screen_step, transform};
use crate::pointer::Wheel;
use crate::source::Pages;
use crate::text::{Bounds, PageId};
use crate::theme;
#[cfg(test)]
use anyhow::Result;
use unicode_width::UnicodeWidthStr;
use vpdf_model::input::{MouseButton, MouseEventKind};
use vpdf_model::scene::{Draw, Layer, Op, Stage, Surface};
use vpdf_model::style::Role;
use vpdf_model::tile::{TileId, TileKey, TileOwner, cover};
use vpdf_reader::motion::Motion;

/// Thumbnails per row aim for this width in pixels.
const TARGET: f32 = 220.0;

/// Page thumbnail grid opened by `gp`.
pub struct Overview {
  selected: usize,
  /// First visible grid row.
  top: usize,
  /// Grid columns and rows of the last frame; a new shape reveals the
  /// selection.
  shape: (usize, usize),
}

/// Grid geometry for the current screen, in page-area pixels.
pub struct Grid {
  columns: usize,
  /// Thumbnail box; pages fit inside it.
  width: f32,
  height: f32,
  gap: f32,
  /// Box plus its label row and gap.
  row: f32,
  rows: usize,
}

impl Grid {
  pub fn shape(&self) -> (usize, usize) {
    (self.columns, self.rows)
  }
  pub fn row_top(&self, row: usize) -> f32 {
    self.gap / 2.0 + row as f32 * self.row
  }

  fn visible_rows(&self, height: f32) -> usize {
    ((height - self.gap / 2.0) / self.row).ceil().max(0.0) as usize
  }

  /// The top row that shows the last of `pages`.
  fn last_top(&self, pages: usize) -> usize {
    pages.div_ceil(self.columns).saturating_sub(self.rows)
  }
}

impl Overview {
  /// First visible grid row.
  pub fn top(&self) -> usize {
    self.top
  }

  /// First grid row the next frame shows: a new grid shape reveals
  /// the selection.
  fn shown_top(&self, grid: &Grid, pages: usize) -> usize {
    let mut next = Self { ..*self };
    if self.shape != (grid.columns, grid.rows) {
      next.reveal(grid);
    }
    next.top.min(grid.last_top(pages))
  }

  /// Scrolls the fewest rows that show the selection.
  fn reveal(&mut self, grid: &Grid) {
    let row = self.selected / grid.columns;
    if row < self.top {
      self.top = row;
    } else if row >= self.top + grid.rows {
      self.top = row + 1 - grid.rows;
    }
  }
}

impl Grid {
  pub fn new(screen: Screen, first: Bounds) -> Self {
    let gap = f32::from(screen.cell_width) * 2.0;
    let label = f32::from(screen.cell_height);
    let aspect = first.height() / first.width().max(1.0);
    let fit = |columns: f32| (screen.width() - gap * (columns + 1.0)) / columns;
    // At least two rows stay visible, so the grid reads as a grid.
    let tallest = ((screen.height() - gap) / 2.0 - label - gap) / aspect;
    let mut width = fit((screen.width() / TARGET).clamp(2.0, 10.0).floor());
    width = width.min(tallest).max(f32::from(screen.cell_width) * 4.0);
    let columns =
      (((screen.width() - gap) / (width + gap)) as usize).clamp(2, 12);
    let width = width.min(fit(columns as f32));
    let height = width * aspect;
    let row = height + label + gap;
    let rows = ((screen.height() - gap) / row).floor().max(1.0) as usize;
    Self { columns, width, height, gap, row, rows }
  }
}

enum Route {
  Horizontal(bool),
  Vertical(bool),
  Edge(bool),
  Scroll(f32),
  Screen(i32),
  Open,
  Close,
  Pass,
  Ignore,
}

fn route(action: Action) -> Route {
  match action {
    Action::Motion(Motion::Left) => Route::Horizontal(false),
    Action::Motion(Motion::Right) => Route::Horizontal(true),
    Action::Motion(Motion::Up) => Route::Vertical(false),
    Action::Motion(Motion::Down) => Route::Vertical(true),
    Action::Motion(Motion::First) => Route::Edge(false),
    Action::Motion(Motion::Last) => Route::Edge(true),
    Action::Scroll(amount) => Route::Scroll(amount),
    Action::Screen(direction) => Route::Screen(direction),
    Action::Follow | Action::Swap => Route::Open,
    Action::Quit | Action::Cancel | Action::Overview => Route::Close,
    Action::History(_)
    | Action::Command
    | Action::Window(_)
    | Action::Tab(_)
    | Action::Panel(_)
    | Action::ClosePanel
    | Action::LocalFocus
    | Action::YankPath => Route::Pass,
    _ => Route::Ignore,
  }
}

pub enum Navigation {
  Handled,
  Open,
  Close,
  Pass,
}
pub enum Pointer {
  None,
  Open,
  Split(PageId),
  Context(Option<PageId>),
}

#[derive(Clone, Copy)]
pub struct Frame {
  pub pane: u32,
  pub screen: Screen,
  pub clip: Screen,
  pub at: vpdf_model::geometry::At,
  pub left: u16,
  pub hovered: Option<PageId>,
  pub mask: Option<Bounds>,
}

impl Overview {
  pub fn new(selected: PageId) -> Self {
    Self { selected: selected.0, top: 0, shape: (0, 0) }
  }
  pub fn selected(&self) -> usize {
    self.selected
  }
  pub fn choose(&mut self, page: PageId) {
    self.selected = page.0;
  }
  pub fn allows(action: Action) -> bool {
    !matches!(route(action), Route::Ignore)
  }

  pub fn navigate(
    &mut self,
    input: Input,
    grid: &Grid,
    pages: usize,
  ) -> Navigation {
    let last = pages - 1;
    let count = input.count;
    let page = grid.columns * grid.rows;
    let selected = self.selected;
    self.selected = match route(input.action) {
      Route::Horizontal(false) => selected.saturating_sub(count),
      Route::Horizontal(true) => selected + count,
      Route::Vertical(false) => selected.saturating_sub(count * grid.columns),
      Route::Vertical(true) => selected + count * grid.columns,
      Route::Edge(_) if input.explicit => count.saturating_sub(1),
      Route::Edge(false) => 0,
      Route::Edge(true) => last,
      Route::Scroll(amount) => {
        let step = (amount * page as f32) as isize * count as isize;
        selected.saturating_add_signed(step)
      }
      Route::Screen(direction) => {
        let step = screen_step(grid.rows) * grid.columns * count;
        selected.saturating_add_signed(direction as isize * step as isize)
      }
      Route::Open => return Navigation::Open,
      Route::Close => return Navigation::Close,
      Route::Pass => return Navigation::Pass,
      Route::Ignore => return Navigation::Handled,
    }
    .min(last);
    self.reveal(grid);
    Navigation::Handled
  }

  pub fn pointer(
    &mut self,
    kind: MouseEventKind,
    hit: Option<PageId>,
    grid: &Grid,
    pages: usize,
  ) -> Pointer {
    match kind {
      // The wheel moves rows, not the selection.
      kind if Wheel::of(kind) == Some(Wheel::Down) => {
        self.top = (self.top + 1).min(grid.last_top(pages));
      }
      kind if Wheel::of(kind) == Some(Wheel::Up) => {
        self.top = self.top.saturating_sub(1);
      }
      MouseEventKind::Down(MouseButton::Left) => {
        if let Some(page) = hit {
          self.selected = page.0;
          return Pointer::Open;
        }
      }
      MouseEventKind::Down(MouseButton::Middle) => {
        if let Some(page) = hit {
          return Pointer::Split(page);
        }
      }
      MouseEventKind::Down(MouseButton::Right) => {
        return Pointer::Context(hit);
      }
      _ => {}
    }
    Pointer::None
  }

  pub fn hit(
    &self,
    (x, y): (f32, f32),
    grid: &Grid,
    pages: usize,
    screen: Screen,
  ) -> Option<PageId> {
    if x < 0.0 || y < 0.0 || x >= screen.width() || y >= screen.height() {
      return None;
    }
    let column = ((x - grid.gap / 2.0) / (grid.width + grid.gap)) as usize;
    let row = ((y - grid.gap / 2.0) / grid.row) as usize;
    // As the next frame scrolls it.
    let index = (self.shown_top(grid, pages) + row) * grid.columns + column;
    let inside =
      column < grid.columns && row < grid.visible_rows(screen.height());
    (inside && index < pages).then_some(PageId(index))
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    source: &impl Pages,
    frame: Frame,
    scene: &mut S,
  ) -> Result<Visible, S::Error> {
    let grid = Grid::new(frame.screen, source.bounds(PageId(0)));
    let pages = source.count();
    let stage = Stage { screen: frame.clip, left: frame.left };
    let screen = frame.clip;
    let hovered = frame.hovered.map(|page| page.0);
    self.top = self.shown_top(&grid, pages);
    self.shape = (grid.columns, grid.rows);
    let (top, selected) = (self.top, self.selected);
    let first = top * grid.columns;
    let visible = first
      ..(first + grid.visible_rows(screen.height()) * grid.columns).min(pages);
    let mut wanted = Vec::new();
    let mut missing = Vec::new();
    for index in visible {
      let cell = index - first;
      let x = grid.gap + (cell % grid.columns) as f32 * (grid.width + grid.gap);
      let y = grid.row_top(cell / grid.columns);
      let bounds = source.bounds(PageId(index));
      let fit =
        (grid.width / bounds.width()).min(grid.height / bounds.height());
      let scale = (fit * 1024.0).round() as u32;
      // The worker draws at the rounded scale, which can differ by a pixel.
      let size = transform(bounds, &page_matrix(bounds, scale, 0));
      let (w, h) = (size.x1.ceil() as i32, size.y1.ceil() as i32);
      let (dx, dy) =
        ((grid.width - w as f32) / 2.0, (grid.height - h as f32) / 2.0);
      let whole = TileKey {
        page: PageId(index),
        scale,
        rotation: 0,
        x: 0,
        y: 0,
        width: w,
        height: h,
        crop: Crop::default(),
      };
      // Keep tile keys reusable after scrolling.
      let bottom = (screen.height() - (y + dy).floor()).ceil() as i32;
      for key in cover(whole, 0..w, 0..bottom.clamp(0, h)) {
        let origin = (x + dx + key.x as f32, y + dy + key.y as f32);
        let id = TileId { owner: TileOwner::new(frame.pane), key };
        if !scene.tile(id, origin, stage)? {
          missing.push(key);
        }
        wanted.push(key);
      }
      frame.label(
        scene,
        || source.label(PageId(index)),
        index == selected,
        (x, y + grid.height),
        grid.width,
      );
      let ring = Bounds {
        x0: x + dx - 3.0,
        y0: y + dy - 3.0,
        x1: x + dx + w as f32 + 3.0,
        y1: y + dy + h as f32 + 3.0,
      };
      let tint = if index == selected {
        Some(theme::RING)
      } else {
        (hovered == Some(index)).then_some(theme::RING_HOVER)
      };
      if let Some(tint) = tint {
        for piece in crate::paint::outside(ring, frame.mask) {
          scene.fill(piece, tint, Layer::AboveText, stage);
        }
      }
    }
    Ok(Visible { wanted, missing })
  }
}
impl Frame {
  fn label<'a, 's>(
    &self,
    scene: &mut impl Draw<'s>,
    label: impl FnOnce() -> &'a str,
    selected: bool,
    (x, y): (f32, f32),
    width: f32,
  ) {
    let screen = self.screen;
    let row = (y / f32::from(screen.cell_height)).ceil() as usize;
    if row >= usize::from(self.clip.rows.saturating_sub(1)) {
      return;
    }
    let columns = (width / f32::from(screen.cell_width)) as usize;
    let label = vpdf_model::text::clean(label(), columns);
    let column = (x / f32::from(screen.cell_width)) as usize
      + (columns.saturating_sub(label.width())) / 2;
    let style =
      if selected { Role::Badge(theme::Color::Blue) } else { Role::Muted };
    crate::paint::move_to(scene, self.at, row, usize::from(self.left) + column);
    scene.push(Op::Style(style));
    scene.text(&label);
    scene.push(Op::Style(Role::Reset));
  }
}

#[cfg(test)]
#[path = "../../../tests/support/overview.rs"]
mod fixture;

#[cfg(test)]
mod tests {
  use super::fixture::Source;
  use super::*;
  #[test]
  fn visible_rows_include_partial() {
    let grid = Grid {
      columns: 5,
      width: 220.0,
      height: 300.0,
      gap: 16.0,
      row: 332.0,
      rows: 3,
    };
    for (height, rows) in
      [(0.0, 0), (8.0, 0), (9.0, 1), (1004.0, 3), (1005.0, 4), (1200.0, 4)]
    {
      assert_eq!(grid.visible_rows(height), rows, "{height}");
    }
  }

  #[test]
  fn recorded_overview_keeps_tiles_and_chrome() {
    use vpdf_model::scene::Recorder;
    let frame = fixture::frame();
    let mut overview = Overview::new(PageId(1));
    let mut recorder = Recorder::new(|_| false);
    let visible = overview.scene(&Source, frame, &mut recorder).unwrap();
    assert_eq!(visible.wanted, visible.missing);
    assert_ne!(visible.wanted, [] as [TileKey; 0]);
    let scene = recorder.into_scene();
    assert!(scene.iter().any(|op| matches!(op, Op::Tile(_))));
    assert!(scene.iter().any(|op| matches!(op, Op::Text(_))));
    assert!(scene.iter().any(|op| matches!(op, Op::Fill(_))));
  }

  fn label_frame() -> Frame {
    let screen = crate::testing::screen::cells(100, 40);
    Frame {
      pane: 1,
      screen,
      clip: screen,
      at: vpdf_model::geometry::At::default(),
      left: 0,
      hovered: None,
      mask: None,
    }
  }

  #[test]
  fn labels_center_by_cells() {
    let frame = label_frame();
    for (label, column) in [("IV", 6), ("章", 6), ("e\u{301}", 6), ("界界", 5)]
    {
      let mut scene = vpdf_model::scene::Scene::new();
      frame.label(&mut scene, || label, false, (16.0, 16.0), 80.0);
      assert_eq!(scene[0], Op::Move { row: 1, column }, "{label}");
    }
  }

  #[test]
  fn hidden_labels_keep_cursor() {
    let frame = label_frame();
    let mut out = Vec::new();
    frame.label(
      &mut out,
      || panic!("Hidden label read"),
      false,
      (16.0, frame.screen.height()),
      220.0,
    );
    assert_eq!(out, []);
  }
}
