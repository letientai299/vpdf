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
          self.reader.message = error;
          changed = true;
        }
      }
    }
    Ok(changed)
  }

  pub fn canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let tiles = tiles::visible(&self.layout, self.reader.scroll);
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
      rect.x1 = rect.x0 + 2.0;
      self.graphics.highlight(
        rect,
        [25, 100, 255, 230],
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

  pub fn text_canvas(&mut self, out: &mut impl Write) -> Result<()> {
    let text = self.doc.text(self.reader.cursor.page)?;
    let height = self.screen.rows.saturating_sub(1) as usize;
    let start = self.reader.cursor.line.saturating_sub(height / 2);
    for (row, line) in text.lines.iter().skip(start).take(height).enumerate() {
      let marker =
        if start + row == self.reader.cursor.line { "> " } else { "  " };
      terminal::line(
        out,
        row as u16,
        &format!("{marker}{}", line.text()),
        self.screen.columns,
      )?;
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
    let page = self.reader.cursor.page.0;
    let status = format!(
      " {} / {} [{}]  {:.0}%  {}",
      page + 1,
      self.doc.pages.len(),
      self.doc.pages[page].label,
      self.layout.zoom * 100.0,
      self.reader.message
    );
    write!(out, "\x1b[7m")?;
    terminal::line(out, self.screen.rows - 1, &status, self.screen.columns)?;
    write!(out, "\x1b[0m\x1b[?2026l")?;
    out.flush()?;
    Ok(())
  }
}
