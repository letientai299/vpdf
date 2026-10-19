use crate::Result;
use crate::app::App;
use crate::layout::Screen;
use crate::marks::Span;
use crate::panel::Pointing;
use crate::pointer::{Hit, List};
use crate::terminal;
use crate::text::{Bounds, Line, PageId, Position};
use crate::tiles::{self, TileKey};
use std::io::Write;
use vpdf_reader::reader::SelectionKind;

/// Parts of `rect` outside `hole`.
fn around(rect: Bounds, hole: Bounds) -> impl Iterator<Item = Bounds> {
  let (top, bottom) = (hole.y0.max(rect.y0), hole.y1.min(rect.y1));
  [
    Bounds { y1: hole.y0.min(rect.y1), ..rect },
    Bounds { y0: hole.y1.max(rect.y0), ..rect },
    Bounds { y0: top, y1: bottom, x1: hole.x0.min(rect.x1), ..rect },
    Bounds { y0: top, y1: bottom, x0: hole.x1.max(rect.x0), ..rect },
  ]
  .into_iter()
  .filter(|part| part.x1 > part.x0 && part.y1 > part.y0)
}

/// Each line's bounds `range` covers on `page`.
pub(crate) fn covered_rows(
  text: &crate::text::Text,
  page: PageId,
  range: (Position, Position),
  linewise: bool,
) -> impl Iterator<Item = Bounds> + '_ {
  let (first, last) = range;
  let skip = if page == first.page { first.line } else { 0 };
  let rows = text.lines.iter().enumerate().skip(skip);
  let rows =
    rows.take_while(move |(line, _)| page != last.page || *line <= last.line);
  rows.filter_map(move |(line, row)| {
    let glyphs = row.glyphs.iter().enumerate();
    let covered = glyphs.filter(move |(glyph, _)| {
      let at = Position { page, line, glyph: *glyph };
      vpdf_reader::motion::covers(range, linewise, at)
    });
    covered.map(|(_, ch)| ch.bounds).reduce(Bounds::union)
  })
}

impl App {
  /// Keeps finished tiles; returns whether a wanted one arrived, since
  /// prefetched tiles need no frame.
  pub fn raster_updates(&mut self) -> bool {
    let (mut changed, mut failed) = (false, false);
    for result in self.worker.results.try_iter() {
      match result {
        Ok(tile) => {
          self.render_ms = tile.render_ms;
          changed |= self.requested.contains(&tile.key);
          self.graphics.keep(tile);
        }
        Err(error) => {
          self.error = Some(error);
          failed = true;
        }
      }
    }
    // A tile that never comes must not hold back deferred work.
    if failed {
      self.resume_outline();
    }
    changed || failed
  }

  pub fn canvas(&mut self, out: &mut impl Write) -> Result<()> {
    self.request_crop();
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
    self.complete = missing.is_empty();
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
      let nearby =
        if self.crop_job.is_none() { self.nearby_pages() } else { Vec::new() };
      self.worker.prepare(&missing, urgent, &nearby);
      self.requested = tiles;
    }
    self.marks(screen, out)?;
    // The side without focus recedes; a veil would dim a menu over it.
    if self.panel_focused() && self.context_rect().is_none() {
      self.graphics.veil([0, 0, 0, 70], screen, left, out)?;
    }
    Ok(())
  }

  pub(crate) fn nearby_pages(&self) -> Vec<PageId> {
    let scroll =
      self.animation.as_ref().map_or(self.reader.scroll, |a| a.target);
    crate::prefetch::nearby(
      self.doc.pages.len(),
      self.layout.page_at(scroll),
      self.heading >= 0.0,
    )
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
      && self.context_menu.is_none()
      && self.region.is_none()
      && self.panel.is_none()
      && self.clues.is_none()
      && self.prompt.is_none()
      && self.bar.scrub.is_none()
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
    if let Some(landing) = &self.landing
      && landing.span == Span::Line
    {
      self.line_band(landing.position, crate::theme::LANDING, screen, out)?;
    }
    if let Some(position) = self.click_mark() {
      self.glyph_block(position, screen, out)?;
    }
    let mut tinted = self.hover_segments();
    if tinted.is_empty()
      && let Some((page, areas)) = self.hover_icon()
    {
      tinted.extend(areas.into_iter().map(|bounds| (page, bounds)));
    }
    for (page, bounds) in tinted {
      let rect = self.layout.project(page, bounds, self.reader.scroll);
      self.tint(rect, crate::theme::LINK, screen, out)?;
    }
    if !self.search.hidden() {
      let visible = self.layout.visible(self.reader.scroll);
      let hits = self.search.hits();
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
    self.glyph_block(self.reader.cursor, screen, out)
  }

  /// A Cursor-style block at `position`.
  fn glyph_block(
    &mut self,
    position: Position,
    screen: Screen,
    out: &mut impl Write,
  ) -> Result<()> {
    if let Some(glyph) = self.doc.text(position.page)?.glyph(position) {
      let mut rect =
        self.layout.project(position.page, glyph.bounds, self.reader.scroll);
      // Translucent block keeps the glyph readable.
      rect.x1 = rect.x1.max(rect.x0 + 2.0);
      self.tint(rect, [25, 100, 255, 120], screen, out)?;
    }
    Ok(())
  }

  /// Tints `rect` around the context menu.
  pub(crate) fn tint(
    &mut self,
    rect: Bounds,
    color: [u8; 4],
    screen: Screen,
    out: &mut impl Write,
  ) -> Result<()> {
    for piece in self.beside_menu(rect) {
      self.graphics.highlight(piece, color, screen, self.left, out)?;
    }
    Ok(())
  }

  /// `rect` minus the context menu.
  fn beside_menu(&self, rect: Bounds) -> impl Iterator<Item = Bounds> + use<> {
    let hole = self.context_rect();
    let whole = hole.is_none().then_some(rect);
    let parts = hole.into_iter().flat_map(move |hole| around(rect, hole));
    whole.into_iter().chain(parts)
  }

  fn line_band(
    &mut self,
    position: Position,
    color: [u8; 4],
    screen: Screen,
    out: &mut impl Write,
  ) -> Result<()> {
    let text = self.doc.text(position.page)?;
    let Some(bounds) = text.lines.get(position.line).and_then(Line::span)
    else {
      return Ok(());
    };
    let page = self
      .layout
      .crop_for(position.page)
      .apply(self.doc.pages[position.page.0].bounds);
    let band = Bounds {
      x0: page.x0,
      x1: page.x1,
      y0: bounds.y0.max(page.y0),
      y1: bounds.y1.min(page.y1),
    };
    if band.height() > 0.0 {
      let rect = self.layout.project(position.page, band, self.reader.scroll);
      for piece in self.beside_menu(rect) {
        self.graphics.backdrop(piece, color, screen, self.left, out)?;
      }
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
    let range = vpdf_reader::motion::range(first, last);
    let (first, last) = range;
    for page in self.layout.visible(self.reader.scroll) {
      if page < first.page.0 || page > last.page.0 {
        continue;
      }
      let text = self.doc.text(PageId(page))?;
      for bounds in covered_rows(&text, PageId(page), range, linewise) {
        let rect =
          self.layout.project(PageId(page), bounds, self.reader.scroll);
        self.tint(rect, color, screen, out)?;
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
    // Views without tiles show whole at once; a covered one shows none.
    self.complete = !self.covered();
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
    if let Some(panel) = &mut self.panel {
      panel.pointing = match self.hover {
        Hit::Row(index) => Pointing::Row(index),
        Hit::Card(_) => Pointing::Card,
        _ => Pointing::Away,
      };
    }
    if self.covered()
      && let Some(panel) = &mut self.panel
    {
      // Panes share one picture id, which the focused pane holds.
      let graphics =
        (!self.text_mode && self.focused).then_some(&mut self.graphics);
      panel.draw(out, self.at, self.left, self.screen, graphics)?;
      self.draw_context(out)?;
      return self.bottom(out);
    }
    if self.overview.is_some() {
      self.overview_canvas(out)?;
    } else if self.text_mode {
      self.text_canvas(out)?;
    } else {
      self.canvas(out)?;
      self.draw_bar(out)?;
    }
    self.draw_hints(out)?;
    self.draw_preview(out)?;
    self.draw_region(out)?;
    self.track_outline();
    if let Some(panel) = &mut self.panel {
      panel.draw(out, self.at, self.left, self.screen, None)?;
    }
    // Over the panel too.
    self.draw_context(out)?;
    self.bottom(out)
  }

  /// Status line, or the prompt with its menu and cursor.
  fn bottom(&mut self, out: &mut impl Write) -> Result<()> {
    let row = self.screen.rows - 1;
    if let Some(prompt) = &self.prompt {
      let pointed = self.pointed_item(List::Completions);
      let (screen, at) = (self.screen, self.at);
      return crate::statusline::prompt(out, prompt, screen, at, pointed);
    }
    // After the panel, which finds clipped rows.
    // Hidden details cost nothing.
    let detail = if self.shows_detail() {
      self.sweep_hint().or_else(|| self.hover_detail())
    } else {
      None
    };
    if let Some(clues) = &self.clues {
      let (labels, grid, keys) = self.clue_grid(clues);
      let bottom = (self.at, row - 1);
      // Clues have no choice; the pointed one shows chosen, as menu rows.
      let shown = (self.pointed_item(List::Clues), None);
      let screen = self.screen;
      crate::grid::draw(out, bottom, &labels, &grid, shown, keys, screen)?;
    }
    self.status_line(out, row, detail.as_deref())?;
    write!(out, "\x1b[?25l")?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tints_skip_the_menu() {
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 10.0, y1: 10.0 };
    let b = |x0, y0, x1, y1| Bounds { x0, y0, x1, y1 };
    // (hole) -> parts
    let cases = [
      (b(20.0, 20.0, 30.0, 30.0), vec![rect]),
      (b(-5.0, -5.0, 15.0, 15.0), vec![]),
      (
        b(2.0, 2.0, 8.0, 8.0),
        vec![
          b(0.0, 0.0, 10.0, 2.0),
          b(0.0, 8.0, 10.0, 10.0),
          b(0.0, 2.0, 2.0, 8.0),
          b(8.0, 2.0, 10.0, 8.0),
        ],
      ),
      (b(5.0, -5.0, 15.0, 15.0), vec![b(0.0, 0.0, 5.0, 10.0)]),
    ];
    for (hole, want) in cases {
      let got: Vec<_> = around(rect, hole).collect();
      let key = |b: &Bounds| (b.x0, b.y0, b.x1, b.y1);
      let got: Vec<_> = got.iter().map(key).collect();
      let want: Vec<_> = want.iter().map(key).collect();
      assert_eq!(got, want, "{hole:?}");
    }
  }
}
