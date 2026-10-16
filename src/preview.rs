use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::text::{Bounds, PageId};
use crate::theme;
use crate::tiles::{TileKey, raster_area};
use mupdf::IRect;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use unicode_width::UnicodeWidthStr;

/// Share of the view a preview may cover.
const HEIGHT: f32 = 0.4;
/// Points shown above the destination, so its first line is whole.
const ABOVE: f32 = 6.0;
/// Accent border width in pixels.
const BORDER: usize = 2;

/// A rendered or extracted look at a link destination.
pub struct Preview {
  pub target: Target,
  serial: u64,
  /// Pixels with border, for Kitty.
  image: Option<(i32, i32, Vec<u8>)>,
  /// Destination text lines, for text mode.
  lines: Vec<String>,
}

impl App {
  /// `K`: shows where the link under the cursor leads.
  pub fn preview_link(&mut self) -> Result<()> {
    let target = self
      .link_at_cursor()?
      .ok_or_else(|| anyhow::anyhow!("No link under cursor"))?;
    let (page, y) = match &target {
      Target::Uri(uri) => {
        self.reader.message = uri.clone();
        return Ok(());
      }
      Target::Page(page) => (*page, 0.0),
      Target::Point { page, y, .. } => (*page, *y),
    };
    let page = PageId(page.0.min(self.doc.pages.len() - 1));
    let lines = self.target_lines(page, y)?;
    let image = if self.text_mode { None } else { Some(self.render(page, y)?) };
    static SERIAL: AtomicU64 = AtomicU64::new(1);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    self.peek = Some(Preview { target, serial, image, lines });
    Ok(())
  }

  fn target_lines(&mut self, page: PageId, y: f32) -> Result<Vec<String>> {
    let text = self.doc.text(page)?;
    let rows = (f32::from(self.screen.rows) * HEIGHT) as usize;
    Ok(
      text
        .lines
        .iter()
        .filter(|line| line.glyphs.first().is_some_and(|g| g.bounds.y1 >= y))
        .take(rows.saturating_sub(2))
        .map(|line| line.text().trim_end().to_owned())
        .collect(),
    )
  }

  /// The destination at reading scale, full page width, with a border.
  fn render(&mut self, page: PageId, y: f32) -> Result<(i32, i32, Vec<u8>)> {
    let layout = &self.layout.pages[page.0];
    let top = crate::layout::transform(
      Bounds { x0: 0.0, y0: y - ABOVE, x1: 0.0, y1: y - ABOVE },
      &layout.matrix,
    )
    .y0
    .max(0.0) as i32;
    let width = (layout.width.min(self.layout.screen.width() * 0.95)) as i32;
    let height = (self.layout.screen.height() * HEIGHT)
      .min(layout.height - top as f32) as i32;
    anyhow::ensure!(width > 8 && height > 8, "Preview area too small");
    let key = TileKey {
      page,
      scale: layout.scale,
      rotation: self.layout.rotation,
      x: 0,
      y: 0,
      width: 0,
      height: 0,
      crop: self.layout.crop,
    };
    let left = self.layout.pan.max(0.0) as i32;
    let area = IRect::new(left, top, left + width, top + height);
    let pixmap = raster_area(&mut self.doc, &key, area)?;
    let mut rgb = pixmap.samples().to_vec();
    frame(&mut rgb, width as usize, height as usize);
    Ok((width, height, rgb))
  }

  /// Draws below the cursor line, or above when there is no room.
  pub fn draw_preview(&mut self, out: &mut impl Write) -> Result<()> {
    if self.peek.is_none() {
      return Ok(());
    }
    let cursor = self.cursor_rect()?;
    let Some(preview) = &self.peek else { return Ok(()) };
    let screen = self.layout.screen;
    let cell = f32::from(screen.cell_height);
    let Some((width, height, rgb)) = &preview.image else {
      return self.draw_preview_text(out, cursor.map_or(0.0, |r| r.y1) / cell);
    };
    let (w, h) = (*width as f32, *height as f32);
    let below = cursor.map_or(0.0, |r| r.y1 + cell * 0.25);
    let y = if below + h <= screen.height() {
      below
    } else {
      (cursor.map_or(0.0, |r| r.y0) - h - cell * 0.25).max(0.0)
    };
    let x = ((screen.width() - w) / 2.0).max(0.0);
    let rect = Bounds { x0: x, y0: y, x1: x + w, y1: y + h };
    let serial = preview.serial;
    let left = self.left;
    self.graphics.picture(
      serial,
      (*width, *height, rgb),
      rect,
      screen,
      left,
      out,
    )
  }

  fn draw_preview_text(&self, out: &mut impl Write, row: f32) -> Result<()> {
    let Some(preview) = &self.peek else { return Ok(()) };
    let columns = usize::from(self.layout.screen.columns).saturating_sub(4);
    let rows = preview.lines.len() + 2;
    let available = usize::from(self.screen.rows.saturating_sub(1));
    let below = row as usize + 1;
    let top = if below + rows <= available {
      below
    } else {
      below.saturating_sub(rows + 2)
    };
    let column = usize::from(self.left) + 1;
    let rule = "─".repeat(columns);
    let at = self.at;
    write!(out, "{}{}╭{rule}╮", at.goto(top, column), theme::MUTED)?;
    for (index, line) in preview.lines.iter().enumerate() {
      let text = crate::terminal::clean(line, columns);
      let pad = columns - text.width();
      write!(
        out,
        "{}{}│{}{text}{}{}│",
        at.goto(top + index + 1, column),
        theme::MUTED,
        theme::RESET,
        " ".repeat(pad),
        theme::MUTED
      )?;
    }
    let bottom = at.goto(top + rows - 1, column);
    write!(out, "{bottom}╰{rule}╯{}", theme::RESET)?;
    Ok(())
  }
}

/// Paints an accent border so the preview reads as a separate surface.
fn frame(rgb: &mut [u8], width: usize, height: usize) {
  const ACCENT: [u8; 3] = [60, 120, 230];
  for y in 0..height {
    for x in 0..width {
      if x < BORDER || y < BORDER || x + BORDER >= width || y + BORDER >= height
      {
        let at = (y * width + x) * 3;
        rgb[at..at + 3].copy_from_slice(&ACCENT);
      }
    }
  }
}
