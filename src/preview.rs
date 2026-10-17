use crate::Result;
use crate::app::App;
use crate::document::{Annotation, Target};
use crate::keys::{Action, Input};
use crate::motion::Motion;
use crate::panel_layout::wrap;
use crate::split::Rect;
use crate::text::{Bounds, PageId, Position};
use crate::theme;
use crate::tiles::{TileKey, raster_area};
use mupdf::IRect;
use std::io::Write;
use unicode_width::UnicodeWidthStr;

const HEIGHT: f32 = 0.4;
const ABOVE: f32 = 6.0;
const IMAGE_BORDER: &str = "\x1b[0;38;2;60;120;230;48;2;255;255;255m";
const NOTE_TEXT: &str = "\x1b[0;38;2;20;20;20;48;2;255;255;255m";

pub struct Preview {
  pub target: Target,
  page: PageId,
  source: Position,
  serial: u64,
  view: View,
}

enum View {
  Image { key: TileKey, rgb: Vec<u8> },
  Text { top: usize, lines: Vec<String> },
  Note { top: usize, contents: String, columns: usize, lines: Vec<String> },
}

impl App {
  pub fn preview_link(&mut self) -> Result<()> {
    let Some(position) = self.explicit_target()? else { return Ok(()) };
    if let Some(annotation) = self.annotation_at(position)? {
      self.preview_annotation(position, &annotation);
      return Ok(());
    }
    let target = self
      .link_target(position)?
      .ok_or_else(|| anyhow::anyhow!("No link or note under cursor"))?;
    self.show_preview(target, position)
  }

  /// Previews `target` beside `source`.
  pub(crate) fn show_preview(
    &mut self,
    target: Target,
    source: Position,
  ) -> Result<()> {
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
    self.peek = Some(Preview {
      target,
      page,
      source,
      serial: crate::graphics::serial(),
      view,
    });
    Ok(())
  }

  pub(crate) fn preview_annotation(
    &mut self,
    position: Position,
    annotation: &Annotation,
  ) {
    if annotation.contents.is_empty() {
      return;
    }
    let columns = self.note_columns();
    let target =
      annotation.areas.first().map_or(Target::Page(position.page), |area| {
        Target::Point { page: position.page, x: area.x0, y: area.y0 }
      });
    self.peek = Some(Preview {
      target,
      page: position.page,
      source: position,
      serial: crate::graphics::serial(),
      view: View::Note {
        top: 0,
        lines: wrap(&annotation.contents, columns),
        contents: annotation.contents.clone(),
        columns,
      },
    });
  }

  fn note_columns(&self) -> usize {
    usize::from(self.layout.screen.columns).saturating_sub(4).min(72)
  }

  fn reflow_note(&mut self) {
    let columns = self.note_columns();
    if let Some(Preview {
      view: View::Note { contents, columns: old, lines, .. },
      ..
    }) = &mut self.peek
      && *old != columns
    {
      *lines = wrap(contents, columns);
      *old = columns;
    }
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
    let screen = self.layout.screen;
    let cell_width = i32::from(screen.cell_width);
    let width = ((layout.width - left as f32).min(screen.width() * 0.95)
      as i32
      / cell_width)
      .min(i32::from(screen.columns.saturating_sub(2)))
      * cell_width;
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
    let rgb = pixmap.samples().to_vec();
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
        if matches!(peek.view, View::Note { .. }) {
          return Ok(());
        }
        return self.follow(&peek.target);
      }
      Action::Quit | Action::Peek => {
        self.dismiss_overlay();
        return Ok(());
      }
      action if self.overlay_transfer(action) => return self.action(input),
      _ => return Ok(()),
    };
    self.scroll_preview(rows * input.count as f32)
  }

  /// How far the preview is scrolled: image pixels or text rows.
  pub fn preview_top(&self) -> Option<i32> {
    self.peek.as_ref().map(|preview| match &preview.view {
      View::Image { key, .. } => key.y,
      View::Text { top, .. } | View::Note { top, .. } => {
        i32::try_from(*top).unwrap_or(i32::MAX)
      }
    })
  }

  /// Scrolls the owned preview by terminal rows.
  pub fn scroll_preview(&mut self, rows: f32) -> Result<()> {
    self.reflow_note();
    let Some(preview) = &self.peek else { return Ok(()) };
    let page = preview.page;
    if page.0 >= self.doc.pages.len() || page.0 >= self.layout.pages.len() {
      self.peek = None;
      return Ok(());
    }
    let visible = self.preview_rows();
    match &preview.view {
      View::Text { top, lines } | View::Note { top, lines, .. } => {
        let step = rows.round() as isize;
        let top = top
          .saturating_add_signed(step)
          .min(lines.len().saturating_sub(visible));
        if let Some(Preview {
          view:
            View::Text { top: current, .. } | View::Note { top: current, .. },
          ..
        }) = &mut self.peek
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
        key.width as usize / usize::from(screen.cell_width),
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
      View::Note { top, lines, columns, .. } => (
        *columns,
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
    let y = top as f32 * f32::from(screen.cell_height);
    let rect = Bounds {
      x0: x,
      y0: y,
      x1: x + (columns + 2) as f32 * f32::from(screen.cell_width),
      y1: y + (rows + 2) as f32 * f32::from(screen.cell_height),
    };
    self.graphics.backdrop(rect, [255; 4], screen, self.left, out)
  }

  /// Pane cells the preview and its border cover.
  pub fn preview_frame(&mut self) -> Result<Option<Rect>> {
    let Some(source) = self.peek.as_ref().map(|preview| preview.source) else {
      return Ok(None);
    };
    let cursor = self.position_rect(source)?;
    let Some(preview) = &self.peek else { return Ok(None) };
    let screen = self.layout.screen;
    let cell = f32::from(screen.cell_height);
    let (columns, rows, _) = self.preview_extent(preview);
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
    let cells = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    Ok(Some(Rect {
      column: cells(column),
      row: cells(top),
      columns: cells(columns + 2),
      rows: cells(rows + 2),
    }))
  }

  pub fn draw_preview(&mut self, out: &mut impl Write) -> Result<()> {
    if self.peek.is_none() {
      return Ok(());
    }
    self.scroll_preview(0.0)?;
    let Some(frame) = self.preview_frame()? else { return Ok(()) };
    let Some(preview) = &self.peek else { return Ok(()) };
    let screen = self.layout.screen;
    let cell = f32::from(screen.cell_height);
    let (_, _, clipped) = self.preview_extent(preview);
    let (top, column) = (usize::from(frame.row), usize::from(frame.column));
    let rows = usize::from(frame.rows) - 2;
    let columns = usize::from(frame.columns) - 2;
    let label =
      border_label(&self.doc.pages[preview.page.0].label, columns, clipped.0);
    let rule = "─".repeat(columns.saturating_sub(label.width()));
    let over_page = matches!(preview.view, View::Image { .. })
      || !self.text_mode && matches!(preview.view, View::Note { .. });
    if over_page {
      self.preview_backdrop(out, (top, column), (rows, columns))?;
    }
    let Some(preview) = &self.peek else { return Ok(()) };
    let border = if over_page { IMAGE_BORDER } else { theme::CURRENT };
    let text_style = if over_page { NOTE_TEXT } else { theme::RESET };
    write!(
      out,
      "{}{}╭{label}{rule}╮{}",
      self.at.goto(top, column),
      border,
      theme::RESET
    )?;
    match &preview.view {
      View::Image { key, rgb } => {
        let pad = " ".repeat(columns);
        for row in top + 1..=top + rows {
          write!(
            out,
            "{}{border}│{pad}│{}",
            self.at.goto(row, column),
            theme::RESET
          )?;
        }
        let x = (column + 1 - usize::from(self.left)) as f32
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
      View::Text { top: first, lines }
      | View::Note { top: first, lines, .. } => {
        for (index, line) in lines.iter().skip(*first).take(rows).enumerate() {
          let text = crate::terminal::clean(line, columns);
          let pad = columns.saturating_sub(text.width());
          write!(
            out,
            "{}{}│{}{text}{}{}│",
            self.at.goto(top + index + 1, column),
            border,
            text_style,
            " ".repeat(pad),
            border
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
      border,
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
