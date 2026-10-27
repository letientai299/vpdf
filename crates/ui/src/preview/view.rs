use crate::Result;
use crate::input::InputOwner;
use crate::keys::{Action, Input};
use crate::layout::{Layout, screen_step};
use crate::paint;
use crate::panel::wrap;
use crate::source::{Pages, Raster};
use crate::split::Rect;
use crate::text::{Bounds, PageId, Position};
use crate::theme;
use unicode_width::UnicodeWidthStr;
use vpdf_model::document::{Annotation, Target};
use vpdf_model::input::{
  KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use vpdf_model::scene::{Draw, Layer, Op, Stage, Surface, Text as SceneText};
use vpdf_model::style::Role;
use vpdf_model::tile::TileKey;
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::TextSource;

const HEIGHT: f32 = 0.4;
const ABOVE: f32 = 6.0;
const LOADS: usize = 4;

#[derive(Clone, Copy)]
pub struct Frame<'a> {
  pub layout: &'a Layout,
  pub pane: paint::Frame,
  pub identity: (u32, Position),
  pub cursor: Option<Bounds>,
  pub link: Option<Bounds>,
}

pub enum Event {
  Scroll(f32),
  Follow,
  Dismiss,
  Action(Input),
  Transfer(Input),
  Ignored,
}

enum Route {
  Rows(f32),
  Scroll(f32),
  Screen(i32),
  Follow,
  Dismiss,
  Command,
  Transfer,
  Ignore,
}

fn route(action: Action) -> Route {
  match action {
    Action::Motion(Motion::Down) => Route::Rows(1.0),
    Action::Motion(Motion::Up) => Route::Rows(-1.0),
    Action::Rows(rows) => Route::Rows(rows as f32),
    Action::Scroll(amount) => Route::Scroll(amount),
    Action::Screen(direction) => Route::Screen(direction),
    Action::Follow => Route::Follow,
    Action::Quit | Action::Peek => Route::Dismiss,
    Action::Command => Route::Command,
    _ if crate::input::transfers_overlay(action) => Route::Transfer,
    _ => Route::Ignore,
  }
}

pub enum Press {
  Ignored,
  Follow,
  Split,
  Dismiss { inside: bool },
}

pub struct Preview {
  target: Target,
  page: PageId,
  source: Position,
  serial: vpdf_model::image::PictureId,
  view: View,
  origin: Origin,
  /// Where the last frame drew it.
  placement: Option<Placement>,
  /// Its source has shown since it opened.
  seen: bool,
  /// A note icon it sits beside: page, index, area.
  anchor: Option<(PageId, usize, Bounds)>,
}

/// A drawn preview's cells.
#[derive(Clone, Copy, Debug)]
struct Placement {
  /// The preview, border included.
  area: Rect,
  /// Gap cells to its link.
  bridge: Rect,
  /// The view it was drawn in.
  view: (u32, Position),
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
    rgb: vpdf_model::image::Rgb,
    background: Option<[u8; 4]>,
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

impl View {
  fn image(
    layout: &Layout,
    key: TileKey,
    source: &mut impl Raster,
  ) -> Result<Self> {
    let rgb = super::image(layout, key, source)?;
    let background = image_background(&rgb);
    Ok(Self::Image { key, rgb, background })
  }
}

impl Preview {
  pub fn open(
    target: Target,
    position: Position,
    seen: bool,
    text: bool,
    layout: &Layout,
    source: &mut (impl TextSource + Raster),
  ) -> Result<Self> {
    let (page, x, y) = match &target {
      Target::Uri(_) => anyhow::bail!("External preview target"),
      Target::Page(page) => (*page, 0.0, 0.0),
      Target::Point { page, x, y } => (*page, *x, *y),
    };
    let page = PageId(page.0.min(source.page_count() - 1));
    let view = if text {
      let text = source.page_text(page)?;
      let top = text
        .lines
        .iter()
        .position(|line| line.glyphs.last().is_some_and(|g| g.bounds.y1 >= y))
        .unwrap_or(text.lines.len());
      let top = top.min(text.lines.len().saturating_sub(1));
      View::Text { top, lines: page_lines(&text), starts: vec![0] }
    } else {
      let projected =
        layout.pages()[page.0].matrix.transform_xy(x, y - ABOVE).1;
      let key = preview_key(layout, page, projected as i32)?;
      View::image(layout, key, source)?
    };
    Ok(Self {
      target,
      page,
      source: position,
      serial: vpdf_model::image::PictureId::fresh(),
      view,
      origin: Origin::Key,
      placement: None,
      seen,
      anchor: None,
    })
  }

  pub fn note(
    position: Position,
    annotation: &Annotation,
    seen: bool,
    layout: &Layout,
  ) -> Option<Self> {
    if annotation.contents.is_empty() {
      return None;
    }
    let columns = note_columns(layout);
    let target =
      annotation.areas.first().map_or(Target::Page(position.page), |area| {
        Target::Point { page: position.page, x: area.x0, y: area.y0 }
      });
    Some(Self {
      target,
      page: position.page,
      source: position,
      serial: vpdf_model::image::PictureId::fresh(),
      origin: Origin::Key,
      placement: None,
      seen,
      anchor: None,
      view: View::Note {
        top: 0,
        lines: wrap(&annotation.contents, columns),
        contents: annotation.contents.clone(),
        columns,
      },
    })
  }

  pub fn source(&self) -> Position {
    self.source
  }
  pub fn target(&self) -> &Target {
    &self.target
  }
  pub fn origin(&self) -> Origin {
    self.origin
  }
  pub fn hover(&mut self, page: PageId, index: usize) {
    self.origin = Origin::Hover(page, index);
  }
  pub fn into_target(self) -> Option<Target> {
    self.follows().then_some(self.target)
  }
  pub fn cells(&self) -> Option<Rect> {
    self.placement.map(|at| at.area)
  }
  pub fn anchor(&mut self, page: PageId, index: usize, icon: Bounds) {
    self.anchor = Some((page, index, icon));
    self.seen = false;
  }
  pub fn shown_anchor(&self, text: bool) -> Option<(PageId, usize, Bounds)> {
    self.anchor.filter(|_| !text)
  }
  pub fn keep(&mut self, shown: bool) -> bool {
    if shown {
      self.seen = true;
    }
    shown || !self.seen
  }
  pub fn retains_hover(
    &self,
    at: Option<(u16, u16)>,
    view: (u32, Position),
    rested: Option<(PageId, usize)>,
  ) -> bool {
    let placement = self.placement.filter(|at| at.view == view);
    let on = |cells: Rect| at.is_some_and(|(c, r)| cells.contains(c, r));
    let inside = placement.is_some_and(|at| on(at.area));
    let bridged = placement.is_some_and(|at| on(at.bridge));
    let on_link = rested.is_some_and(|link| self.hovered() == Some(link));
    let elsewhere = rested.is_some() && !on_link;
    inside || on_link || bridged && !elsewhere
  }

  pub fn press(
    &self,
    mouse: MouseEvent,
    inside: bool,
    owner: InputOwner,
  ) -> Press {
    let key = self.origin == Origin::Key;
    if key && owner != InputOwner::Peek {
      return Press::Ignored;
    }
    let alt = mouse.modifiers.contains(KeyModifiers::ALT);
    let button = match mouse.kind {
      MouseEventKind::Down(button) if inside && !alt => Some(button),
      _ => None,
    };
    match button {
      Some(MouseButton::Left) => Press::Follow,
      Some(MouseButton::Middle) if self.follows() => Press::Split,
      _ if key => Press::Ignored,
      _ => Press::Dismiss { inside },
    }
  }

  pub fn allows(action: Action) -> bool {
    !matches!(route(action), Route::Ignore)
  }

  pub fn input(input: Input, layout: &Layout) -> Event {
    let rows = match route(input.action) {
      Route::Rows(rows) => rows,
      Route::Scroll(amount) => amount * preview_rows(layout) as f32,
      Route::Screen(direction) => {
        direction as f32 * screen_step(preview_rows(layout)) as f32
      }
      Route::Follow => return Event::Follow,
      Route::Dismiss => return Event::Dismiss,
      Route::Command => return Event::Action(input),
      Route::Transfer => return Event::Transfer(input),
      Route::Ignore => return Event::Ignored,
    };
    Event::Scroll(rows * input.count as f32)
  }

  pub fn top(&self) -> i32 {
    match &self.view {
      View::Image { key, .. } => key.y,
      View::Text { top, .. } | View::Note { top, .. } => {
        i32::try_from(*top).unwrap_or(i32::MAX)
      }
    }
  }

  fn reflow_note(&mut self, layout: &Layout) {
    let columns = note_columns(layout);
    if let View::Note { contents, columns: old, lines, .. } = &mut self.view
      && *old != columns
    {
      *lines = wrap(contents, columns);
      *old = columns;
    }
  }

  fn unloaded(&self, pages: usize) -> Option<PageId> {
    let View::Text { starts, .. } = &self.view else { return None };
    let next = self.page.0 + starts.len();
    (next < pages).then_some(PageId(next))
  }

  fn extend_text(&mut self, need: usize, source: &mut impl TextSource) {
    for _ in 0..LOADS {
      let View::Text { lines, .. } = &self.view else { return };
      let Some(next) =
        self.unloaded(source.page_count()).filter(|_| lines.len() < need)
      else {
        return;
      };
      let more =
        source.page_text(next).map_or_default(|text| page_lines(&text));
      if let View::Text { lines, starts, .. } = &mut self.view {
        starts.push(lines.len());
        lines.push(String::new());
        lines.extend(more);
      }
    }
  }

  /// False discards an invalidated preview.
  pub fn scroll(
    &mut self,
    rows: f32,
    layout: &Layout,
    source: &mut (impl TextSource + Raster),
  ) -> Result<bool> {
    self.reflow_note(layout);
    let page = self.page;
    if page.0 >= source.page_count() || page.0 >= layout.pages().len() {
      return Ok(false);
    }
    let visible = preview_rows(layout);
    match &self.view {
      View::Text { top, .. } | View::Note { top, .. } => {
        let wanted = top.saturating_add_signed(rows.round() as isize);
        self.extend_text(wanted + visible, source);
        if let View::Text { top, lines, .. } | View::Note { top, lines, .. } =
          &mut self.view
        {
          *top = wanted.min(lines.len().saturating_sub(visible));
        }
      }
      View::Image { key, .. } => {
        let top =
          (key.y as f32 + rows * f32::from(layout.screen().cell_height)) as i32;
        let next = preview_key(layout, page, top)?;
        if *key == next {
          return Ok(true);
        }
        self.view = View::image(layout, next, source)?;
        self.serial = vpdf_model::image::PictureId::fresh();
      }
    }
    Ok(true)
  }
  fn extent(
    &self,
    layout: &Layout,
    pages: usize,
  ) -> (usize, usize, (bool, bool)) {
    let screen = layout.screen();
    let cell = f32::from(screen.cell_height);
    match &self.view {
      View::Image { key, .. } => {
        let top = layout.pages()[self.page.0].top;
        let bottom = top + (key.y + key.height) as f32;
        (
          key.width as usize / usize::from(screen.cell_width),
          (key.height as f32 / cell).ceil() as usize,
          (key.y > 0, bottom < layout_end(layout)),
        )
      }
      View::Text { top, lines, .. } => {
        let more = self.unloaded(pages).is_some();
        (
          usize::from(screen.columns).saturating_sub(4),
          preview_rows(layout).min(lines.len()),
          (*top > 0, more || *top + preview_rows(layout) < lines.len()),
        )
      }
      View::Note { top, lines, columns, .. } => (
        *columns,
        preview_rows(layout).min(lines.len()),
        (*top > 0, *top + preview_rows(layout) < lines.len()),
      ),
    }
  }

  /// The page the preview's top row shows.
  fn top_page(&self, layout: &Layout) -> PageId {
    match &self.view {
      View::Image { key, .. } => {
        let pages = layout.pages();
        let top = pages[key.page.0].top + key.y as f32;
        // Past a page's gap, the next one.
        let index = (key.page.0..pages.len())
          .find(|&index| pages[index].top + pages[index].height > top);
        index.map_or(key.page, PageId)
      }
      View::Text { top, starts, .. } => {
        let later = starts.partition_point(|start| start <= top);
        PageId(self.page.0 + later.saturating_sub(1))
      }
      View::Note { .. } => self.page,
    }
  }

  /// Cells `preview` covers beside `cursor`.
  fn frame(&self, frame: &Frame<'_>, pages: usize) -> Rect {
    let screen = frame.layout.screen();
    let cursor = frame.cursor;
    let cell = f32::from(screen.cell_height);
    let (columns, rows, _) = self.extent(frame.layout, pages);
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
    let column = usize::from(frame.pane.left)
      + usize::from(screen.columns).saturating_sub(columns + 2) / 2;
    let cells = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    Rect {
      column: cells(column),
      row: cells(top),
      columns: cells(columns + 2),
      rows: cells(rows + 2),
    }
  }

  pub fn scene<'a, S: Surface<'a>>(
    &mut self,
    scene: &mut S,
    input: Frame<'_>,
    source: &impl Pages,
  ) -> std::result::Result<(), S::Error> {
    let frame = self.frame(&input, source.count());
    let screen = input.layout.screen();
    let cell = f32::from(screen.cell_height);
    let (_, _, clipped) = self.extent(input.layout, source.count());
    let (top, column) = (usize::from(frame.row), usize::from(frame.column));
    let rows = usize::from(frame.rows) - 2;
    let columns = usize::from(frame.columns) - 2;
    let shown = self.top_page(input.layout);
    let label = border_label(source.label(shown), columns, clipped.0);
    let rule = "─".repeat(columns.saturating_sub(label.width()));
    let over_page = matches!(self.view, View::Image { .. })
      || !input.pane.text && matches!(self.view, View::Note { .. });
    if over_page {
      // Image peeks cover text panes too.
      let pane = paint::Frame { text: false, screen, ..input.pane };
      let color = match &self.view {
        View::Image { background: Some(color), .. } => {
          theme::Paint::Rgba(*color)
        }
        _ => theme::PREVIEW_FILL,
      };
      backdrop(scene, pane, frame, label.width(), color);
    }
    let bridge = input
      .link
      .or(input.cursor)
      .map_or_default(|link| bridge(&input, frame, link));
    self.placement =
      Some(Placement { area: frame, bridge, view: input.identity });
    let border = if over_page { Role::PreviewBorder } else { Role::Current };
    let text_style = if over_page { Role::PreviewText } else { Role::Reset };
    paint::move_to(scene, input.pane.at, top, column);
    scene.push(Op::Style(border));
    scene.text("┌");
    scene.text(&label);
    scene.text(&rule);
    scene.text("┐");
    scene.push(Op::Style(Role::Reset));
    match &self.view {
      View::Image { rgb, .. } => {
        for row in top + 1..=top + rows {
          paint::move_to(scene, input.pane.at, row, column);
          scene.push(Op::Style(border));
          scene.text("│");
          scene.push(Op::Spaces(columns));
          scene.text("│");
          scene.push(Op::Style(Role::Reset));
        }
        let x = (column + 1 - usize::from(input.pane.left)) as f32
          * f32::from(screen.cell_width);
        let y = (top + 1) as f32 * cell;
        let stage = Stage { screen, left: input.pane.left };
        scene.picture(self.serial, rgb, (x, y), stage)?;
      }
      View::Text { top: first, lines, .. }
      | View::Note { top: first, lines, .. } => {
        for (index, line) in lines.iter().skip(*first).take(rows).enumerate() {
          let text = SceneText::fit(line, columns);
          let pad = columns.saturating_sub(text.as_str().width());
          paint::move_to(scene, input.pane.at, top + index + 1, column);
          scene.push(Op::Style(border));
          scene.text("│");
          scene.push(Op::Style(text_style));
          scene.push(Op::Text(text));
          scene.push(Op::Spaces(pad));
          scene.push(Op::Style(border));
          scene.text("│");
        }
      }
    }
    let marker = if clipped.1 { "↓" } else { "─" };
    let rule = "─".repeat(columns.saturating_sub(1));
    paint::move_to(scene, input.pane.at, top + rows + 1, column);
    scene.push(Op::Style(border));
    scene.text("└");
    scene.text(marker);
    scene.text(&rule);
    scene.text("┘");
    scene.push(Op::Style(Role::Reset));
    Ok(())
  }
}
fn backdrop<'a>(
  scene: &mut impl Draw<'a>,
  pane: paint::Frame,
  cells: Rect,
  label: usize,
  color: theme::Paint,
) {
  let mut rect = paint::cell_bounds(pane.screen, pane.left, cells);
  let half_width = f32::from(pane.screen.cell_width) / 2.0;
  let half_height = f32::from(pane.screen.cell_height) / 2.0;
  rect.x0 += half_width;
  rect.x1 -= half_width;
  rect.y0 += half_height;
  rect.y1 -= half_height;
  scene.fill(rect, color, Layer::BelowText, pane.stage());
  // Keep title legible above the rule.
  let title = Bounds {
    x0: rect.x0 + half_width,
    x1: rect.x0 + half_width + label as f32 * half_width * 2.0,
    y0: rect.y0 - half_height,
    y1: rect.y0,
  };
  scene.fill(title, color, Layer::BelowText, pane.stage());
  if label == 0 {
    return;
  }
  let label = Bounds { y1: title.y1 + half_height, ..title };
  for edge in [
    Bounds { y1: label.y0 + 1.0, ..label },
    Bounds { y0: label.y1 - 1.0, ..label },
    Bounds { x1: label.x0 + 1.0, ..label },
    Bounds { x0: label.x1 - 1.0, ..label },
  ] {
    scene.fill(edge, theme::BADGE_FILL, Layer::BelowText, pane.stage());
  }
}

fn image_background(image: &vpdf_model::image::Rgb) -> Option<[u8; 4]> {
  let (width, height) = (image.width() as usize, image.height() as usize);
  let channels = image.pixels().len() / (width * height);
  let edge = (0..width).flat_map(|x| [(x, 0), (x, height - 1)]).chain(
    (1..height.saturating_sub(1)).flat_map(|y| [(0, y), (width - 1, y)]),
  );
  let mut colors = std::collections::BTreeMap::new();
  for (x, y) in edge {
    let at = (y * width + x) * channels;
    let pixel = &image.pixels()[at..at + channels];
    if channels == 4 && pixel[3] != 255 {
      continue;
    }
    let color = [pixel[0], pixel[1], pixel[2], 255];
    *colors.entry(color).or_insert(0usize) += 1;
  }
  colors.into_iter().max_by_key(|&(_, count)| count).map(|(color, _)| color)
}

/// Cells from `link`'s edge row to `frame` (U2).
fn bridge(input: &Frame<'_>, frame: Rect, link: Bounds) -> Rect {
  let screen = input.layout.screen();
  let (width, height) =
    (f32::from(screen.cell_width), f32::from(screen.cell_height));
  let left = usize::from(input.pane.left);
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

fn note_columns(layout: &Layout) -> usize {
  usize::from(layout.screen().columns).saturating_sub(4).min(72)
}

fn preview_rows(layout: &Layout) -> usize {
  ((f32::from(layout.screen().rows) * HEIGHT) as usize)
    .max(3)
    .min(usize::from(layout.screen().rows.saturating_sub(1)))
    .saturating_sub(2)
}

fn preview_key(layout: &Layout, page: PageId, top: i32) -> Result<TileKey> {
  let placed = &layout.pages()[page.0];
  let left = layout.shown_pan(page) as i32;
  let screen = layout.screen();
  let cell_width = i32::from(screen.cell_width);
  let width = ((placed.width - left as f32).min(screen.width() * 0.95) as i32
    / cell_width)
    .min(i32::from(screen.columns.saturating_sub(2)))
    * cell_width;
  // The strip runs to the end (U12).
  let last = layout_end(layout) - placed.top;
  let height = (preview_rows(layout) as f32
    * f32::from(layout.screen().cell_height))
  .min(last) as i32;
  anyhow::ensure!(width > 8 && height > 8, "Preview area too small");
  let top = top.clamp(0, (last as i32 - height).max(0));
  Ok(TileKey {
    page,
    scale: placed.scale,
    rotation: layout.rotation(),
    x: left,
    y: top,
    width,
    height,
    crop: layout.crop_for(page),
    recolor: layout.recolor(),
  })
}

fn layout_end(layout: &Layout) -> f32 {
  layout.pages().last().map_or(0.0, |last| last.top + last.height)
}
fn border_label(label: &str, columns: usize, clipped: bool) -> String {
  let marker = if clipped { "↑" } else { "" };
  if columns < label.width().saturating_add(marker.width() + 4) {
    return vpdf_model::text::clean(marker, columns);
  }
  let label =
    vpdf_model::text::clean(&format!("p{label}"), columns - marker.width() - 3);
  format!(" {label} {marker} ")
}

/// A page's lines for previews.
fn page_lines(text: &crate::text::Text) -> Vec<String> {
  let lines = text.lines.iter().map(|line| line.text().trim_end().to_owned());
  lines.collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  use crate::testing::text_fixture as text;
  use crate::text::Text;
  use std::sync::Arc;

  struct Source {
    count: usize,
    requested: Vec<PageId>,
    rasters: Vec<TileKey>,
  }

  impl TextSource for Source {
    fn page_count(&self) -> usize {
      self.count
    }
    fn page_text(&mut self, page: PageId) -> Result<Arc<Text>> {
      self.requested.push(page);
      anyhow::ensure!(page != PageId(2), "Unreadable test page");
      Ok(text(&[]))
    }
  }

  impl Raster for Source {
    fn raster(
      &mut self,
      key: TileKey,
      visit: impl FnOnce(&[u8], usize) -> Result<()>,
    ) -> Result<()> {
      self.rasters.push(key);
      let stride = key.width as usize * 3;
      visit(&vec![255; stride * key.height as usize], stride)
    }
  }

  #[test]
  fn recolor_refreshes_preview_once() -> Result<()> {
    let screen = crate::testing::screen::cells(80, 30);
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 };
    let mut layout = Layout::new([bounds], screen);
    layout
      .set_recolor(vpdf_model::recolor::Recolor::parse("dark 100").unwrap());
    let mut source =
      Source { count: 1, requested: Vec::new(), rasters: Vec::new() };
    let mut preview = Preview::open(
      Target::Page(PageId(0)),
      Position::default(),
      false,
      false,
      &layout,
      &mut source,
    )?;
    let View::Image { key, rgb, background } = &preview.view else {
      panic!("Missing image")
    };
    assert_eq!(key.recolor, layout.recolor());
    assert_eq!(&rgb.pixels()[..4], [0, 0, 0, 255]);
    assert_eq!(*background, Some([0, 0, 0, 255]));
    let first = preview.serial;
    layout
      .set_recolor(vpdf_model::recolor::Recolor::parse("sepia 100").unwrap());
    preview.scroll(0.0, &layout, &mut source)?;
    assert_ne!(preview.serial, first);
    let View::Image { key, rgb, background } = &preview.view else {
      panic!("Missing image")
    };
    assert_eq!(key.recolor, layout.recolor());
    assert_eq!(&rgb.pixels()[..4], [238, 220, 185, 255]);
    assert_eq!(*background, Some([238, 220, 185, 255]));
    let second = preview.serial;
    let rasters = source.rasters.len();
    preview.scroll(0.0, &layout, &mut source)?;
    assert_eq!(preview.serial, second);
    assert_eq!(source.rasters.len(), rasters);
    Ok(())
  }

  #[test]
  fn backdrop_stays_inside_border() {
    use vpdf_model::scene::Scene;

    let pane = paint::Frame {
      at: vpdf_model::geometry::At { column: 7, row: 3 },
      screen: crate::testing::screen::cells(80, 30),
      left: 4,
      text: false,
    };
    let cells = Rect { column: 10, row: 5, columns: 20, rows: 8 };
    let mut scene = Scene::new();
    backdrop(&mut scene, pane, cells, 3, theme::PREVIEW_FILL);
    let fills: Vec<_> = scene
      .iter()
      .filter_map(|op| {
        let Op::Fill(fill) = op else { panic!("Expected backdrop") };
        assert_eq!(fill.layer(), Layer::BelowText);
        assert_eq!(fill.stage(), pane.stage());
        (fill.color() == theme::PREVIEW_FILL).then(|| fill.rect())
      })
      .collect();
    assert_eq!(
      fills,
      [
        Bounds { x0: 52.0, y0: 88.0, x1: 204.0, y1: 200.0 },
        Bounds { x0: 56.0, y0: 80.0, x1: 80.0, y1: 88.0 },
      ]
    );
    let edges: Vec<_> = scene
      .iter()
      .filter_map(|op| {
        let Op::Fill(fill) = op else { panic!("Expected label border") };
        (fill.color() == theme::BADGE_FILL).then(|| fill.rect())
      })
      .collect();
    assert_eq!(edges.len(), 4);
    for (x, y) in [
      (56.5, 80.5),
      (79.5, 80.5),
      (56.5, 95.5),
      (79.5, 95.5),
      (60.0, 80.5),
      (60.0, 95.5),
    ] {
      assert!(edges.iter().any(|edge| edge.contains(x, y)));
    }
    assert!(edges.iter().all(|edge| !edge.contains(60.0, 88.0)));
    for (x, y) in [(49.0, 100.0), (207.0, 100.0), (100.0, 81.0), (100.0, 207.0)]
    {
      assert!(!fills.iter().any(|fill| fill.contains(x, y)));
    }
  }

  #[test]
  fn labels_identify_page_numbers() {
    assert_eq!(border_label("11", 8, false), " p11  ");
    assert_eq!(border_label("11", 8, true), " p11 ↑ ");
    assert_eq!(border_label("11", 4, true), "↑");
    assert_eq!(border_label("11", 5, true), "↑");
    assert_eq!(border_label("11", 5, false), "");
    assert_eq!(border_label("11", 6, true), "↑");
    assert_eq!(border_label("11", 6, false), " p11  ");
    assert_eq!(border_label("", 0, false), "");
    assert_eq!(border_label("11", 1, true), "↑");
  }

  #[test]
  fn background_ignores_ink() {
    use vpdf_model::image::Rgb;

    for paper in [[255; 4], [25, 35, 45, 255], [240, 220, 190, 255]] {
      for channels in [3, 4] {
        let mut pixels = paper[..channels].repeat(25);
        pixels[..3].fill(0);
        pixels[12 * channels..12 * channels + 3].fill(0);
        let image = if channels == 3 {
          Rgb::new(5, 5, pixels).unwrap()
        } else {
          pixels[9 * channels + 3] = 0;
          Rgb::rgba(5, 5, pixels).unwrap()
        };
        assert_eq!(image_background(&image), Some(paper));
      }
    }
    let blank = Rgb::rgba(2, 2, vec![0; 16]).unwrap();
    assert_eq!(image_background(&blank), None);
  }

  #[test]
  fn colorful_edges_bound_fills() {
    use vpdf_model::image::Rgb;
    use vpdf_model::scene::Scene;

    let pixels: Vec<_> =
      (0..1280 * 16).flat_map(|i| [i as u8, (i / 256) as u8, 1, 255]).collect();
    let image = Rgb::rgba(1280, 16, pixels).unwrap();
    let color = theme::Paint::Rgba(image_background(&image).unwrap());
    let pane = paint::Frame {
      at: vpdf_model::geometry::At::default(),
      screen: crate::testing::screen::cells(200, 30),
      left: 0,
      text: false,
    };
    let cells = Rect { column: 2, row: 2, columns: 162, rows: 3 };
    let mut scene = Scene::new();
    backdrop(&mut scene, pane, cells, 6, color);
    assert_eq!(scene.len(), 6);
    for op in scene.iter().take(2) {
      let Op::Fill(fill) = op else { panic!("Expected background") };
      assert_eq!(fill.color(), color);
    }
  }

  #[test]
  fn missing_pages_keep_loading_bounded() -> Result<()> {
    let screen = crate::testing::screen::cells(80, 30);
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 600.0, y1: 800.0 };
    let layout = Layout::new([bounds; 10], screen);
    let mut source =
      Source { count: 10, requested: Vec::new(), rasters: Vec::new() };
    let mut preview = Preview::open(
      Target::Page(PageId(0)),
      Position::default(),
      false,
      true,
      &layout,
      &mut source,
    )?;
    assert_eq!(source.requested, vec![PageId(0)]);
    for end in [5, 9, 10] {
      assert!(preview.scroll(9999.0, &layout, &mut source)?);
      assert_eq!(source.requested, (0..end).map(PageId).collect::<Vec<_>>());
    }
    assert!(preview.scroll(9999.0, &layout, &mut source)?);
    assert_eq!(source.requested.len(), 10);
    assert!(preview.keep(false), "initially offscreen survives");
    assert!(preview.keep(true));
    assert!(!preview.keep(false), "shown source left view");
    assert_eq!(source.rasters, [] as [TileKey; 0]);
    source.count = 0;
    assert!(
      !preview.scroll(0.0, &layout, &mut source)?,
      "reload invalidates page"
    );
    Ok(())
  }
}
