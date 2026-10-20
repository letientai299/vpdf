use crate::Result;
use crate::app::App;
use crate::pdf_text::PdfText;
use crossterm::event::MouseEvent;
use std::io::Write;
use vpdf_model::text::PageId;
use vpdf_terminal::graphics::Graphics;
use vpdf_ui::keys::Input;
use vpdf_ui::overview::{Frame, Grid, Navigation, Overview, Pointer};
use vpdf_ui::pointer::Hit;

impl App {
  fn grid(&self) -> Grid {
    Grid::new(self.layout.screen(), self.doc.pages[0].bounds)
  }
  pub fn open_overview(&mut self) -> Result<()> {
    anyhow::ensure!(!self.text_mode, "Page overview needs Kitty graphics");
    let selected = self.navigation_position().page;
    self.overlays.open_overview(Overview::new(selected));
    self.tiles.invalidate();
    if let Some(panel) = &mut self.panel {
      panel.view.focus(false);
    }
    self.relayout()
  }

  pub fn overview_input(&mut self, input: Input) -> Result<bool> {
    let grid = self.grid();
    let pages = self.doc.pages.len();
    let Some(overview) = self.overlays.overview_mut() else { return Ok(false) };
    match overview.navigate(input, &grid, pages) {
      Navigation::Open => self.open_selected().map(|()| true),
      Navigation::Close => {
        self.close_overview();
        Ok(true)
      }
      Navigation::Handled => Ok(true),
      Navigation::Pass => Ok(false),
    }
  }

  pub fn overview_mouse(&mut self, mouse: MouseEvent) -> Result<()> {
    let grid = self.grid();
    let pages = self.doc.pages.len();
    let hit = self.thumbnail_at(mouse.column, mouse.row);
    let Some(overview) = self.overlays.overview_mut() else { return Ok(()) };
    match overview.pointer(mouse.kind, hit, &grid, pages) {
      Pointer::None => Ok(()),
      Pointer::Open => self.open_selected(),
      Pointer::Split(page) => {
        self.open_split(&vpdf_model::document::Target::Page(page))
      }
      Pointer::Context(hit) => {
        self.overview_context(mouse, hit);
        Ok(())
      }
    }
  }

  pub fn thumbnail_at(&self, column: u16, row: u16) -> Option<PageId> {
    let overview = self.overlays.overview()?;
    overview.hit(
      self.cell_point(column, row, None),
      &self.grid(),
      self.doc.pages.len(),
      self.uncovered(),
    )
  }
  fn open_selected(&mut self) -> Result<()> {
    let Some(overview) = self.overlays.close_overview() else { return Ok(()) };
    let page = PageId(overview.selected());
    self.tiles.invalidate();
    self.go_to_page(page)
  }

  pub fn close_overview(&mut self) {
    self.overlays.close_overview();
    self.tiles.invalidate();
  }

  pub fn overview_canvas(
    &mut self,
    graphics: &mut Graphics,
    out: &mut impl Write,
  ) -> Result<()> {
    let mut painter =
      vpdf_terminal::painter::Painter::new(out).with_graphics(graphics);
    let result = self.scene_overview(&mut painter);
    painter.finish()?;
    result
  }

  pub(crate) fn scene_overview<'a, S: vpdf_model::scene::Surface<'a>>(
    &mut self,
    scene: &mut S,
  ) -> Result<()>
  where
    S::Error: Into<anyhow::Error>,
  {
    let frame = Frame {
      pane: self.id,
      screen: self.layout.screen(),
      clip: self.uncovered(),
      at: self.at,
      left: self.left,
      hovered: match self.hover.hit() {
        Hit::Thumbnail(page) => Some(page),
        _ => None,
      },
      mask: self.context_rect(),
    };
    let Some(overview) = self.overlays.overview_mut() else { return Ok(()) };
    let tiles = overview
      .scene(&PdfText(&mut self.doc), frame, scene)
      .map_err(Into::into)?;
    let update = self.tiles.update(tiles);
    self.complete = update.complete;
    if let Some(request) = update.request {
      self.worker.service.request(request.keys(), request.urgent());
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::{Duration, Instant};
  use vpdf_reader::motion::Motion;
  use vpdf_ui::keys::Action;
  use vpdf_ui::layout::Screen;
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
    let (columns, rows) = grid.shape();
    let partial = rows * columns;
    let y = grid.row_top(rows);
    assert!(y < screen.height() && screen.height() - y < 256.0);
    let mut out = Vec::new();
    app.draw(&mut out)?;
    let wanted = app.tiles.wanted().to_vec();
    for page in partial..partial + columns {
      let keys: Vec<_> = wanted.iter().filter(|k| k.page.0 == page).collect();
      assert_eq!(keys.len(), 1, "only the visible tile of page {page}");
      assert_eq!(keys[0].y, 0);
    }
    assert!(wanted.iter().all(|k| k.page.0 < partial + columns));
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
    assert_eq!(app.tiles.wanted(), wanted.as_slice());
    app.overlays.overview_mut().unwrap().choose(PageId(partial - 1));
    app.overview_input(Input::plain(Action::Motion(Motion::Right)))?;
    out.clear();
    app.draw(&mut out)?;
    assert_eq!(app.overlays.overview().unwrap().top(), 1);
    for key in wanted.iter().filter(|k| k.page.0 >= partial) {
      assert!(app.tiles.wants(key), "reuse the partial row's tiles");
    }
    app.overview_input(Input::plain(Action::Motion(Motion::Last)))?;
    out.clear();
    app.draw(&mut out)?;
    assert_eq!(
      app.tiles.wanted().iter().map(|key| key.page.0).max(),
      Some(app.doc.pages.len() - 1)
    );
    assert_eq!(app.thumbnail_at(4, screen.rows - 2), None);
    Ok(())
  }
}
