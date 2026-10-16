use crate::Result;
use crate::app::App;
use crate::reader::SelectionKind;
use crate::terminal;
use crate::text::{Bounds, PageId, Position};
use crate::tiles;
use std::io::Write;

impl App {
  pub fn raster_updates(&mut self, out: &mut impl Write) -> Result<bool> {
    let mut changed = false;
    for result in self.worker.results.try_iter() {
      match result {
        Ok(tile) => {
          self.render_ms = tile.render_ms;
          self.graphics.upload(&tile, out)?;
          changed = true;
        }
        Err(error) => {
          self.error = Some(error);
          changed = true;
        }
      }
    }
    Ok(changed)
  }

  pub fn canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let mut tiles = tiles::visible(&self.layout, self.reader.scroll);
    if let Some(animation) = &self.animation {
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
        self.left,
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
      self.worker.request(&missing);
      self.requested = tiles;
    }
    self.marks(out)
  }

  pub fn marks(&mut self, out: &mut impl Write) -> Result<()> {
    if !self.search.hidden {
      let visible = self.layout.visible(self.reader.scroll);
      let ranges: Vec<_> = self
        .search
        .hits
        .iter()
        .filter(|hit| visible.contains(&hit.first.page.0))
        .take(300)
        .map(|hit| (hit.first, hit.last))
        .collect();
      for (first, last) in ranges {
        self.mark_range(first, last, false, [255, 190, 30, 90], out)?;
      }
    }
    if let Some(selection) = self.reader.selection {
      self.mark_range(
        selection.anchor,
        self.reader.cursor,
        selection.kind == SelectionKind::Line,
        [70, 130, 255, 85],
        out,
      )?;
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
        self.layout.screen,
        self.left,
        out,
      )?;
    }
    Ok(())
  }

  pub fn mark_range(
    &mut self,
    first: Position,
    last: Position,
    linewise: bool,
    color: [u8; 4],
    out: &mut impl Write,
  ) -> Result<()> {
    let (first, last) = crate::motion::range(first, last);
    for page in self.layout.visible(self.reader.scroll) {
      if page < first.page.0 || page > last.page.0 {
        continue;
      }
      let text = self.doc.text(PageId(page))?;
      for (line, row) in text.lines.iter().enumerate() {
        if page == first.page.0 && line < first.line {
          continue;
        }
        if page == last.page.0 && line > last.line {
          break;
        }
        let mut bounds: Option<Bounds> = None;
        for (glyph, ch) in row.glyphs.iter().enumerate() {
          let pos = Position { page: PageId(page), line, glyph };
          let selected = if linewise {
            (pos.page, pos.line) >= (first.page, first.line)
              && (pos.page, pos.line) <= (last.page, last.line)
          } else {
            pos >= first && pos <= last
          };
          if !selected {
            continue;
          }
          bounds = Some(bounds.map_or(ch.bounds, |b| Bounds {
            x0: b.x0.min(ch.bounds.x0),
            y0: b.y0.min(ch.bounds.y0),
            x1: b.x1.max(ch.bounds.x1),
            y1: b.y1.max(ch.bounds.y1),
          }));
        }
        if let Some(bounds) = bounds {
          let rect =
            self.layout.project(PageId(page), bounds, self.reader.scroll);
          self.graphics.highlight(
            rect,
            color,
            self.layout.screen,
            self.left,
            out,
          )?;
        }
      }
    }
    Ok(())
  }

  pub fn draw(&mut self, out: &mut impl Write) -> Result<()> {
    write!(out, "\x1b[?2026h\x1b[0m")?;
    for row in 0..self.screen.rows {
      terminal::line(out, row, "", self.screen.columns)?;
    }
    self.graphics.begin(out)?;
    if self.text_mode {
      self.text_canvas(out)?;
    } else {
      self.canvas(out)?;
    }
    self.track_outline();
    if let Some(panel) = &mut self.panel {
      panel.draw(out, self.left, self.screen.rows)?;
    }
    self.bottom(out)?;
    write!(out, "\x1b[?2026l")?;
    out.flush()?;
    Ok(())
  }

  /// Status line, or the prompt with its menu and cursor.
  fn bottom(&self, out: &mut impl Write) -> Result<()> {
    let row = self.screen.rows - 1;
    let Some(prompt) = &self.prompt else {
      self.status(out, row)?;
      write!(out, "\x1b[?25l")?;
      return Ok(());
    };
    crate::statusline::prompt(out, prompt, self.screen)
  }
}
