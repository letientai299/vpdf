use crate::Result;
use crate::app::App;
use crate::document::Target;
use crate::keys::{Action, Input};
use crate::motion::Motion;
use crate::text::{Bounds, PageId};
use crate::theme;
use crate::tiles::{TileKey, raster_area};
use mupdf::IRect;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

const HEIGHT: f32 = 0.4;
const ABOVE: f32 = 6.0;
const BORDER: usize = 2;
const ACCENT: [u8; 4] = [60, 120, 230, 255];

pub struct Preview {
  pub target: Target,
  page: PageId,
  serial: u64,
  view: View,
}

enum View {
  Image { key: TileKey, rgb: Vec<u8> },
  Text { top: usize, lines: Vec<String> },
}

impl App {
  pub fn preview_link(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    let target = self
      .link_target(position)?
      .ok_or_else(|| anyhow::anyhow!("No link under cursor"))?;
    let (page, x, y) = match &target {
      Target::Uri(uri) => {
        self.reader.message.clone_from(uri);
        return Ok(());
      }
      Target::Page(page) => (*page, 0.0, 0.0),
      Target::Point { page, x, y } => (*page, *x, *y),
    };
    let page = PageId(page.0.min(self.doc.pages.len() - 1));
    let view = if self.text_mode {
      let text = self.doc.text(page)?;
      let top = text
        .lines
        .iter()
        .position(|line| line.glyphs.last().is_some_and(|g| g.bounds.y1 >= y))
        .unwrap_or(text.lines.len());
      let lines: Vec<_> = text
        .lines
        .iter()
        .map(|line| line.text().trim_end().to_owned())
        .collect();
      let top = top.min(lines.len().saturating_sub(self.preview_rows()));
      View::Text { top, lines }
    } else {
      let projected =
        self.layout.pages[page.0].matrix.transform_xy(x, y - ABOVE).1;
      self.preview_image(self.preview_key(page, projected as i32)?)?
    };
    self.peek =
      Some(Preview { target, page, serial: crate::graphics::serial(), view });
    Ok(())
  }

  fn preview_rows(&self) -> usize {
    ((f32::from(self.layout.screen.rows) * HEIGHT) as usize)
      .max(3)
      .min(usize::from(self.layout.screen.rows.saturating_sub(1)))
      .saturating_sub(2)
  }

  fn preview_key(&self, page: PageId, top: i32) -> Result<TileKey> {
    let layout = &self.layout.pages[page.0];
    let (x, _) = self.layout.origin(page, 0.0);
    let left = (-x).max(0.0) as i32;
    let width = (layout.width - left as f32)
      .min(self.layout.screen.width() * 0.95) as i32;
    let height = (self.preview_rows() as f32
      * f32::from(self.layout.screen.cell_height))
    .min(layout.height) as i32;
    anyhow::ensure!(width > 8 && height > 8, "Preview area too small");
    let top = top.clamp(0, (layout.height as i32 - height).max(0));
    Ok(TileKey {
      page,
      scale: layout.scale,
      rotation: self.layout.rotation,
      x: left,
      y: top,
      width,
      height,
      crop: self.layout.crop,
    })
  }

  fn preview_image(&mut self, key: TileKey) -> Result<View> {
    let (width, height) = (key.width, key.height);
    let area = IRect::new(key.x, key.y, key.x + width, key.y + height);
    let pixmap = raster_area(&mut self.doc, &key, area)?;
    let mut rgb = pixmap.samples().to_vec();
    frame(&mut rgb, width as usize, height as usize);
    Ok(View::Image { key, rgb })
  }

  pub(crate) fn preview_input(&mut self, input: Input) -> Result<()> {
    let rows = match input.action {
      Action::Motion(Motion::Down) => 1.0,
      Action::Motion(Motion::Up) => -1.0,
      Action::Scroll(amount) => amount * self.preview_rows() as f32,
      Action::Rows(rows) => rows as f32,
      Action::Command => return self.action(input),
      Action::Follow => {
        let Some(peek) = self.peek.take() else { return Ok(()) };
        return self.follow(&peek.target);
      }
      Action::Quit | Action::Peek => {
        self.peek = None;
        return Ok(());
      }
      Action::Window(_)
      | Action::Tab(_)
      | Action::LocalFocus
      | Action::History(_)
      | Action::Panel(_)
      | Action::ClosePanel => {
        self.peek = None;
        return self.action(input);
      }
      _ => return Ok(()),
    };
    self.scroll_preview(rows * input.count as f32)
  }

  /// Scrolls the owned preview by terminal rows.
  pub fn scroll_preview(&mut self, rows: f32) -> Result<()> {
    let Some(preview) = &self.peek else { return Ok(()) };
    let page = preview.page;
    if page.0 >= self.doc.pages.len() || page.0 >= self.layout.pages.len() {
      self.peek = None;
      return Ok(());
    }
    let visible = self.preview_rows();
    match &preview.view {
      View::Text { top, lines } => {
        let step = rows.round() as isize;
        let top = top
          .saturating_add_signed(step)
          .min(lines.len().saturating_sub(visible));
        if let Some(Preview { view: View::Text { top: current, .. }, .. }) =
          &mut self.peek
        {
          *current = top;
        }
      }
      View::Image { key, .. } => {
        let top = (key.y as f32
          + rows * f32::from(self.layout.screen.cell_height))
          as i32;
        let next = self.preview_key(page, top)?;
        if *key == next {
          return Ok(());
        }
        let view = self.preview_image(next)?;
        if let Some(preview) = &mut self.peek {
          preview.view = view;
          preview.serial = crate::graphics::serial();
        }
      }
    }
    Ok(())
  }

  fn preview_extent(&self, preview: &Preview) -> (usize, usize, (bool, bool)) {
    let screen = self.layout.screen;
    let cell = f32::from(screen.cell_height);
    match &preview.view {
      View::Image { key, .. } => (
        (key.width as usize / usize::from(screen.cell_width)).saturating_sub(2),
        (key.height as f32 / cell).ceil() as usize,
        (
          key.y > 0,
          key.y + key.height < self.layout.pages[preview.page.0].height as i32,
        ),
      ),
      View::Text { top, lines } => (
        usize::from(screen.columns).saturating_sub(4),
        self.preview_rows().min(lines.len()),
        (*top > 0, *top + self.preview_rows() < lines.len()),
      ),
    }
  }

  fn preview_backdrop(
    &mut self,
    out: &mut impl Write,
    (top, column): (usize, usize),
    (rows, columns): (usize, usize),
  ) -> Result<()> {
    let screen = self.layout.screen;
    let x =
      (column - usize::from(self.left)) as f32 * f32::from(screen.cell_width);
    for row in [top, top + rows + 1] {
      let y = row as f32 * f32::from(screen.cell_height);
      let rect = Bounds {
        x0: x,
        y0: y,
        x1: x + (columns + 2) as f32 * f32::from(screen.cell_width),
        y1: y + f32::from(screen.cell_height),
      };
      self.graphics.backdrop(rect, ACCENT, screen, self.left, out)?;
    }
    Ok(())
  }

  pub fn draw_preview(&mut self, out: &mut impl Write) -> Result<()> {
    if self.peek.is_none() {
      return Ok(());
    }
    self.scroll_preview(0.0)?;
    if self.peek.is_none() {
      return Ok(());
    }
    let cursor = match self.target {
      Some(position) => self.position_rect(position)?,
      None => None,
    };
    let Some(preview) = &self.peek else { return Ok(()) };
    let screen = self.layout.screen;
    let cell = f32::from(screen.cell_height);
    let (columns, rows, clipped) = self.preview_extent(preview);
    let available = usize::from(screen.rows.saturating_sub(1));
    let rows = rows.min(available.saturating_sub(2));
    let below = cursor.map_or(0, |r| (r.y1 / cell).ceil() as usize);
    let top = if below + rows + 2 <= available {
      below
    } else {
      ((cursor.map_or(0.0, |r| r.y0) / cell) as usize)
        .saturating_sub(rows + 2)
        .min(available.saturating_sub(rows + 2))
    };
    let column = usize::from(self.left)
      + usize::from(screen.columns).saturating_sub(columns + 2) / 2;
    let label =
      border_label(&self.doc.pages[preview.page.0].label, columns, clipped.0);
    let rule = "─".repeat(columns.saturating_sub(label.width()));
    if matches!(preview.view, View::Image { .. }) {
      self.preview_backdrop(out, (top, column), (rows, columns))?;
    }
    let Some(preview) = &self.peek else { return Ok(()) };
    write!(
      out,
      "{}{}╭{label}{rule}╮{}",
      self.at.goto(top, column),
      theme::BAR,
      theme::RESET
    )?;
    match &preview.view {
      View::Image { key, rgb } => {
        let x = (column - usize::from(self.left)) as f32
          * f32::from(screen.cell_width);
        let y = (top + 1) as f32 * cell;
        let rect = Bounds {
          x0: x,
          y0: y,
          x1: x + key.width as f32,
          y1: y + key.height as f32,
        };
        self.graphics.picture(
          preview.serial,
          (key.width, key.height, rgb),
          rect,
          screen,
          self.left,
          out,
        )?;
      }
      View::Text { top: first, lines } => {
        for (index, line) in lines.iter().skip(*first).take(rows).enumerate() {
          let text = crate::terminal::clean(line, columns);
          let pad = columns.saturating_sub(text.width());
          write!(
            out,
            "{}{}│{}{text}{}{}│",
            self.at.goto(top + index + 1, column),
            theme::MUTED,
            theme::RESET,
            " ".repeat(pad),
            theme::MUTED
          )?;
        }
      }
    }
    let marker = if clipped.1 { "↓" } else { "─" };
    let rule = "─".repeat(columns.saturating_sub(1));
    write!(
      out,
      "{}{}╰{marker}{rule}╯{}",
      self.at.goto(top + rows + 1, column),
      theme::BAR,
      theme::RESET
    )?;
    Ok(())
  }
}

fn border_label(label: &str, columns: usize, clipped: bool) -> String {
  let marker = if clipped { "↑" } else { "" };
  if columns < 4 {
    return crate::terminal::clean(marker, columns);
  }
  let label = crate::terminal::clean(label, columns - marker.width() - 3);
  format!(" {label} {marker} ")
}

fn frame(rgb: &mut [u8], width: usize, height: usize) {
  for y in 0..height {
    for x in 0..width {
      if x < BORDER || y < BORDER || x + BORDER >= width || y + BORDER >= height
      {
        let at = (y * width + x) * 3;
        rgb[at..at + 3].copy_from_slice(&ACCENT[..3]);
      }
    }
  }
}
