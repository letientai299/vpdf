use crate::input::Preedit;
use crate::style::{BACKGROUND, Style};
use eframe::egui::{
  self, Color32, FontId, Painter, Pos2, Rect, TextureHandle, TextureOptions,
  Vec2,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use unicode_width::UnicodeWidthChar;
use vpdf::session::native::Frame;
use vpdf_model::cache::Cache;
use vpdf_model::geometry::Screen;
use vpdf_model::image::PictureId;
use vpdf_model::scene::{Layer, Op, TilePlacement};
use vpdf_model::text::PageId;
use vpdf_model::tile::{TileId, TileKey, TileOwner};

const BUDGET: usize = 64 << 20;
pub const FONT_SIZE: f32 = 16.0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ImageId {
  Tile(TileId),
  Picture(PictureId),
}

struct Texture {
  pixels: Arc<Vec<u8>>,
  handle: TextureHandle,
  size: [usize; 2],
}

pub struct Renderer {
  textures: Cache<ImageId, Texture>,
  pub uploaded: usize,
  pub upload_bytes: usize,
  /// Tiles of the last paint drawn from another scale.
  pub stand_ins: usize,
  /// Tiles of the last paint drawn blank.
  pub missing: usize,
}

impl Default for Renderer {
  fn default() -> Self {
    Self {
      textures: Cache::new(BUDGET),
      uploaded: 0,
      upload_bytes: 0,
      stand_ins: 0,
      missing: 0,
    }
  }
}

impl Renderer {
  pub fn paint(
    &mut self,
    painter: &Painter,
    frame: &Frame,
    screen: Screen,
    scale: f32,
    preedit: Option<&Preedit>,
  ) -> Option<egui::output::IMEOutput> {
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
      let (screen, left) = placement.frame();
      if ready.contains(&id)
        && let Some(texture) = self.textures.get(&ImageId::Tile(id))
      {
        texture.paint(painter, placement.origin(), screen, left, scale);
      } else {
        pending.push(**placement);
      }
    }
    let kept =
      if pending.is_empty() { HashMap::default() } else { self.pages() };
    let covered = pending
      .iter()
      .filter(|placement| stand_in(painter, &kept, **placement, scale))
      .count();
    self.stand_ins = covered;
    self.missing = pending.len() - covered;
    fills(painter, &frame.scene, Layer::BelowText, scale);
    let mut cells = Grid::new(screen);
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
      let (screen, left) = picture.frame();
      texture.paint(painter, picture.origin(), screen, left, scale);
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
      texture.size == size && Arc::ptr_eq(&texture.pixels, pixels)
    }) {
      return;
    }
    let image = egui::ColorImage::from_rgb(size, pixels);
    let handle = painter.ctx().load_texture(
      format!("image {id:?}"),
      image,
      TextureOptions::LINEAR,
    );
    self.textures.insert(
      id,
      Texture { pixels: Arc::clone(pixels), handle, size },
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
    screen: Screen,
    left: u16,
    scale: f32,
  ) {
    let size = Vec2::new(self.size[0] as f32, self.size[1] as f32);
    let rect = placed(screen, left, scale, origin, size);
    image(painter, clip(screen, left, scale), rect, self.handle.id());
  }
}

/// Screen points of an image `size` pixels wide with its top-left at
/// `origin`, rounded down so neighbors meet without a seam.
fn placed(
  screen: Screen,
  left: u16,
  scale: f32,
  origin: (f32, f32),
  size: Vec2,
) -> Rect {
  Rect::from_min_size(
    offset(screen, left, scale)
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
) -> bool {
  let TileId { owner, key } = placement.id();
  let Some(textures) = kept.get(&(owner, key.page)) else { return false };
  let (screen, left) = placement.frame();
  let origin = placement.origin();
  let page = (origin.0 - key.x as f32, origin.1 - key.y as f32);
  let corner = offset(screen, left, scale);
  let size = Vec2::new(key.width as f32, key.height as f32);
  let target = clip(screen, left, scale)
    .intersect(placed(screen, left, scale, origin, size));
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
  !reaching.is_empty()
}

fn image(painter: &Painter, clip: Rect, rect: Rect, texture: egui::TextureId) {
  painter.with_clip_rect(clip).image(
    texture,
    rect,
    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
    Color32::WHITE,
  );
}

fn offset(screen: Screen, left: u16, scale: f32) -> Pos2 {
  Pos2::new(
    (f32::from(screen.origin.0) + f32::from(left))
      * f32::from(screen.cell_width)
      / scale,
    f32::from(screen.origin.1) * f32::from(screen.cell_height) / scale,
  )
}

fn clip(screen: Screen, left: u16, scale: f32) -> Rect {
  Rect::from_min_size(
    offset(screen, left, scale),
    Vec2::new(screen.width(), screen.height()) / scale,
  )
}

fn fills(painter: &Painter, scene: &[Op<'_>], layer: Layer, scale: f32) {
  for op in scene {
    let Op::Fill(fill) = op else { continue };
    if fill.layer() != layer {
      continue;
    }
    let (screen, left) = fill.frame();
    let bounds = fill.rect();
    let min =
      offset(screen, left, scale) + Vec2::new(bounds.x0, bounds.y0) / scale;
    let rect = Rect::from_min_size(
      min,
      Vec2::new(bounds.width(), bounds.height()) / scale,
    );
    painter.with_clip_rect(clip(screen, left, scale)).rect_filled(
      rect,
      0.0,
      Color32::from_rgba_unmultiplied(
        fill.color()[0],
        fill.color()[1],
        fill.color()[2],
        fill.color()[3],
      ),
    );
  }
}

#[derive(Clone, Default)]
struct Cell {
  text: String,
  columns: u8,
  style: Style,
}

struct Grid {
  cells: Vec<Cell>,
  columns: usize,
  cursor: (usize, usize),
  saved: (usize, usize, Style),
  style: Style,
  visible: bool,
  masked: bool,
  input: std::ops::Range<usize>,
  previous: Option<usize>,
}

impl Grid {
  fn new(screen: Screen) -> Self {
    Self {
      cells: vec![
        Cell::default();
        usize::from(screen.columns) * usize::from(screen.rows)
      ],
      columns: usize::from(screen.columns),
      cursor: (0, 0),
      saved: (0, 0, Style::default()),
      style: Style::default(),
      visible: false,
      masked: false,
      input: 0..usize::from(screen.columns),
      previous: None,
    }
  }
  fn op(&mut self, op: &Op<'_>) {
    match op {
      Op::Move { column, row } => {
        self.cursor = (*column, *row);
        self.previous = None;
      }
      Op::Style(role) => self.style.apply(*role),
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
      self.cells[index - 1] = Cell::default();
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
        *cell = Cell::default();
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
    for index in (first + width..end).rev() {
      self.cells[index] = self.cells[index - width].clone();
    }
    if self.cells[end - 1].columns == 2 {
      self.cells[end - 1] = Cell::default();
    }
    self.cells[first..first + width].fill(Cell::default());
    let style = self.style;
    self.style = Style { underline: true, ..Style::default() };
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
  fn paint(&self, painter: &Painter, screen: Screen, scale: f32) {
    let size =
      Vec2::new(f32::from(screen.cell_width), f32::from(screen.cell_height))
        / scale;
    for (index, cell) in self.cells.iter().enumerate() {
      let pos = Pos2::new(
        (index % self.columns) as f32 * size.x,
        (index / self.columns) as f32 * size.y,
      );
      if let Some(color) = cell.style.background {
        painter.rect_filled(
          Rect::from_min_size(
            pos,
            size * Vec2::new(f32::from(cell.columns.max(1)), 1.0),
          ),
          0.0,
          color,
        );
      }
    }
    for (index, cell) in self.cells.iter().enumerate() {
      if cell.text.is_empty() || cell.text == " " {
        continue;
      }
      let pos = Pos2::new(
        (index % self.columns) as f32 * size.x,
        (index / self.columns) as f32 * size.y,
      );
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
    if self.visible && self.cursor.0 < self.columns {
      let pos =
        Pos2::new(self.cursor.0 as f32 * size.x, self.cursor.1 as f32 * size.y);
      painter.rect_filled(
        Rect::from_min_size(pos, size),
        0.0,
        Color32::from_rgba_unmultiplied(216, 222, 233, 100),
      );
    }
  }
}

pub fn background() -> Color32 {
  BACKGROUND
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_model::scene::Text;

  #[test]
  fn textures_reuse_replace_and_retire() {
    use vpdf_model::geometry::Crop;
    use vpdf_model::scene::TilePlacement;
    use vpdf_model::text::PageId;
    use vpdf_model::tile::{TileKey, TileOwner};
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
      key: TileKey {
        page: PageId(0),
        scale: 1024,
        rotation: 0,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        crop: Crop::default(),
      },
    };
    let mut frame = Frame {
      metrics: vpdf::session::native::Metrics::default(),
      content: true,
      screen,
      scene: vec![Op::Tile(Box::new(TilePlacement::new(
        id,
        (-0.25, 0.0),
        screen,
        0,
      )))],
      images: vec![vpdf::session::native::Image {
        id,
        pixels: Arc::new(vec![255, 255, 255]),
      }],
      owners: vec![owner],
      title: String::new(),
      cursor: vpdf_model::style::Shape::Default,
      complete: true,
      quit: false,
      clipboard: None,
      reloaded: Vec::new(),
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
    frame.images.clear();
    frame.owners.clear();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
      renderer.paint(ui.painter(), &frame, screen, 1.0, None);
    });
    output.textures_delta.clear();
    assert_eq!(renderer.textures.used(), 0);
  }

  #[test]
  fn other_scales_stand_in_for_pending_tiles() {
    use vpdf_model::geometry::Crop;
    use vpdf_model::text::PageId;
    use vpdf_model::tile::{TileKey, TileOwner};
    let screen = Screen {
      columns: 40,
      rows: 20,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
    let owner = TileOwner::new(1);
    let key = |page, scale, x, size| TileKey {
      page: PageId(page),
      scale,
      rotation: 0,
      x,
      y: 0,
      width: size,
      height: size,
      crop: Crop::default(),
    };
    let tile = |key| TileId { owner, key };
    let frame = |scene: Vec<TileId>, images: Vec<TileId>| Frame {
      metrics: vpdf::session::native::Metrics::default(),
      content: true,
      screen,
      scene: scene
        .into_iter()
        .map(|id| {
          let origin = (10.0 + id.key.x as f32, 0.0);
          Op::Tile(Box::new(TilePlacement::new(id, origin, screen, 0)))
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
      title: String::new(),
      cursor: vpdf_model::style::Shape::Default,
      complete: false,
      quit: false,
      clipboard: None,
      reloaded: Vec::new(),
    };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    let mut paint = |frame: &Frame| {
      let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        renderer.paint(ui.painter(), frame, screen, 1.0, None);
      });
      output.textures_delta.clear();
      (output.shapes, (renderer.stand_ins, renderer.missing))
    };
    let old = tile(key(0, 1024, 0, 4));
    paint(&frame(vec![old], vec![old]));
    // Twice the scale: the old tile covers the new one's 8 pixels.
    let new = tile(key(0, 2048, 0, 8));
    let other = tile(key(1, 2048, 0, 8));
    let (shapes, counts) = paint(&frame(vec![new, other], Vec::new()));
    assert_eq!(counts, (1, 1));
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
    // A reloaded document's kept textures stand in for nothing.
    let mut reloaded = frame(vec![new, other], Vec::new());
    reloaded.reloaded.push(owner);
    assert_eq!(paint(&reloaded).1, (0, 2));
    assert_eq!(paint(&frame(vec![new], vec![new])).1, (0, 0));
  }

  #[test]
  fn pictures_keep_layer_shape_and_lifetime() {
    use vpdf_model::image::Rgb;
    use vpdf_model::scene::{Draw, Recorder, Surface};
    use vpdf_model::text::Bounds;
    let screen = Screen {
      columns: 4,
      rows: 2,
      cell_width: 10,
      cell_height: 20,
      origin: (2, 1),
    };
    let id = PictureId::fresh();
    let pixels: Arc<Vec<u8>> = Arc::new(vec![255; 6]);
    let mut frame = Frame {
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
    };
    let ctx = egui::Context::default();
    let mut renderer = Renderer::default();
    for (width, height, expected) in [(2, 1, 1), (2, 1, 1), (1, 2, 2)] {
      let image = Rgb::new(width, height, Arc::clone(&pixels)).unwrap();
      let mut recorder = Recorder::new(|_| false);
      recorder.fill(
        Bounds { x0: 0.0, y0: 0.0, x1: 5.0, y1: 5.0 },
        [0, 0, 255, 255],
        Layer::AboveText,
        screen,
        1,
      );
      recorder.text("x");
      recorder.picture(id, &image, (-0.25, 0.0), screen, 1).unwrap();
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
    let screen = Screen {
      columns: 20,
      rows: 2,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
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
    let screen = Screen {
      columns: 8,
      rows: 2,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
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
  fn combining_marks_keep_wide_base() {
    let screen = Screen {
      columns: 4,
      rows: 2,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
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
    let screen = Screen {
      columns: 8,
      rows: 2,
      cell_width: 20,
      cell_height: 40,
      origin: (0, 0),
    };
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
    let screen = Screen {
      columns: 10,
      rows: 2,
      cell_width: 10,
      cell_height: 20,
      origin: (0, 0),
    };
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
