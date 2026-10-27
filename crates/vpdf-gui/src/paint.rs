use crate::input::Preedit;
use crate::style::Style;
use eframe::egui::{
  self, Color32, FontId, Painter, Pos2, Rect, TextureHandle, TextureOptions,
  Vec2,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use unicode_width::UnicodeWidthChar;
use vpdf::session::native::Frame;
use vpdf::trace::Coverage;
use vpdf_model::cache::Cache;
use vpdf_model::geometry::Screen;
use vpdf_model::image::PictureId;
use vpdf_model::scene::{Layer, Op, Stage, TilePlacement};
use vpdf_model::text::PageId;
use vpdf_model::theme::Colors;
use vpdf_model::tile::{TileId, TileKey, TileOwner};

const BUDGET: usize = 64 << 20;
pub const FONT_SIZE: f32 = 16.0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ImageId {
  Tile(TileId),
  Picture(PictureId),
}

struct Texture {
  pixels: Weak<Vec<u8>>,
  handle: Handle,
  size: [usize; 2],
}

enum Handle {
  Managed(TextureHandle),
  Native(crate::upload::Handle),
}

impl Handle {
  fn id(&self) -> egui::TextureId {
    match self {
      Self::Managed(handle) => handle.id(),
      Self::Native(handle) => handle.id(),
    }
  }
}

pub struct Renderer {
  textures: Cache<ImageId, Texture>,
  uploads: Option<crate::upload::Uploads>,
  pub uploaded: usize,
  pub upload_bytes: usize,
  /// Tiles of the last paint.
  pub coverage: Coverage,
  grid: Grid,
}

impl Default for Renderer {
  fn default() -> Self {
    Self {
      textures: Cache::new(BUDGET),
      uploads: None,
      uploaded: 0,
      upload_bytes: 0,
      coverage: Coverage::default(),
      grid: Grid::default(),
    }
  }
}

impl Renderer {
  pub fn staging_capacity(&self) -> usize {
    self.uploads.as_ref().map_or(0, crate::upload::Uploads::capacity)
  }
  pub fn with_gpu(state: eframe::egui_wgpu::RenderState) -> Self {
    Self {
      uploads: Some(crate::upload::Uploads::new(state)),
      ..Self::default()
    }
  }
  pub fn paint(
    &mut self,
    painter: &Painter,
    frame: &Frame,
    screen: Screen,
    scale: f32,
    preedit: Option<&Preedit>,
  ) -> Option<egui::output::IMEOutput> {
    if let Some(uploads) = &mut self.uploads {
      uploads.begin();
    }
    let theme = scene_theme(&frame.scene);
    painter.rect_filled(
      painter.clip_rect(),
      0.0,
      crate::style::rgb(theme.palette().background),
    );
    let ready: HashSet<_> = frame.images.iter().map(|image| image.id).collect();
    let retired: Vec<_> = self
      .textures
      .entries()
      .filter(|(id, _)| match id {
        ImageId::Tile(id) => {
          !frame.owners.contains(&id.owner)
            || frame.reloaded.contains(&id.owner)
        }
        ImageId::Picture(id) => !frame
          .scene
          .iter()
          .any(|op| matches!(op, Op::Picture(picture) if picture.id() == *id)),
      })
      .map(|(id, _)| *id)
      .collect();
    for id in retired {
      self.textures.remove(&id);
    }
    for image in &frame.images {
      let key = image.id.key;
      self.upload(
        painter,
        ImageId::Tile(image.id),
        &image.pixels,
        [key.width as usize, key.height as usize],
      );
    }
    let mut pending = Vec::new();
    for op in &frame.scene {
      let Op::Tile(placement) = op else { continue };
      let id = placement.id();
      if ready.contains(&id)
        && let Some(texture) = self.textures.get(&ImageId::Tile(id))
      {
        texture.paint(painter, placement.origin(), placement.stage(), scale);
      } else if placement.visible().is_some() {
        pending.push(**placement);
      }
    }
    let kept =
      if pending.is_empty() { HashMap::default() } else { self.pages() };
    let coverage = &mut self.coverage;
    *coverage = Coverage::default();
    for placement in pending {
      let Some(visible) = placement.visible() else { continue };
      let area = visible.width() * visible.height();
      let covered = stand_in(painter, &kept, placement, scale).min(area);
      coverage.stand_ins += usize::from(covered > 0.0);
      coverage.missing += usize::from(covered < area);
      coverage.stand_in_pixels += covered;
      coverage.missing_pixels += area - covered;
    }
    fills(painter, &frame.scene, Layer::BelowText, scale);
    let cells = &mut self.grid;
    cells.reset(screen);
    for op in &frame.scene {
      cells.op(op);
    }
    cells.compose(preedit);
    cells.paint(painter, screen, scale);
    let ime = cells.ime(screen, scale);
    fills(painter, &frame.scene, Layer::AboveText, scale);
    for op in &frame.scene {
      let Op::Picture(picture) = op else { continue };
      let image = picture.image();
      let id = ImageId::Picture(picture.id());
      self.upload(
        painter,
        id,
        image.pixels(),
        [image.width() as usize, image.height() as usize],
      );
      let Some(texture) = self.textures.get(&id) else { continue };
      texture.paint(painter, picture.origin(), picture.stage(), scale);
    }
    if let Some(uploads) = &mut self.uploads {
      uploads.finish();
    }
    ime
  }
  /// Kept tile textures by owner and page.
  fn pages(&self) -> HashMap<(TileOwner, PageId), Vec<Kept>> {
    let mut pages: HashMap<_, Vec<_>> = HashMap::new();
    for (id, texture) in self.textures.entries() {
      let ImageId::Tile(id) = id else { continue };
      pages.entry((id.owner, id.key.page)).or_default().push(Kept {
        key: id.key,
        texture: texture.handle.id(),
        size: texture.size,
      });
    }
    pages
  }
  fn upload(
    &mut self,
    painter: &Painter,
    id: ImageId,
    pixels: &Arc<Vec<u8>>,
    size: [usize; 2],
  ) {
    if self.textures.get(&id).is_some_and(|texture| {
      texture.size == size
        && std::ptr::eq(texture.pixels.as_ptr(), Arc::as_ptr(pixels))
    }) {
      return;
    }
    let handle = if let (ImageId::Tile(_), Some(uploads)) =
      (id, &mut self.uploads)
      && crate::upload::fits(size)
    {
      let Some(handle) = uploads.upload(pixels, size) else {
        self.textures.remove(&id);
        painter
          .ctx()
          .request_repaint_after(std::time::Duration::from_millis(1));
        return;
      };
      Handle::Native(handle)
    } else {
      let image = if pixels.len() == size[0] * size[1] * 4 {
        egui::ColorImage::from_rgba_unmultiplied(size, pixels)
      } else {
        egui::ColorImage::from_rgb(size, pixels)
      };
      Handle::Managed(painter.ctx().load_texture(
        format!("image {id:?}"),
        image,
        TextureOptions::LINEAR,
      ))
    };
    self.textures.insert(
      id,
      Texture { pixels: Arc::downgrade(pixels), handle, size },
      size[0] * size[1] * 4,
    );
    self.uploaded += 1;
    self.upload_bytes += size[0] * size[1] * 4;
  }
}

impl Texture {
  fn paint(
    &self,
    painter: &Painter,
    origin: (f32, f32),
    stage: Stage,
    scale: f32,
  ) {
    let size = Vec2::new(self.size[0] as f32, self.size[1] as f32);
    let rect = placed(stage, scale, origin, size);
    image(painter, clip(stage, scale), rect, self.handle.id());
  }
}

/// Screen points of an image `size` pixels wide with its top-left at
/// `origin`, rounded down so neighbors meet without a seam.
fn placed(stage: Stage, scale: f32, origin: (f32, f32), size: Vec2) -> Rect {
  Rect::from_min_size(
    offset(stage, scale)
      + Vec2::new(origin.0.floor(), origin.1.floor()) / scale,
    size / scale,
  )
}

/// A kept texture that can stand in for a tile at another scale.
struct Kept {
  key: TileKey,
  texture: egui::TextureId,
  size: [usize; 2],
}

/// Covers a tile still rendering with kept textures of its page at
/// other scales, nearest last, as after a resize or zoom.
fn stand_in(
  painter: &Painter,
  kept: &HashMap<(TileOwner, PageId), Vec<Kept>>,
  placement: TilePlacement,
  scale: f32,
) -> f32 {
  let TileId { owner, key } = placement.id();
  let Some(textures) = kept.get(&(owner, key.page)) else { return 0.0 };
  let stage = placement.stage();
  let origin = placement.origin();
  let page = (origin.0 - key.x as f32, origin.1 - key.y as f32);
  let corner = offset(stage, scale);
  let size = Vec2::new(key.width as f32, key.height as f32);
  let target = clip(stage, scale).intersect(placed(stage, scale, origin, size));
  let rect = |kept: &Kept| {
    let ratio = key.scale as f32 / kept.key.scale as f32;
    Rect::from_min_size(
      corner
        + Vec2::new(
          page.0 + kept.key.x as f32 * ratio,
          page.1 + kept.key.y as f32 * ratio,
        ) / scale,
      Vec2::new(kept.size[0] as f32, kept.size[1] as f32) * ratio / scale,
    )
  };
  let mut reaching: Vec<_> = textures
    .iter()
    .filter(|kept| {
      kept.key.rotation == key.rotation
        && kept.key.crop == key.crop
        && kept.key.scale != key.scale
    })
    .map(|kept| (kept, rect(kept)))
    .filter(|(_, rect)| rect.intersects(target))
    .collect();
  let distance =
    |other: u32| (f64::from(other) / f64::from(key.scale)).ln().abs();
  // Farther scales fill gaps the nearest one leaves.
  reaching.sort_by(|(a, _), (b, _)| {
    distance(b.key.scale).total_cmp(&distance(a.key.scale))
  });
  for (kept, rect) in &reaching {
    image(painter, target, *rect, kept.texture);
  }
  let areas: Vec<_> = reaching
    .iter()
    .map(|(_, rect)| {
      let rect = rect.intersect(target);
      vpdf_model::text::Bounds {
        x0: rect.min.x,
        y0: rect.min.y,
        x1: rect.max.x,
        y1: rect.max.y,
      }
    })
    .collect();
  vpdf_model::geometry::union_area(&areas) * scale * scale
}

fn image(painter: &Painter, clip: Rect, rect: Rect, texture: egui::TextureId) {
  painter.with_clip_rect(clip).image(
    texture,
    rect,
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
    Color32::WHITE,
  );
}

fn offset(Stage { screen, left }: Stage, scale: f32) -> Pos2 {
  Pos2::new(
    (f32::from(screen.origin.0) + f32::from(left))
      * f32::from(screen.cell_width)
      / scale,
    f32::from(screen.origin.1) * f32::from(screen.cell_height) / scale,
  )
}

fn clip(stage: Stage, scale: f32) -> Rect {
  let screen = stage.screen;
  Rect::from_min_size(
    offset(stage, scale),
    Vec2::new(screen.width(), screen.height()) / scale,
  )
}

fn fills(painter: &Painter, scene: &[Op<'_>], layer: Layer, scale: f32) {
  let mut theme = Colors::Terminal;
  for op in scene {
    if let Op::Theme(selected) = op {
      theme = *selected;
    }
    let fill = match op {
      Op::Fill(fill) => **fill,
      Op::Polygon(polygon) => polygon.fill(),
      _ => continue,
    };
    if fill.layer() != layer {
      continue;
    }
    let stage = fill.stage();
    let origin = offset(stage, scale);
    let point = |(x, y): (f32, f32)| origin + Vec2::new(x, y) / scale;
    let color = fill.color().resolve(theme);
    let color =
      Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]);
    let painter = painter.with_clip_rect(clip(stage, scale));
    if let Op::Polygon(polygon) = op {
      let bounds = polygon.clip();
      let clip = Rect::from_min_max(
        point((bounds.x0, bounds.y0)),
        point((bounds.x1, bounds.y1)),
      );
      painter.with_clip_rect(clip).add(egui::Shape::convex_polygon(
        polygon.quad().0.map(point).to_vec(),
        color,
        egui::Stroke::NONE,
      ));
    } else {
      let bounds = fill.rect();
      painter.rect_filled(
        Rect::from_min_max(
          point((bounds.x0, bounds.y0)),
          point((bounds.x1, bounds.y1)),
        ),
        0.0,
        color,
      );
    }
  }
}

#[derive(Clone, Default)]
struct Cell {
  text: String,
  columns: u8,
  style: Style,
}

impl Cell {
  /// Blanks the cell, keeping its text allocation.
  fn clear(&mut self) {
    self.text.clear();
    self.columns = 0;
    self.style = Style::default();
  }
}

#[derive(Default)]
struct Grid {
  cells: Vec<Cell>,
  /// Rows with cells written since the last reset.
  dirty: Vec<bool>,
  columns: usize,
  cursor: (usize, usize),
  saved: (usize, usize, Style),
  style: Style,
  theme: Colors,
  visible: bool,
  masked: bool,
  input: std::ops::Range<usize>,
  previous: Option<usize>,
}

impl Grid {
  #[cfg(test)]
  fn new(screen: Screen) -> Self {
    let mut grid = Self::default();
    grid.reset(screen);
    grid
  }
  /// Blanks the grid for `screen`, keeping its storage.
  fn reset(&mut self, screen: Screen) {
    let columns = usize::from(screen.columns);
    let mut cells = std::mem::take(&mut self.cells);
    let mut dirty = std::mem::take(&mut self.dirty);
    if columns == self.columns {
      let rows = cells.chunks_mut(columns.max(1)).zip(&dirty);
      for (row, _) in rows.filter(|(_, dirty)| **dirty) {
        row.iter_mut().for_each(Cell::clear);
      }
    } else {
      cells.iter_mut().for_each(Cell::clear);
    }
    cells.resize_with(columns * usize::from(screen.rows), Cell::default);
    dirty.clear();
    dirty.resize(usize::from(screen.rows), false);
    *self =
      Self { cells, dirty, columns, input: 0..columns, ..Self::default() };
  }
  /// Marks the cursor row for the next reset and paint.
  fn touch(&mut self) {
    if let Some(dirty) = self.dirty.get_mut(self.cursor.1) {
      *dirty = true;
    }
  }
  fn op(&mut self, op: &Op<'_>) {
    if matches!(
      op,
      Op::Text(_) | Op::Character(_) | Op::Spaces(_) | Op::Erase(_)
    ) {
      self.touch();
    }
    match op {
      Op::Move { column, row } => {
        self.cursor = (*column, *row);
        self.previous = None;
      }
      Op::Theme(theme) => {
        self.theme = *theme;
        self.style = Style::new(*theme);
      }
      Op::Style(role) => self.style.apply(*role, self.theme),
      Op::Text(text) => {
        for ch in text.as_str().chars() {
          self.character(ch, ch.width().unwrap_or(0) as u8);
        }
      }
      Op::Character(ch) => self.character(ch.value(), ch.columns()),
      Op::Spaces(count) => {
        for _ in 0..*count {
          self.character(' ', 1);
        }
      }
      Op::Erase(count) => {
        let cursor = self.cursor;
        for _ in 0..*count {
          self.character(' ', 1);
        }
        self.cursor = cursor;
      }
      Op::SaveCursor => self.saved = (self.cursor.0, self.cursor.1, self.style),
      Op::RestoreCursor => {
        self.cursor = (self.saved.0, self.saved.1);
        self.previous = None;
        self.style = self.saved.2;
      }
      Op::HideCursor => self.visible = false,
      Op::ShowCursor => self.visible = true,
      Op::InputArea { column, columns, masked } => {
        self.masked = *masked;
        self.input = *column..column.saturating_add(*columns).min(self.columns);
      }
      Op::UnderlineOff => self.style.underline = false,
      _ => {}
    }
  }
  fn character(&mut self, ch: char, columns: u8) {
    if columns == 0 {
      if let Some(cell) =
        self.previous.and_then(|index| self.cells.get_mut(index))
      {
        cell.text.push(ch);
      }
      return;
    }
    if self.cursor.0 + usize::from(columns) > self.columns {
      return;
    }
    let Some(index) = self
      .cursor
      .1
      .checked_mul(self.columns)
      .and_then(|index| index.checked_add(self.cursor.0))
    else {
      return;
    };
    if self.cursor.0 > 0
      && self.cells.get(index - 1).is_some_and(|cell| cell.columns == 2)
    {
      self.cells[index - 1].clear();
    }
    self.previous = Some(index);
    if let Some(cell) = self.cells.get_mut(index) {
      cell.text.clear();
      cell.text.push(ch);
      cell.columns = columns;
      cell.style = self.style;
    }
    for next in 1..usize::from(columns) {
      if let Some(cell) = self.cells.get_mut(index + next) {
        cell.clear();
      }
    }
    self.cursor.0 += usize::from(columns);
  }
  fn compose(&mut self, preedit: Option<&Preedit>) {
    let Some(preedit) = preedit.filter(|_| self.visible) else { return };
    let masked;
    let preedit = if self.masked {
      masked = Preedit {
        text: "*".repeat(preedit.text.chars().count()),
        active: preedit.active.clone(),
      };
      &masked
    } else {
      preedit
    };
    let start = self.cursor;
    if self.columns == 0
      || !self.input.contains(&start.0)
      || start.1 >= self.cells.len() / self.columns
    {
      return;
    }
    let width: usize = preedit
      .text
      .chars()
      .filter(|ch| !ch.is_control())
      .map(|ch| ch.width().unwrap_or(0))
      .sum();
    let width = width.min(self.input.end - start.0);
    let first = start.1 * self.columns + start.0;
    let end = start.1 * self.columns + self.input.end;
    self.touch();
    self.cells[first..end].rotate_right(width);
    if self.cells[end - 1].columns == 2 {
      self.cells[end - 1].clear();
    }
    self.cells[first..first + width].iter_mut().for_each(Cell::clear);
    let style = self.style;
    self.style = Style { underline: true, ..Style::new(self.theme) };
    self.previous = None;
    let caret = preedit.active.as_ref().map_or(usize::MAX, |range| range.end);
    let mut cursor = None;
    for (index, ch) in preedit.text.chars().enumerate() {
      if index == caret {
        cursor = Some(self.cursor);
      }
      if !ch.is_control() {
        let width = ch.width().unwrap_or(0) as u8;
        if self.cursor.0 + usize::from(width) > self.input.end {
          break;
        }
        self.character(ch, width);
      }
    }
    self.cursor = cursor.unwrap_or(self.cursor);
    self.cursor.0 = self.cursor.0.min(self.input.end.saturating_sub(1));
    self.cursor.1 = start.1;
    self.style = style;
  }
  fn ime(&self, screen: Screen, scale: f32) -> Option<egui::output::IMEOutput> {
    if !self.visible
      || !self.input.contains(&self.cursor.0)
      || self.cursor.1 >= usize::from(screen.rows)
    {
      return None;
    }
    let size = egui::vec2(
      f32::from(screen.cell_width) / scale,
      f32::from(screen.cell_height) / scale,
    );
    let pos =
      egui::pos2(self.cursor.0 as f32 * size.x, self.cursor.1 as f32 * size.y);
    Some(egui::output::IMEOutput {
      purpose: if self.masked {
        egui::IMEPurpose::Password
      } else {
        egui::IMEPurpose::Normal
      },
      rect: Rect::from_min_size(
        egui::pos2(self.input.start as f32 * size.x, pos.y),
        egui::vec2(self.input.len() as f32 * size.x, size.y),
      ),
      cursor_rect: Rect::from_min_size(pos, egui::vec2(1.0, size.y)),
      should_interrupt_composition: false,
    })
  }
  /// Cells of dirty rows at their positions for cells of `size`.
  fn written(&self, size: Vec2) -> impl Iterator<Item = (Pos2, &Cell)> {
    let rows = self.cells.chunks(self.columns.max(1)).enumerate();
    rows.zip(&self.dirty).filter(|(_, dirty)| **dirty).flat_map(
      move |((row, cells), _)| {
        let y = row as f32 * size.y;
        cells.iter().enumerate().map(move |(column, cell)| {
          (Pos2::new(column as f32 * size.x, y), cell)
        })
      },
    )
  }
  fn paint(&self, painter: &Painter, screen: Screen, scale: f32) {
    let size =
      Vec2::new(f32::from(screen.cell_width), f32::from(screen.cell_height))
        / scale;
    for (pos, cell) in self.written(size) {
      let Some(color) = cell.style.background else { continue };
      let mut rect = Rect::from_min_size(
        pos,
        size * Vec2::new(f32::from(cell.columns.max(1)), 1.0),
      );
      if pos.y >= f32::from(screen.rows.saturating_sub(1)) * size.y {
        rect.max.y = rect.max.y.max(painter.clip_rect().bottom());
        if rect.max.x >= f32::from(screen.columns) * size.x {
          rect.max.x = rect.max.x.max(painter.clip_rect().right());
        }
      }
      painter.rect_filled(rect, 0.0, color);
    }
    let mut blocks = egui::Mesh::default();
    for (pos, cell) in self.written(size) {
      if cell.text.is_empty() || cell.text == " " {
        continue;
      }
      let eighths = match cell.text.as_str() {
        "█" => 8,
        "▉" => 7,
        "▊" => 6,
        "▋" => 5,
        "▌" => 4,
        "▍" => 3,
        "▎" => 2,
        "▏" => 1,
        _ => 0,
      };
      if eighths > 0 {
        // Mesh edges avoid glyph gaps and feathering.
        blocks.add_colored_rect(
          Rect::from_min_size(
            pos,
            Vec2::new(size.x * eighths as f32 / 8.0, size.y),
          ),
          cell.style.foreground,
        );
        continue;
      }
      let font = FontId::monospace(FONT_SIZE);
      let galley = painter.layout_job(egui::text::LayoutJob::single_section(
        cell.text.clone(),
        egui::TextFormat {
          font_id: font,
          color: cell.style.foreground,
          italics: cell.style.italic,
          ..Default::default()
        },
      ));
      painter.galley(pos, Arc::clone(&galley), cell.style.foreground);
      if cell.style.bold {
        painter.galley(
          pos + Vec2::new(0.35, 0.0),
          galley,
          cell.style.foreground,
        );
      }
      if cell.style.underline {
        painter.line_segment(
          [
            pos + Vec2::new(0.0, size.y - 1.0),
            pos + Vec2::new(size.x, size.y - 1.0),
          ],
          (1.0, cell.style.foreground),
        );
      }
    }
    if !blocks.is_empty() {
      painter.add(blocks);
    }
    if self.visible && self.cursor.0 < self.columns {
      let pos =
        Pos2::new(self.cursor.0 as f32 * size.x, self.cursor.1 as f32 * size.y);
      painter.rect_filled(Rect::from_min_size(pos, size), 0.0, {
        let [r, g, b] = self.theme.palette().foreground;
        Color32::from_rgba_unmultiplied(r, g, b, 100)
      });
    }
  }
}

pub fn scene_theme(scene: &[Op<'_>]) -> Colors {
  scene
    .iter()
    .find_map(|op| match op {
      Op::Theme(theme) => Some(*theme),
      _ => None,
    })
    .unwrap_or_default()
}

pub fn background() -> Color32 {
  crate::style::rgb(Colors::Terminal.palette().background)
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::scene::Text;

  #[test]
  fn progress_blocks_fill_cells() {
    let screen = crate::screen::pixels(16, 2, 11, 19);
    for scale in [1.0, 1.5, 2.0] {
      let mut grid = Grid::new(screen);
      grid.op(&Op::Move { column: 0, row: 1 });
      grid.op(&Op::Text(Text::new("██▉▊▋▌▍▎▏· 57%")));
      let ctx = egui::Context::default();
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        grid.paint(ui.painter(), screen, scale);
      });
      let meshes: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
          egui::Shape::Mesh(mesh) => Some(mesh),
          _ => None,
        })
        .collect();
      assert_eq!(meshes.len(), 1);
      let mesh = meshes[0];
      assert!(mesh.is_valid());
      assert_eq!(mesh.indices.len(), 9 * 6);
      assert_eq!(mesh.vertices.len(), 9 * 4);
      let size = Vec2::new(11.0, 19.0) / scale;
      for (column, (vertices, eighths)) in mesh
        .vertices
        .as_chunks::<4>()
        .0
        .iter()
        .zip([8, 8, 7, 6, 5, 4, 3, 2, 1])
        .enumerate()
      {
        let left = column as f32 * size.x;
        let right = left + size.x * eighths as f32 / 8.0;
        assert_eq!(vertices[0].pos, Pos2::new(left, size.y));
        assert_eq!(vertices[1].pos, Pos2::new(right, size.y));
        assert_eq!(vertices[2].pos, Pos2::new(left, size.y * 2.0));
        assert_eq!(vertices[3].pos, Pos2::new(right, size.y * 2.0));
        assert!(vertices.iter().all(|vertex| {
          vertex.color == grid.cells[16 + column].style.foreground
            && vertex.uv == egui::epaint::WHITE_UV
        }));
      }
      assert_eq!(mesh.vertices[1].pos, mesh.vertices[4].pos);
      assert_eq!(mesh.vertices[5].pos, mesh.vertices[8].pos);
      let text: String = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
          egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
          _ => None,
        })
        .collect();
      assert_eq!(text, "·57%");
      output.textures_delta.clear();
    }
  }

  #[test]
  fn status_fills_bottom_remainder() {
    use vpdf_model::style::Role;
    let screen = crate::screen::pixels(2, 2, 11, 19);
    for scale in [1.0, 1.5, 2.0] {
      let mut grid = Grid::new(screen);
      grid.op(&Op::Style(Role::Bar));
      grid.op(&Op::Spaces(2));
      grid.op(&Op::Move { column: 0, row: 1 });
      grid.op(&Op::Style(Role::StatusFile));
      grid.op(&Op::Spaces(1));
      grid.op(&Op::Style(Role::Bar));
      grid.op(&Op::Spaces(1));
      let bounds =
        Rect::from_min_size(Pos2::ZERO, Vec2::new(27.0, 45.0) / scale);
      let ctx = egui::Context::default();
      let mut output = ctx.run_ui(
        egui::RawInput { screen_rect: Some(bounds), ..Default::default() },
        |ui| grid.paint(&ui.painter().with_clip_rect(bounds), screen, scale),
      );
      let backgrounds: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
          egui::Shape::Rect(rect) => Some(rect),
          _ => None,
        })
        .collect();
      assert_eq!(backgrounds.len(), 4);
      for (column, rect) in backgrounds[..2].iter().enumerate() {
        assert_eq!(
          rect.rect,
          Rect::from_min_size(
            Pos2::new(column as f32 * 11.0 / scale, 0.0),
            Vec2::new(11.0, 19.0) / scale,
          )
        );
      }
      for (column, rect) in backgrounds[2..].iter().enumerate() {
        assert_eq!(rect.rect.top(), 19.0 / scale);
        assert_eq!(rect.rect.bottom(), bounds.bottom());
        assert_eq!(rect.fill, grid.cells[2 + column].style.background.unwrap());
      }
      assert_eq!(backgrounds[2].rect.right(), backgrounds[3].rect.left());
      assert_eq!(backgrounds[3].rect.right(), bounds.right());
      output.textures_delta.clear();
    }
  }

  fn blank(screen: Screen) -> Frame {
    Frame {
      metrics: vpdf::session::native::Metrics::default(),
      content: true,
      screen,
      scene: Vec::new(),
      images: Vec::new(),
      owners: Vec::new(),
      title: String::new(),
      cursor: vpdf_model::style::Shape::Default,
      complete: true,
      quit: false,
      clipboard: None,
      reloaded: Vec::new(),
    }
  }

  #[test]
  fn themed_clear_preserves_tiles() {
    use vpdf_model::style::Role;
    let screen = Screen {
      columns: 4,
      rows: 2,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
    let owner = TileOwner::new(1);
    let id = TileId {
      owner,
      key: TileKey { scale: 1024, width: 20, height: 20, ..TileKey::default() },
    };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    for &theme in &[
      Colors::Terminal,
      Colors::Palette(vpdf_model::theme::Palette::light()),
      Colors::Palette(vpdf_model::theme::Palette::dark()),
    ] {
      let frame = Frame {
        scene: vec![
          Op::Theme(theme),
          Op::Style(Role::Reset),
          Op::Move { column: 0, row: 0 },
          Op::Erase(4),
          Op::Move { column: 0, row: 1 },
          Op::Erase(4),
          Op::Tile(Box::new(TilePlacement::new(
            id,
            (0.0, 0.0),
            Stage { screen, left: 0 },
          ))),
          Op::Style(Role::Bar),
          Op::Text(Text::new(":")),
          Op::Spaces(3),
        ],
        images: vec![vpdf::session::native::Image {
          id,
          pixels: Arc::new(vec![255; 20 * 20 * 3]),
        }],
        owners: vec![owner],
        ..blank(screen)
      };
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        renderer.paint(ui.painter(), &frame, screen, 1.0, None);
      });
      output.textures_delta.clear();
      let texture = renderer.textures.get(&ImageId::Tile(id)).unwrap();
      let tile = output
        .shapes
        .iter()
        .position(|shape| shape.shape.texture_id() == texture.handle.id())
        .unwrap();
      let covers = |point| {
        output.shapes[tile + 1..].iter().any(|shape| {
          matches!(&shape.shape, egui::Shape::Rect(rect)
          if rect.fill.a() > 0 && rect.rect.contains(point))
        })
      };
      assert!(!covers(Pos2::new(5.0, 5.0)), "{theme:?}: tile covered");
      assert!(covers(Pos2::new(5.0, 25.0)), "{theme:?}: bar transparent");
    }
  }

  #[test]
  fn textures_reuse_replace_and_retire() {
    use vpdf_model::scene::TilePlacement;
    use vpdf_model::tile::{TileKey, TileOwner};
    let screen = crate::screen::pixels(4, 2, 10, 20);
    let owner = TileOwner::new(1);
    let id = TileId {
      owner,
      key: TileKey { scale: 1024, width: 1, height: 1, ..TileKey::default() },
    };
    let mut frame = Frame {
      scene: vec![Op::Tile(Box::new(TilePlacement::new(
        id,
        (-0.25, 0.0),
        Stage { screen, left: 0 },
      )))],
      images: vec![vpdf::session::native::Image {
        id,
        pixels: Arc::new(vec![255, 255, 255]),
      }],
      owners: vec![owner],
      ..blank(screen)
    };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    for (index, expected) in [1, 1, 2, 2].into_iter().enumerate() {
      if index == 2 {
        frame.images[0].pixels = Arc::new(vec![0, 0, 0]);
      }
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        renderer.paint(ui.painter(), &frame, screen, 1.0, None);
      });
      output.textures_delta.clear();
      assert_eq!(renderer.uploaded, expected);
      assert_eq!(renderer.upload_bytes, expected * 4);
    }
    let released = Arc::downgrade(&frame.images[0].pixels);
    frame.images.clear();
    assert!(released.upgrade().is_none());
    assert_eq!(renderer.textures.used(), 4);
    frame.owners.clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      renderer.paint(ui.painter(), &frame, screen, 1.0, None);
    });
    output.textures_delta.clear();
    assert_eq!(renderer.textures.used(), 0);
  }

  #[test]
  fn other_scales_stand_in_for_pending_tiles() {
    use vpdf_model::text::PageId;
    use vpdf_model::tile::{TileKey, TileOwner};
    let screen = crate::screen::pixels(40, 20, 10, 20);
    let owner = TileOwner::new(1);
    let key = |page, scale, x, size| TileKey {
      page: PageId(page),
      scale,
      x,
      width: size,
      height: size,
      ..TileKey::default()
    };
    let tile = |key| TileId { owner, key };
    let frame = |scene: Vec<TileId>, images: Vec<TileId>| Frame {
      scene: scene
        .into_iter()
        .map(|id| {
          let origin = (10.0 + id.key.x as f32, 0.0);
          let stage = Stage { screen, left: 0 };
          Op::Tile(Box::new(TilePlacement::new(id, origin, stage)))
        })
        .collect(),
      images: images
        .into_iter()
        .map(|id| vpdf::session::native::Image {
          id,
          pixels: Arc::new(vec![
            255;
            (id.key.width * id.key.height * 3) as usize
          ]),
        })
        .collect(),
      owners: vec![owner],
      complete: false,
      ..blank(screen)
    };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    let mut paint = |frame: &Frame| {
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        renderer.paint(ui.painter(), frame, screen, 1.0, None);
      });
      output.textures_delta.clear();
      let coverage = renderer.coverage;
      (
        output.shapes,
        (coverage.stand_ins, coverage.missing),
        (coverage.stand_in_pixels, coverage.missing_pixels),
      )
    };
    let old = tile(key(0, 1024, 0, 4));
    paint(&frame(vec![old], vec![old]));
    // Twice the scale: the old tile covers the new one's 8 pixels.
    let new = tile(key(0, 2048, 0, 8));
    let other = tile(key(1, 2048, 0, 8));
    let (shapes, counts, areas) = paint(&frame(vec![new, other], Vec::new()));
    assert_eq!(counts, (1, 1));
    assert_eq!(areas, (64.0, 64.0));
    let image = shapes
      .iter()
      .find_map(|shape| match &shape.shape {
        egui::Shape::Mesh(mesh) => Some((mesh.calc_bounds(), shape.clip_rect)),
        _ => None,
      })
      .expect("a stand-in image");
    assert_eq!(
      image.0,
      Rect::from_min_size(Pos2::new(10.0, 0.0), Vec2::splat(8.0))
    );
    assert_eq!(image.1.min, Pos2::new(10.0, 0.0));
    assert_eq!(image.1.max, Pos2::new(18.0, 8.0));
    let overlapping = tile(key(0, 512, 0, 2));
    paint(&frame(vec![overlapping], vec![overlapping]));
    let partial = tile(key(0, 2048, 0, 16));
    let (_, counts, areas) = paint(&frame(vec![partial], Vec::new()));
    assert_eq!(counts, (1, 1));
    assert_eq!(areas, (64.0, 192.0));
    let offscreen = tile(key(0, 2048, 1000, 16));
    let (_, counts, areas) = paint(&frame(vec![offscreen], Vec::new()));
    assert_eq!(counts, (0, 0));
    assert_eq!(areas, (0.0, 0.0));
    let edge = tile(key(1, 2048, 385, 16));
    assert_eq!(paint(&frame(vec![edge], Vec::new())).2, (0.0, 80.0));
    // A reloaded document's kept textures stand in for nothing.
    let mut reloaded = frame(vec![new, other], Vec::new());
    reloaded.reloaded.push(owner);
    assert_eq!(paint(&reloaded).1, (0, 2));
    assert_eq!(paint(&frame(vec![new], vec![new])).1, (0, 0));
  }

  #[test]
  fn transparent_previews_follow_palette() {
    use vpdf_model::image::Rgb;
    use vpdf_model::scene::{Draw, Recorder, Surface};
    use vpdf_model::theme::{Overlay, Paint, Palette};
    let screen = crate::screen::pixels(4, 2, 10, 20);
    let id = PictureId::fresh();
    let image = Rgb::rgba(2, 1, vec![0, 0, 0, 0, 255, 255, 255, 255]).unwrap();
    let stage = Stage { screen, left: 0 };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    let mut custom = Palette::dark();
    custom.background = [0x12, 0x34, 0x56];
    for palette in [Palette::light(), Palette::dark(), custom] {
      let mut frame = blank(screen);
      let mut scene = Recorder::new(|_| false);
      scene.push(Op::Theme(Colors::Palette(palette)));
      scene.fill(
        vpdf_model::text::Bounds { x0: 0.0, y0: 0.0, x1: 20.0, y1: 20.0 },
        Paint::Overlay(Overlay::Preview),
        Layer::BelowText,
        stage,
      );
      scene.picture(id, &image, (0.0, 0.0), stage).unwrap();
      frame.scene = scene.into_scene();
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        renderer.paint(ui.painter(), &frame, screen, 1.0, None);
      });
      assert!(output.shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(rect)
          if rect.fill == crate::style::rgb(palette.background))
      }));
      let texture = renderer.textures.get(&ImageId::Picture(id)).unwrap();
      if let Some(delta) = output
        .textures_delta
        .set
        .get(&texture.handle.id())
        .and_then(|deltas| deltas.first())
      {
        let egui::ImageData::Color(pixels) = &delta.image;
        assert_eq!(pixels.pixels, [Color32::TRANSPARENT, Color32::WHITE]);
      }
      assert_eq!(renderer.uploaded, 1);
      output.textures_delta.clear();
    }
  }

  #[test]
  fn pictures_keep_layer_shape_and_lifetime() {
    use vpdf_model::image::Rgb;
    use vpdf_model::scene::{Draw, Recorder, Surface};
    use vpdf_model::text::Bounds;
    let screen =
      Screen { origin: (2, 1), ..crate::screen::pixels(4, 2, 10, 20) };
    let id = PictureId::fresh();
    let pixels: Arc<Vec<u8>> = Arc::new(vec![255; 6]);
    let mut frame = blank(screen);
    let stage = Stage { screen, left: 1 };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    for (width, height, expected) in [(2, 1, 1), (2, 1, 1), (1, 2, 2)] {
      let image = Rgb::new(width, height, Arc::clone(&pixels)).unwrap();
      let mut recorder = Recorder::new(|_| false);
      recorder.fill(
        Bounds { x0: 0.0, y0: 0.0, x1: 5.0, y1: 5.0 },
        [0, 0, 255, 255],
        Layer::AboveText,
        stage,
      );
      recorder.text("x");
      recorder.picture(id, &image, (-0.25, 0.0), stage).unwrap();
      frame.scene = recorder.into_scene();
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        renderer.paint(ui.painter(), &frame, screen, 2.0, None);
      });
      output.textures_delta.clear();
      assert_eq!(renderer.uploaded, expected);
      assert_eq!(renderer.upload_bytes, expected * pixels.len() / 3 * 4);
      let texture = renderer.textures.get(&ImageId::Picture(id)).unwrap();
      let last = output.shapes.last().unwrap();
      assert_eq!(last.shape.texture_id(), texture.handle.id());
      assert_eq!(last.clip_rect.min, Pos2::new(15.0, 10.0));
      assert_eq!(texture.size, [width as usize, height as usize]);
    }
    frame.scene.clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      renderer.paint(ui.painter(), &frame, screen, 2.0, None);
    });
    output.textures_delta.clear();
    assert_eq!(renderer.textures.used(), 0);
    assert_eq!(Arc::strong_count(&pixels), 1);
  }

  #[test]
  fn password_preedit_stays_masked() {
    let screen = crate::screen::pixels(20, 2, 10, 20);
    let mut grid = Grid::new(screen);
    grid.op(&Op::InputArea { column: 0, columns: 20, masked: true });
    grid.op(&Op::ShowCursor);
    grid.compose(Some(&Preedit {
      text: "章a\u{301}".into(),
      active: Some(0..2),
    }));
    let text: String =
      grid.cells.iter().map(|cell| cell.text.as_str()).collect();
    assert_eq!(text, "***");
    assert_eq!(grid.cursor, (2, 0));
    assert_eq!(
      grid.ime(screen, 1.0).unwrap().purpose,
      egui::IMEPurpose::Password
    );
  }

  #[test]
  fn underline_removal_preserves_style() {
    use vpdf_model::style::Role;
    let screen = crate::screen::pixels(8, 2, 10, 20);
    for role in [Role::Hover, Role::HoverDim, Role::TextLink, Role::BarHover] {
      let mut grid = Grid::new(screen);
      grid.op(&Op::Style(role));
      let mut expected = grid.style;
      assert!(expected.underline);
      expected.underline = false;
      grid.op(&Op::UnderlineOff);
      grid.op(&Op::Text(Text::new("x")));
      assert_eq!(grid.cells[0].style, expected);
      grid.op(&Op::Style(Role::Reset));
      assert_eq!(grid.style, Style::default());
    }
  }

  #[test]
  fn reset_blanks_reused_cells() {
    let blank = |grid: &Grid| {
      grid.cells.iter().all(|cell| cell.text.is_empty() && cell.columns == 0)
        && !grid.dirty.contains(&true)
    };
    let mut grid = Grid::new(crate::screen::pixels(4, 3, 10, 20));
    for screen in [(4, 3), (4, 2), (4, 3), (3, 3), (5, 1)] {
      grid.op(&Op::Move { column: 1, row: 1 });
      grid.op(&Op::Style(vpdf_model::style::Role::Bar));
      grid.op(&Op::Text(Text::new("章x")));
      grid.op(&Op::Move { column: 0, row: 0 });
      grid.op(&Op::Text(Text::new("a")));
      grid.op(&Op::Text(Text::new("\u{301}")));
      grid.op(&Op::Erase(2));
      grid.op(&Op::Move { column: 0, row: 9 });
      grid.op(&Op::Spaces(2));
      grid.op(&Op::Move { column: 0, row: grid.dirty.len() - 1 });
      grid.op(&Op::ShowCursor);
      grid.compose(Some(&Preedit { text: "章".into(), active: None }));
      let rows = grid.cells.chunks(grid.columns).zip(&grid.dirty);
      for (row, dirty) in rows {
        assert!(*dirty || row.iter().all(|cell| cell.text.is_empty()));
      }
      grid.reset(crate::screen::pixels(screen.0, screen.1, 10, 20));
      assert!(blank(&grid), "{screen:?}");
      assert_eq!(grid.cells.len(), usize::from(screen.0 * screen.1));
      assert_eq!(grid.input, 0..usize::from(screen.0));
    }
  }

  #[test]
  fn combining_marks_keep_wide_base() {
    let screen = crate::screen::pixels(4, 2, 10, 20);
    let mut grid = Grid::new(screen);
    grid.op(&Op::Text(Text::new("章\u{301}a\u{301}")));
    assert_eq!(grid.cells[0].text, "章\u{301}");
    assert_eq!(grid.cells[2].text, "a\u{301}");
    assert_eq!(grid.cursor, (3, 0));
    grid.op(&Op::Move { column: 0, row: 1 });
    grid.op(&Op::Text(Text::new("\u{301}")));
    assert_eq!(grid.cells[3].text, "");
  }

  #[test]
  fn composition_places_scaled_caret_and_shifts_suffix() {
    let screen = crate::screen::pixels(8, 2, 20, 40);
    let mut grid = Grid::new(screen);
    grid.op(&Op::Move { column: 1, row: 1 });
    grid.op(&Op::Text(Text::new("suffix")));
    grid.op(&Op::Move { column: 1, row: 1 });
    assert!(grid.ime(screen, 2.0).is_none());
    grid.op(&Op::ShowCursor);
    grid.compose(Some(&Preedit {
      text: "章a\u{301}".into(),
      active: Some(1..3),
    }));
    assert_eq!(grid.cells[1 + 8].text, "章");
    assert_eq!(grid.cells[3 + 8].text, "a\u{301}");
    assert!(grid.cells[1 + 8].style.underline);
    assert_eq!(grid.cells[4 + 8].text, "s");
    let ime = grid.ime(screen, 2.0).unwrap();
    assert_eq!(ime.cursor_rect.min, egui::pos2(40.0, 20.0));
    assert_eq!(
      ime.rect,
      Rect::from_min_size(egui::pos2(0.0, 20.0), egui::vec2(80.0, 20.0))
    );
    grid.op(&Op::Move { column: 7, row: 1 });
    grid.compose(Some(&Preedit { text: "章x".into(), active: None }));
    assert_eq!(grid.cursor, (7, 1));
    assert_eq!(grid.cells[15].text, "");
  }

  #[test]
  fn composition_stays_inside_split_prompt() {
    let screen = crate::screen::pixels(10, 2, 10, 20);
    let mut grid = Grid::new(screen);
    grid.op(&Op::Move { column: 0, row: 1 });
    grid.op(&Op::Text(Text::new("left right")));
    grid.op(&Op::Move { column: 2, row: 1 });
    grid.op(&Op::InputArea { column: 0, columns: 4, masked: false });
    grid.op(&Op::ShowCursor);
    grid.compose(Some(&Preedit { text: "章章".into(), active: None }));
    assert_eq!(grid.cells[12].text, "章");
    assert_eq!(grid.cells[15].text, "r");
    assert_eq!(grid.cursor, (3, 1));
    assert_eq!(grid.ime(screen, 1.0).unwrap().rect.width(), 40.0);
  }
}

#[cfg(test)]
mod polygon_tests {
  use super::*;
  use vpdf_model::scene::Draw;
  use vpdf_model::text::{Bounds, Quad};

  #[test]
  fn native_polygons_clip() {
    let screen =
      Screen { origin: (2, 3), ..crate::screen::pixels(10, 8, 10, 10) };
    let bounds = Bounds { x0: 10.25, y0: 8.5, x1: 80.75, y1: 60.5 };
    let quad = Quad([(0.0, 30.0), (90.0, 10.0), (90.0, 30.0), (0.0, 50.0)]);
    let stage = Stage { screen, left: 1 };
    let mut scene = Vec::new();
    scene.quad(quad, bounds, [20, 80, 160, 90], Layer::AboveText, stage);
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      fills(ui.painter(), &scene, Layer::BelowText, 2.0);
      fills(ui.painter(), &scene, Layer::AboveText, 2.0);
    });
    output.textures_delta.clear();
    let polygons: Vec<_> = output
      .shapes
      .iter()
      .filter(|shape| matches!(shape.shape, egui::Shape::Path(_)))
      .collect();
    assert_eq!(polygons.len(), 1);
    let shape = polygons[0];
    let egui::Shape::Path(path) = &shape.shape else { unreachable!() };
    assert_eq!(path.points.len(), 4);
    let origin = offset(stage, 2.0);
    assert_eq!(path.points[0], origin + Vec2::new(0.0, 30.0) / 2.0);
    let expected = Rect::from_min_max(
      origin + Vec2::new(bounds.x0, bounds.y0) / 2.0,
      origin + Vec2::new(bounds.x1, bounds.y1) / 2.0,
    );
    assert_eq!(shape.clip_rect, expected);
  }
}
