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
/// Strip rows between pages.
const GAP: u8 = 200;
/// Pages one text scroll loads at most.
const LOADS: usize = 4;

pub struct Preview {
  pub target: Target,
  page: PageId,
  pub(crate) source: Position,
  serial: u64,
  view: View,
  pub origin: Origin,
  /// Where the last frame drew it.
  pub placement: Option<Placement>,
  /// Its source has shown since it opened.
  seen: bool,
}

/// A drawn preview's cells.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
  /// The preview, border included.
  pub area: Rect,
  /// Gap cells to its link.
  pub bridge: Rect,
  /// The view it was drawn in.
  pub(crate) view: (u32, Position),
}

impl Preview {
  /// The link a hover preview shows.
  pub fn hovered(&self) -> Option<(PageId, usize)> {
    match self.origin {
      Origin::Hover(page, index) => Some((page, index)),
      Origin::Key => None,
    }
  }

  /// Clicks and Enter follow; notes have none.
  pub fn follows(&self) -> bool {
    !matches!(self.view, View::Note { .. })
  }
}

/// What opened a preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
  /// `K`; it owns keys.
  Key,
  /// Resting on a page's link; passive.
  Hover(PageId, usize),
}

enum View {
  /// A strip from its page onward.
  Image {
    key: TileKey,
    rgb: Vec<u8>,
  },
  /// Lines from its page onward; page `starts`.
  Text {
    top: usize,
    lines: Vec<String>,
    starts: Vec<usize>,
  },
  Note {
    top: usize,
    contents: String,
    columns: usize,
    lines: Vec<String>,
  },
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
      // Past its last line, show that line.
      let top = top.min(text.lines.len().saturating_sub(1));
      View::Text { top, lines: page_lines(&text), starts: vec![0] }
    } else {
      let projected =
        self.layout.pages[page.0].matrix.transform_xy(x, y - ABOVE).1;
      self.preview_image(self.preview_key(page, projected as i32)?)?
    };
    // Panel notes may open off screen.
    let seen = self.visible_position(source).unwrap_or(false);
    self.peek = Some(Preview {
      target,
      page,
      source,
      serial: crate::graphics::serial(),
      view,
      origin: Origin::Key,
      placement: None,
      seen,
    });
    // Fill a late target from later pages.
    self.scroll_preview(0.0)
  }

  /// Loads later pages toward `need` lines.
  fn extend_text(&mut self, need: usize) {
    // Large counts load in steps.
    for _ in 0..LOADS {
      let Some(preview @ Preview { view: View::Text { lines, .. }, .. }) =
        &self.peek
      else {
        return;
      };
      let Some(next) = self.unloaded(preview).filter(|_| lines.len() < need)
      else {
        return;
      };
      // Unreadable pages show blank.
      let text = self.doc.text(next);
      let more = text.map_or_default(|text| page_lines(&text));
      if let Some(Preview { view: View::Text { lines, starts, .. }, .. }) =
        &mut self.peek
      {
        // A blank row starts each page.
        starts.push(lines.len());
        lines.push(String::new());
        lines.extend(more);
      }
    }
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
    let seen = self.visible_position(position).unwrap_or(false);
    let target =
      annotation.areas.first().map_or(Target::Page(position.page), |area| {
        Target::Point { page: position.page, x: area.x0, y: area.y0 }
      });
    self.peek = Some(Preview {
      target,
      page: position.page,
      source: position,
      serial: crate::graphics::serial(),
      origin: Origin::Key,
      placement: None,
      seen,
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
    let left = self.layout.shown_pan(page) as i32;
    let screen = self.layout.screen;
    let cell_width = i32::from(screen.cell_width);
    let width = ((layout.width - left as f32).min(screen.width() * 0.95)
      as i32
      / cell_width)
      .min(i32::from(screen.columns.saturating_sub(2)))
      * cell_width;
    // The strip runs to the end (U12).
    let last = self.layout_end() - layout.top;
    let height = (self.preview_rows() as f32
      * f32::from(self.layout.screen.cell_height))
    .min(last) as i32;
    anyhow::ensure!(width > 8 && height > 8, "Preview area too small");
    let top = top.clamp(0, (last as i32 - height).max(0));
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

  /// The first page a text preview lacks.
  fn unloaded(&self, preview: &Preview) -> Option<PageId> {
    let View::Text { starts, .. } = &preview.view else { return None };
    let next = preview.page.0 + starts.len();
    (next < self.doc.pages.len()).then_some(PageId(next))
  }

  /// Where the last page ends.
  fn layout_end(&self) -> f32 {
    self.layout.pages.last().map_or(0.0, |last| last.top + last.height)
  }

  /// Rasters the strip `key` names.
  fn preview_image(&mut self, key: TileKey) -> Result<View> {
    let stride = usize::try_from(key.width)? * 3;
    let mut rgb = vec![GAP; stride * usize::try_from(key.height)?];
    let first = self.layout.pages[key.page.0].top as i32;
    let end = key.y + key.height;
    for index in key.page.0..self.layout.pages.len() {
      let page = PageId(index);
      let layout = &self.layout.pages[index];
      // Page rows in strip coordinates.
      let start = layout.top as i32 - first;
      let (from, to) =
        (key.y.max(start), end.min(start + layout.height as i32));
      if from >= end {
        break;
      }
      let pan = self.layout.shown_pan(page) as i32;
      // Wider fitting pages show their middle.
      let wide = layout.width as i32 - key.width;
      let left = match pan {
        _ if index == key.page.0 => key.x,
        0 => (wide / 2).max(0),
        pan => pan,
      };
      let width = key.width.min(layout.width as i32 - left);
      if from >= to || width <= 0 {
        continue;
      }
      let part = TileKey {
        page,
        scale: layout.scale,
        x: left,
        y: from - start,
        width,
        height: to - from,
        ..key
      };
      let area =
        IRect::new(part.x, part.y, part.x + width, part.y + part.height);
      let pixmap = match raster_area(&mut self.doc, &part, area) {
        Ok(pixmap) => pixmap,
        Err(error) if index == key.page.0 => return Err(error),
        // Later failures leave gap rows.
        Err(_) => continue,
      };
      let row = usize::try_from(width)? * 3;
      let offset = usize::try_from(from - key.y)?;
      // Fitting pages centre, as in the view.
      let indent = if pan == 0 { (key.width - width) / 2 } else { 0 };
      let indent = usize::try_from(indent)? * 3;
      let source = usize::try_from(pixmap.stride())?;
      for (line, pixels) in pixmap.samples().chunks(source).enumerate() {
        let at = (offset + line) * stride + indent;
        rgb[at..at + row].copy_from_slice(&pixels[..row]);
      }
    }
    Ok(View::Image { key, rgb })
  }

  pub(crate) fn preview_input(&mut self, input: Input) -> Result<()> {
    let rows = match input.action {
      Action::Motion(Motion::Down) => 1.0,
      Action::Motion(Motion::Up) => -1.0,
      Action::Scroll(amount) => amount * self.preview_rows() as f32,
      Action::Rows(rows) => rows as f32,
      Action::Command => return self.action(input),
      Action::Follow => return self.follow_preview(),
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

  /// Identifies the view: scroll and text top.
  pub(crate) fn view_key(&self) -> (u32, Position) {
    (self.reader.scroll.to_bits(), self.text_top)
  }

  /// Closes the preview, following its link.
  pub(crate) fn follow_preview(&mut self) -> Result<()> {
    let Some(peek) = self.peek.take() else { return Ok(()) };
    if matches!(peek.view, View::Note { .. }) {
      return Ok(());
    }
    self.follow(&peek.target)
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
      View::Text { top, .. } | View::Note { top, .. } => {
        let wanted = top.saturating_add_signed(rows.round() as isize);
        self.extend_text(wanted + visible);
        if let Some(Preview {
          view: View::Text { top, lines, .. } | View::Note { top, lines, .. },
          ..
        }) = &mut self.peek
        {
          *top = wanted.min(lines.len().saturating_sub(visible));
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
      View::Image { key, .. } => {
        let top = self.layout.pages[preview.page.0].top;
        let bottom = top + (key.y + key.height) as f32;
        (
          key.width as usize / usize::from(screen.cell_width),
          (key.height as f32 / cell).ceil() as usize,
          (key.y > 0, bottom < self.layout_end()),
        )
      }
      View::Text { top, lines, .. } => {
        let more = self.unloaded(preview).is_some();
        (
          usize::from(screen.columns).saturating_sub(4),
          self.preview_rows().min(lines.len()),
          (*top > 0, more || *top + self.preview_rows() < lines.len()),
        )
      }
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
    frame: Rect,
  ) -> Result<()> {
    let screen = self.layout.screen;
    let rect = self.cell_bounds(frame);
    self.graphics.backdrop(rect, [255; 4], screen, self.left, out)
  }

  /// Records `frame` and its bridge to `link`.
  fn place_preview(&mut self, frame: Rect, link: Option<Bounds>) {
    let bridge = link.map_or_default(|link| self.bridge(frame, link));
    let view = self.view_key();
    let placement = Placement { area: frame, bridge, view };
    if let Some(peek) = &mut self.peek {
      peek.placement = Some(placement);
    }
  }

  /// Cells from `link`'s edge row to `frame` (U2).
  fn bridge(&self, frame: Rect, link: Bounds) -> Rect {
    let screen = self.layout.screen;
    let (width, height) =
      (f32::from(screen.cell_width), f32::from(screen.cell_height));
    let left = usize::from(self.left);
    let (start, end) = (
      usize::from(frame.column),
      usize::from(frame.column) + usize::from(frame.columns),
    );
    // Touch the frame's nearest column, and its edge row.
    let x0 = (left + (link.x0 / width) as usize).min(end - 1);
    let x1 = (left + (link.x1 / width).ceil() as usize).max(start + 1);
    let below = f32::from(frame.row) * height >= link.y0;
    let (y0, y1) = if below {
      ((link.y1 / height) as usize, usize::from(frame.row) + 1)
    } else {
      let bottom = usize::from(frame.row) + usize::from(frame.rows);
      (bottom - 1, (link.y0 / height).ceil() as usize)
    };
    let cells = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    Rect {
      column: cells(x0),
      row: cells(y0),
      columns: cells(x1.saturating_sub(x0)),
      rows: cells(y1.saturating_sub(y0)),
    }
  }

  /// The page the preview's top row shows.
  fn top_page(&self, preview: &Preview) -> PageId {
    match &preview.view {
      View::Image { key, .. } => {
        let pages = &self.layout.pages;
        let top = pages[key.page.0].top + key.y as f32;
        // Past a page's gap, the next one.
        let index = (key.page.0..pages.len())
          .find(|&index| pages[index].top + pages[index].height > top);
        index.map_or(key.page, PageId)
      }
      View::Text { top, starts, .. } => {
        let later = starts.partition_point(|start| start <= top);
        PageId(preview.page.0 + later.saturating_sub(1))
      }
      View::Note { .. } => preview.page,
    }
  }

  /// Pane cells the drawn preview covers.
  pub fn preview_cells(&self) -> Option<Rect> {
    self.peek.as_ref()?.placement.map(|at| at.area)
  }

  /// The previewed link's destination for status.
  pub(crate) fn preview_label(&mut self) -> Option<String> {
    let peek = self.peek.as_ref().filter(|peek| peek.follows())?;
    let (target, hovered) = (peek.target.clone(), peek.hovered());
    // As on its link: citations show the reference.
    let link = hovered
      .and_then(|(page, index)| Some((page, self.link(page, index).ok()??)));
    if let Some((page, link)) = link {
      return Some(self.link_label(page, &link));
    }
    Some(crate::links::destination(&target, &self.doc, ""))
  }

  /// The hovered link's box; PDF view only.
  fn hovered_box(&mut self) -> Option<Bounds> {
    let (page, index) = self.peek.as_ref()?.hovered()?;
    let link = self.link(page, index).ok().flatten()?;
    let pdf = !self.text_mode;
    pdf.then(|| self.layout.project(page, link.bounds, self.reader.scroll))
  }

  /// Closes it once its source leaves (U17).
  pub fn drop_stray_preview(&mut self) {
    let Some(source) = self.peek.as_ref().map(|peek| peek.source) else {
      return;
    };
    // Unreadable text counts as gone.
    let shown = self.visible_position(source).unwrap_or(false);
    let Some(peek) = &mut self.peek else { return };
    if shown {
      peek.seen = true;
    } else if peek.seen {
      self.peek = None;
    }
  }

  /// Cells `preview` covers beside `cursor`.
  fn preview_frame(&self, preview: &Preview, cursor: Option<Bounds>) -> Rect {
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
    Rect {
      column: cells(column),
      row: cells(top),
      columns: cells(columns + 2),
      rows: cells(rows + 2),
    }
  }

  pub fn draw_preview(&mut self, out: &mut impl Write) -> Result<()> {
    if self.peek.is_none() {
      return Ok(());
    }
    self.scroll_preview(0.0)?;
    let Some(source) = self.peek.as_ref().map(|preview| preview.source) else {
      return Ok(());
    };
    let cursor = self.position_rect(source)?;
    let Some(preview) = &self.peek else { return Ok(()) };
    let frame = self.preview_frame(preview, cursor);
    let screen = self.layout.screen;
    let cell = f32::from(screen.cell_height);
    let (_, _, clipped) = self.preview_extent(preview);
    let (top, column) = (usize::from(frame.row), usize::from(frame.column));
    let rows = usize::from(frame.rows) - 2;
    let columns = usize::from(frame.columns) - 2;
    let shown = self.top_page(preview);
    let label = border_label(self.doc.label(shown), columns, clipped.0);
    let rule = "─".repeat(columns.saturating_sub(label.width()));
    let over_page = matches!(preview.view, View::Image { .. })
      || !self.text_mode && matches!(preview.view, View::Note { .. });
    if over_page {
      self.preview_backdrop(out, frame)?;
    }
    let link = self.hovered_box().or(cursor);
    self.place_preview(frame, link);
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
      View::Text { top: first, lines, .. }
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

/// A page's lines for previews.
fn page_lines(text: &crate::text::Text) -> Vec<String> {
  let lines = text.lines.iter().map(|line| line.text().trim_end().to_owned());
  lines.collect()
}
