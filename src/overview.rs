use crate::Result;
use crate::app::App;
use crate::graphics::Graphics;
use crate::keys::{Action, Input};
use crate::layout::{Crop, page_matrix, screen_step, transform};
use crate::pointer::{Hit, Wheel};
use crate::text::{Bounds, PageId};
use crate::tiles::{self, TileKey};
use crate::{terminal, theme};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::io::Write;
use vpdf_reader::motion::Motion;

/// Thumbnails per row aim for this width in pixels.
const TARGET: f32 = 220.0;

/// Page thumbnail grid opened by `gp`.
pub struct Overview {
  pub selected: usize,
  /// First visible grid row.
  top: usize,
  /// Grid columns and rows of the last frame; a new shape reveals the
  /// selection.
  shape: (usize, usize),
}

/// Grid geometry for the current screen, in page-area pixels.
struct Grid {
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

impl App {
  fn grid(&self) -> Grid {
    let screen = self.layout.screen;
    let gap = f32::from(screen.cell_width) * 2.0;
    let label = f32::from(screen.cell_height);
    let first = self.doc.pages[0].bounds;
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
    Grid { columns, width, height, gap, row, rows }
  }

  pub fn open_overview(&mut self) -> Result<()> {
    anyhow::ensure!(!self.text_mode, "Page overview needs Kitty graphics");
    let selected = self.navigation_position().page.0;
    self.overview = Some(Overview { selected, top: 0, shape: (0, 0) });
    self.requested.clear();
    if let Some(panel) = &mut self.panel {
      panel.focused = false;
    }
    self.relayout()
  }

  /// List motions move between thumbnails; Enter opens the page.
  pub fn overview_input(&mut self, input: Input) -> Result<bool> {
    let grid = self.grid();
    let last = self.doc.pages.len() - 1;
    let Some(overview) = &mut self.overview else { return Ok(false) };
    let count = input.count;
    let page = grid.columns * grid.rows;
    let selected = overview.selected;
    overview.selected = match input.action {
      Action::Motion(Motion::Left) => selected.saturating_sub(count),
      Action::Motion(Motion::Right) => selected + count,
      Action::Motion(Motion::Up) => {
        selected.saturating_sub(count * grid.columns)
      }
      Action::Motion(Motion::Down) => selected + count * grid.columns,
      Action::Motion(Motion::First | Motion::Last) if input.explicit => {
        count.saturating_sub(1)
      }
      Action::Motion(Motion::First) => 0,
      Action::Motion(Motion::Last) => last,
      Action::Scroll(amount) => {
        let step = (amount * page as f32) as isize * count as isize;
        selected.saturating_add_signed(step)
      }
      Action::Screen(direction) => {
        let step = screen_step(grid.rows) * grid.columns * count;
        selected.saturating_add_signed(direction as isize * step as isize)
      }
      Action::Follow | Action::Swap => {
        return self.open_selected().map(|()| true);
      }
      Action::Quit | Action::Cancel | Action::Overview => {
        self.close_overview();
        return Ok(true);
      }
      Action::History(_) => return Ok(false),
      // `:`, panes, tabs, and history pass through.
      _ => {
        return Ok(!matches!(
          input.action,
          Action::Command | Action::Window(_) | Action::Tab(_)
        ));
      }
    }
    .min(last);
    overview.reveal(&grid);
    Ok(true)
  }

  pub fn overview_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
    let grid = self.grid();
    let pages = self.doc.pages.len();
    let hit = self.thumbnail_at(mouse.column, mouse.row);
    let Some(overview) = &mut self.overview else { return Ok(()) };
    match mouse.kind {
      // The wheel moves rows, not the selection.
      kind if Wheel::of(kind) == Some(Wheel::Down) => {
        overview.top = (overview.top + 1).min(grid.last_top(pages));
      }
      kind if Wheel::of(kind) == Some(Wheel::Up) => {
        overview.top = overview.top.saturating_sub(1);
      }
      MouseEventKind::Down(MouseButton::Left) => {
        if let Some(page) = hit {
          overview.selected = page.0;
          return self.open_selected();
        }
      }
      MouseEventKind::Down(MouseButton::Middle) => {
        if let Some(page) = hit {
          return self.open_split(&crate::document::Target::Page(page));
        }
      }
      MouseEventKind::Down(MouseButton::Right) => {
        self.overview_context(mouse, hit);
      }
      _ => {}
    }
    Ok(())
  }

  /// The thumbnail drawn at a pane cell.
  pub fn thumbnail_at(&self, column: u16, row: u16) -> Option<PageId> {
    let overview = self.overview.as_ref()?;
    let grid = self.grid();
    let (x, y) = self.cell_point(column, row, None);
    let screen = self.uncovered();
    if x < 0.0 || y < 0.0 || x >= screen.width() || y >= screen.height() {
      return None;
    }
    let column = ((x - grid.gap / 2.0) / (grid.width + grid.gap)) as usize;
    let row = ((y - grid.gap / 2.0) / grid.row) as usize;
    // As the next frame scrolls it.
    let pages = self.doc.pages.len();
    let index =
      (overview.shown_top(&grid, pages) + row) * grid.columns + column;
    let inside =
      column < grid.columns && row < grid.visible_rows(screen.height());
    (inside && index < pages).then_some(PageId(index))
  }

  fn open_selected(&mut self) -> Result<()> {
    let Some(overview) = self.overview.take() else { return Ok(()) };
    let page = PageId(overview.selected);
    self.requested.clear();
    self.go_to_page(page)
  }

  pub fn close_overview(&mut self) {
    self.overview = None;
    self.requested.clear();
  }

  /// Draws visible thumbnails and requests the missing ones.
  pub fn overview_canvas(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let grid = self.grid();
    let pages = self.doc.pages.len();
    let (screen, left) = (self.uncovered(), self.left);
    let hovered = match self.hover {
      Hit::Thumbnail(page) => Some(page.0),
      _ => None,
    };
    let Some(overview) = &mut self.overview else { return Ok(()) };
    overview.top = overview.shown_top(&grid, pages);
    overview.shape = (grid.columns, grid.rows);
    let (top, selected) = (overview.top, overview.selected);
    let first = top * grid.columns;
    let visible = first
      ..(first + grid.visible_rows(screen.height()) * grid.columns).min(pages);
    let mut wanted = Vec::new();
    let mut missing = Vec::new();
    for index in visible {
      let cell = index - first;
      let x = grid.gap + (cell % grid.columns) as f32 * (grid.width + grid.gap);
      let y = grid.gap / 2.0 + (cell / grid.columns) as f32 * grid.row;
      let bounds = self.doc.pages[index].bounds;
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
      for key in tiles::cover(whole, 0..w, 0..bottom.clamp(0, h)) {
        let origin = (x + dx + key.x as f32, y + dy + key.y as f32);
        let pane = (self.id, key);
        if !graphics.place_at(pane, origin, screen, left, out)? {
          missing.push(key);
        }
        wanted.push(key);
      }
      self.thumbnail_label(out, index, (x, y + grid.height), grid.width)?;
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
        self.tint(graphics, ring, tint, screen, out)?;
      }
    }
    self.complete = missing.is_empty();
    if wanted != self.requested {
      self.worker.request(&missing, missing.len());
      self.requested = wanted;
    }
    Ok(())
  }

  fn thumbnail_label(
    &self,
    out: &mut impl Write,
    index: usize,
    (x, y): (f32, f32),
    width: f32,
  ) -> Result<()> {
    let screen = self.layout.screen;
    let row = (y / f32::from(screen.cell_height)).ceil() as usize;
    if row >= usize::from(self.uncovered().rows.saturating_sub(1)) {
      return Ok(());
    }
    let columns = (width / f32::from(screen.cell_width)) as usize;
    let label = terminal::clean(self.doc.label(PageId(index)), columns);
    let column = (x / f32::from(screen.cell_width)) as usize
      + (columns.saturating_sub(label.len())) / 2;
    let style = if self.overview.as_ref().is_some_and(|o| o.selected == index) {
      theme::badge(theme::Color::Blue)
    } else {
      theme::MUTED.to_owned()
    };
    write!(
      out,
      "{}{style}{label}{}",
      self.at.goto(row, usize::from(self.left) + column),
      theme::RESET
    )?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::layout::Screen;
  use std::time::{Duration, Instant};

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
  #[ignore = "requires verified public PDF corpus"]
  fn partial_thumbnails_reuse_tiles() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 156,
      rows: 76,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut app =
      crate::solo::Solo::open(&root.join("book.pdf"), "", false, screen)?;
    app.open_overview()?;
    let grid = app.grid();
    let partial = grid.rows * grid.columns;
    let y = grid.gap / 2.0 + grid.rows as f32 * grid.row;
    assert!(y < screen.height() && screen.height() - y < 256.0);
    let mut out = Vec::new();
    app.draw(&mut out)?;
    let wanted = app.requested.clone();
    for page in partial..partial + grid.columns {
      let keys: Vec<_> = wanted.iter().filter(|k| k.page.0 == page).collect();
      assert_eq!(keys.len(), 1, "only the visible tile of page {page}");
      assert_eq!(keys[0].y, 0);
    }
    assert!(wanted.iter().all(|k| k.page.0 < partial + grid.columns));
    let mut label = Vec::new();
    app.thumbnail_label(
      &mut label,
      partial,
      (grid.gap, y + grid.height),
      grid.width,
    )?;
    assert!(label.is_empty(), "hidden labels cannot move the cursor");
    let row = (y / f32::from(screen.cell_height)).ceil() as u16 + 1;
    assert_eq!(app.thumbnail_at(4, row), Some(PageId(partial)));
    assert_eq!(app.thumbnail_at(4, screen.rows - 1), None);
    let started = Instant::now();
    while !app.complete {
      assert!(started.elapsed() < Duration::from_secs(10));
      app.raster_updates();
      out.clear();
      app.draw(&mut out)?;
      std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.requested, wanted);
    app.overview.as_mut().unwrap().selected = partial - 1;
    app.overview_input(Input::plain(Action::Motion(Motion::Right)))?;
    out.clear();
    app.draw(&mut out)?;
    assert_eq!(app.overview.as_ref().unwrap().top(), 1);
    for key in wanted.iter().filter(|k| k.page.0 >= partial) {
      assert!(app.requested.contains(key), "reuse the partial row's tiles");
    }
    app.overview_input(Input::plain(Action::Motion(Motion::Last)))?;
    out.clear();
    app.draw(&mut out)?;
    assert_eq!(
      app.requested.iter().map(|key| key.page.0).max(),
      Some(app.doc.pages.len() - 1)
    );
    assert_eq!(app.thumbnail_at(4, screen.rows - 2), None);
    Ok(())
  }
}
