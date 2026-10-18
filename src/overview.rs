use crate::Result;
use crate::app::App;
use crate::keys::{Action, Input};
use crate::layout::{Crop, page_matrix, transform};
use crate::motion::Motion;
use crate::pointer::{Hit, Wheel};
use crate::text::{Bounds, PageId, Position};
use crate::tiles::{self, TileKey};
use crate::{terminal, theme};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use std::io::Write;

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
      Action::Motion(Motion::First) => 0,
      Action::Motion(Motion::Last) => last,
      Action::Scroll(amount) => {
        let step = (amount * page as f32) as isize * count as isize;
        selected.saturating_add_signed(step)
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
    let column = ((x - grid.gap / 2.0) / (grid.width + grid.gap)) as usize;
    let row = ((y - grid.gap / 2.0) / grid.row) as usize;
    // As the next frame scrolls it.
    let pages = self.doc.pages.len();
    let index =
      (overview.shown_top(&grid, pages) + row) * grid.columns + column;
    let inside = column < grid.columns && row < grid.rows;
    (inside && index < pages).then_some(PageId(index))
  }

  fn open_selected(&mut self) -> Result<()> {
    let Some(overview) = self.overview.take() else { return Ok(()) };
    let page = PageId(overview.selected);
    self.requested.clear();
    self.go_to(Position { page, ..Position::default() }, true)
  }

  pub fn close_overview(&mut self) {
    self.overview = None;
    self.requested.clear();
  }

  /// Draws visible thumbnails and requests the missing ones.
  pub fn overview_canvas(&mut self, out: &mut impl Write) -> Result<()> {
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
    let visible = first..(first + grid.rows * grid.columns).min(pages);
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
      for key in tiles::cover(whole, 0..w, 0..h) {
        let origin = (x + dx + key.x as f32, y + dy + key.y as f32);
        if !self.graphics.place_at(&key, origin, screen, left, out)? {
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
        self.tint(ring, tint, screen, out)?;
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
    let columns = (width / f32::from(screen.cell_width)) as usize;
    let label = terminal::clean(self.doc.label(PageId(index)), columns);
    let column = (x / f32::from(screen.cell_width)) as usize
      + (columns.saturating_sub(label.len())) / 2;
    let row = (y / f32::from(screen.cell_height)).ceil() as usize;
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
