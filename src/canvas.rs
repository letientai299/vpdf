use crate::Result;
use crate::app::App;
use crate::layout::Screen;
use crate::reader::SelectionKind;
use crate::terminal;
use crate::text::{Bounds, PageId, Position};
use crate::tiles::{self, TileKey};
use std::io::Write;

impl App {
  /// Keeps finished tiles; returns whether a wanted one arrived, since
  /// prefetched tiles need no frame.
  pub fn raster_updates(&mut self) -> bool {
    let mut changed = false;
    for result in self.worker.results.try_iter() {
      match result {
        Ok(tile) => {
          self.render_ms = tile.render_ms;
          changed |= self.requested.contains(&tile.key);
          self.graphics.keep(tile);
        }
        Err(error) => {
          self.error = Some(error);
          changed = true;
        }
      }
    }
    changed
  }

  pub fn canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let (screen, left) = (self.uncovered(), self.left);
    let mut tiles = tiles::visible(&self.layout, self.reader.scroll);
    if let Some(animation) = &self.animation {
      self.heading = (animation.target - self.reader.scroll).signum();
      for tile in tiles::visible(&self.layout, animation.target) {
        if !tiles.contains(&tile) {
          tiles.push(tile);
        }
      }
    }
    let mut missing = Vec::new();
    for key in &tiles {
      if !self.graphics.place(
        key,
        &self.layout,
        self.reader.scroll,
        screen,
        left,
        out,
      )? {
        missing.push(*key);
      }
    }
    if tiles != self.requested {
      missing.sort_by_key(|key| {
        let page = &self.layout.pages[key.page.0];
        ((page.top + key.y as f32 - self.reader.scroll).abs() as u32, key.x)
      });
      let urgent = missing.len();
      for key in self.ahead(&tiles) {
        if !self.graphics.has(&key) {
          missing.push(key);
        }
      }
      self.worker.request(&missing, urgent);
      self.requested = tiles;
    }
    self.marks(screen, out)?;
    if self.panel_focused() {
      // The side without focus recedes.
      self.graphics.veil([0, 0, 0, 70], screen, left, out)?;
    }
    Ok(())
  }

  /// Tiles a screen beyond the view each way, the way of the last scroll
  /// first and nearest first, so scrolling finds them already rendered.
  fn ahead(&self, near: &[TileKey]) -> Vec<TileKey> {
    let base = self.animation.as_ref().map_or(self.reader.scroll, |a| a.target);
    let height = self.layout.screen.height();
    let mut keys = Vec::new();
    for step in [self.heading, -self.heading] {
      let scroll = self.layout.clamp_scroll(base + step * height);
      let mut screen = tiles::visible(&self.layout, scroll);
      if step < 0.0 {
        screen.reverse();
      }
      for key in screen {
        if !near.contains(&key) && !keys.contains(&key) {
          keys.push(key);
        }
      }
    }
    keys
  }

  /// Whether the frame writes no text above the status line.
  fn plain(&self) -> bool {
    self.overview.is_none()
      && !self.text_mode
      && self.hints.is_none()
      && self.peek.is_none()
      && self.region.is_none()
      && self.panel.is_none()
      && self.clues.is_none()
      && self.prompt.is_none()
  }

  pub fn panel_focused(&self) -> bool {
    self.panel.as_ref().is_some_and(|panel| panel.focused)
  }

  /// Selection overlays clipped to `screen`.
  pub fn marks(&mut self, screen: Screen, out: &mut impl Write) -> Result<()> {
    if self.cursorline && self.reader.selection.is_some() {
      self.line_band(
        self.reader.cursor,
        crate::theme::CURSORLINE,
        screen,
        out,
      )?;
    }
    if let Some(landing) = &self.landing {
      self.line_band(landing.position, crate::theme::LANDING, screen, out)?;
    }
    if !self.search.hidden {
      let visible = self.layout.visible(self.reader.scroll);
      let hits = &self.search.hits;
      let start = hits.partition_point(|hit| hit.first.page.0 < visible.start);
      let ranges: Vec<_> = hits[start..]
        .iter()
        .take_while(|hit| hit.first.page.0 < visible.end)
        .take(300)
        .map(|hit| (hit.first, hit.last))
        .collect();
      for (first, last) in ranges {
        self.mark_range(first, last, false, [255, 190, 30, 90], screen, out)?;
      }
    }
    if let Some(selection) = self.reader.selection {
      self.mark_range(
        selection.anchor,
        self.reader.cursor,
        selection.kind == SelectionKind::Line,
        [70, 130, 255, 85],
        screen,
        out,
      )?;
    }
    if !self.reader.uses_cursor() {
      return Ok(());
    }
    let cursor = self.reader.cursor;
    if let Some(glyph) = self.doc.text(cursor.page)?.glyph(cursor) {
      let mut rect =
        self.layout.project(cursor.page, glyph.bounds, self.reader.scroll);
      // Translucent block keeps the glyph readable.
      rect.x1 = rect.x1.max(rect.x0 + 2.0);
      self.graphics.highlight(
        rect,
        [25, 100, 255, 120],
        screen,
        self.left,
        out,
      )?;
    }
    Ok(())
  }

  fn line_band(
    &mut self,
    position: Position,
    color: [u8; 4],
    screen: Screen,
    out: &mut impl Write,
  ) -> Result<()> {
    let text = self.doc.text(position.page)?;
    let Some(bounds) = text.lines.get(position.line).and_then(|line| {
      line.glyphs.iter().map(|glyph| glyph.bounds).reduce(Bounds::union)
    }) else {
      return Ok(());
    };
    let page = self.layout.crop.apply(self.doc.pages[position.page.0].bounds);
    let band = Bounds {
      x0: page.x0,
      x1: page.x1,
      y0: bounds.y0.max(page.y0),
      y1: bounds.y1.min(page.y1),
    };
    if band.height() > 0.0 {
      let rect = self.layout.project(position.page, band, self.reader.scroll);
      self.graphics.backdrop(rect, color, screen, self.left, out)?;
    }
    Ok(())
  }

  /// Glyph rectangle in page-area pixels.
  pub fn position_rect(&mut self, cursor: Position) -> Result<Option<Bounds>> {
    if self.text_mode {
      let cell = self.text_cell(cursor)?;
      let (width, height) =
        (f32::from(self.screen.cell_width), f32::from(self.screen.cell_height));
      return Ok(cell.map(|(column, row)| {
        let (x, y) =
          (f32::from(column - self.left) * width, f32::from(row) * height);
        Bounds { x0: x, y0: y, x1: x + width, y1: y + height }
      }));
    }
    let text = self.doc.text(cursor.page)?;
    Ok(text.glyph(cursor).map(|glyph| {
      self.layout.project(cursor.page, glyph.bounds, self.reader.scroll)
    }))
  }

  pub fn mark_range(
    &mut self,
    first: Position,
    last: Position,
    linewise: bool,
    color: [u8; 4],
    screen: Screen,
    out: &mut impl Write,
  ) -> Result<()> {
    let range = crate::motion::range(first, last);
    let (first, last) = range;
    for page in self.layout.visible(self.reader.scroll) {
      if page < first.page.0 || page > last.page.0 {
        continue;
      }
      let text = self.doc.text(PageId(page))?;
      let skip = if page == first.page.0 { first.line } else { 0 };
      for (line, row) in text.lines.iter().enumerate().skip(skip) {
        if page == last.page.0 && line > last.line {
          break;
        }
        let covered = |glyph| {
          crate::motion::covers(
            range,
            linewise,
            Position { page: PageId(page), line, glyph },
          )
        };
        let bounds = row
          .glyphs
          .iter()
          .enumerate()
          .filter(|(glyph, _)| covered(*glyph))
          .map(|(_, ch)| ch.bounds)
          .reduce(Bounds::union);
        if let Some(bounds) = bounds {
          let rect =
            self.layout.project(PageId(page), bounds, self.reader.scroll);
          self.graphics.highlight(rect, color, screen, self.left, out)?;
        }
      }
    }
    Ok(())
  }

  /// One frame with this pane alone on screen.
  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "{}\x1b[0m", terminal::frame_start())?;
    self.graphics.begin(out)?;
    self.draw_pane(out)?;
    self.graphics.end(out)?;
    write!(out, "{}", terminal::frame_end())?;
    out.flush()?;
    Ok(())
  }

  /// Draws within the pane; the caller starts and ends the frame.
  pub fn draw_pane(&mut self, out: &mut impl Write) -> Result<()> {
    // A shrunken terminal can leave no room below the status line.
    if self.screen.rows < 2 || self.screen.columns == 0 {
      return Ok(());
    }
    // Rows above the status line stay blank between plain frames.
    let plain = self.plain();
    if !(plain && self.blank) {
      for row in 0..self.screen.rows {
        terminal::clear(out, self.at, row, self.screen.columns)?;
      }
    }
    self.blank = plain;
    if self.covered()
      && let Some(panel) = &mut self.panel
    {
      // Panes share one picture id, which the focused pane holds.
      let graphics =
        (!self.text_mode && self.focused).then_some(&mut self.graphics);
      panel.draw(out, self.at, self.left, self.screen, graphics)?;
      return self.bottom(out);
    }
    if self.overview.is_some() {
      self.overview_canvas(out)?;
    } else if self.text_mode {
      self.text_canvas(out)?;
    } else {
      self.canvas(out)?;
    }
    self.draw_hints(out)?;
    self.draw_preview(out)?;
    self.draw_region(out)?;
    self.track_outline();
    if let Some(panel) = &mut self.panel {
      panel.draw(out, self.at, self.left, self.screen, None)?;
    }
    self.bottom(out)
  }

  /// Status line, or the prompt with its menu and cursor.
  fn bottom(&self, out: &mut impl Write) -> Result<()> {
    let row = self.screen.rows - 1;
    let Some(prompt) = &self.prompt else {
      if let Some(clues) = &self.clues {
        let (labels, grid, keys) = self.clue_grid(clues);
        let bottom = (self.at, row - 1);
        crate::grid::draw(
          out,
          bottom,
          &labels,
          &grid,
          None,
          keys,
          self.screen,
        )?;
      }
      self.status(out, row)?;
      write!(out, "\x1b[?25l")?;
      return Ok(());
    };
    crate::statusline::prompt(out, prompt, self.screen, self.at)
  }
}
