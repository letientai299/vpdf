use crate::geometry::{PixelRect, Screen};
use crate::image::{PictureId, Rgb};
use crate::style::Role;
use crate::text::{Bounds, Quad};
use crate::theme::{Colors, Paint};
mod polygon;
use crate::tile::TileId;
pub use polygon::Polygon;
use std::borrow::Cow;
use unicode_width::UnicodeWidthChar;

/// Printable text without drawing controls.
///
/// ```compile_fail
/// use vpdf_model::scene::Text;
/// let text = Text("\x1b[2J".into());
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Text<'a>(Cow<'a, str>);

impl<'a> Text<'a> {
  pub fn new(text: impl Into<Cow<'a, str>>) -> Self {
    let text = text.into();
    if text.chars().any(char::is_control) {
      return Self(crate::text::printable(&text).collect::<String>().into());
    }
    Self(text)
  }

  pub fn fit(text: &str, columns: usize) -> Self {
    Self(crate::text::clean(text, columns).into())
  }

  pub fn as_str(&self) -> &str {
    &self.0
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Character {
  value: char,
  columns: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layer {
  BelowText,
  AboveText,
}

/// Pane pixels and their first terminal column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stage {
  pub screen: Screen,
  pub left: u16,
}

/// Pixel coordinates kept as bits for `Eq`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Origin([u32; 2]);

impl Origin {
  fn new((x, y): (f32, f32)) -> Self {
    Self([x.to_bits(), y.to_bits()])
  }

  fn get(self) -> (f32, f32) {
    let [x, y] = self.0.map(f32::from_bits);
    (x, y)
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Fill {
  area: FillArea,
  color: Paint,
  layer: Layer,
  stage: Stage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FillArea {
  Pixels(PixelRect),
  Cells { column: u16, columns: u16, rows: u16 },
}

impl Fill {
  pub fn new(
    rect: Bounds,
    color: impl Into<Paint>,
    layer: Layer,
    stage: Stage,
  ) -> Option<Self> {
    let rect = PixelRect::clip(rect, stage.screen)?;
    Some(Self {
      area: FillArea::Pixels(rect),
      color: color.into(),
      layer,
      stage,
    })
  }

  pub fn stretched(
    color: impl Into<Paint>,
    layer: Layer,
    (column, columns, rows): (u16, u16, u16),
    stage: Stage,
  ) -> Self {
    let area = FillArea::Cells { column, columns, rows };
    Self { area, color: color.into(), layer, stage }
  }

  pub fn area(self) -> FillArea {
    self.area
  }

  pub fn rect(self) -> Bounds {
    match self.area {
      FillArea::Pixels(rect) => rect.bounds(),
      FillArea::Cells { column, columns, rows } => {
        let screen = self.stage.screen;
        let x0 = f32::from(column) * f32::from(screen.cell_width);
        Bounds {
          x0,
          y0: 0.0,
          x1: x0 + f32::from(columns) * f32::from(screen.cell_width),
          y1: f32::from(rows) * f32::from(screen.cell_height),
        }
      }
    }
  }

  pub fn color(self) -> Paint {
    self.color
  }

  pub fn layer(self) -> Layer {
    self.layer
  }

  pub fn stage(self) -> Stage {
    self.stage
  }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TilePlacement {
  id: TileId,
  origin: Origin,
  stage: Stage,
}

impl TilePlacement {
  pub fn new(id: TileId, origin: (f32, f32), stage: Stage) -> Self {
    Self { id, origin: Origin::new(origin), stage }
  }
  pub fn id(self) -> TileId {
    self.id
  }
  /// Page-area pixels before clipping and rounding.
  pub fn origin(self) -> (f32, f32) {
    self.origin.get()
  }
  pub fn visible(self) -> Option<Bounds> {
    let size = (self.id.key.width as f32, self.id.key.height as f32);
    let rect = Bounds::placed(self.origin(), size);
    crate::geometry::clip(rect, self.stage.screen.bounds())
  }
  pub fn stage(self) -> Stage {
    self.stage
  }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Picture {
  id: PictureId,
  image: Rgb,
  origin: Origin,
  stage: Stage,
}

impl Picture {
  pub fn new(
    id: PictureId,
    image: &Rgb,
    origin: (f32, f32),
    stage: Stage,
  ) -> Self {
    Self { id, image: image.clone(), origin: Origin::new(origin), stage }
  }
  pub fn id(&self) -> PictureId {
    self.id
  }
  pub fn image(&self) -> &Rgb {
    &self.image
  }
  pub fn origin(&self) -> (f32, f32) {
    self.origin.get()
  }
  pub fn stage(&self) -> Stage {
    self.stage
  }
}

impl Character {
  pub fn new(value: char) -> Option<Self> {
    if value.is_control() {
      return None;
    }
    let columns = value.width()?;
    if columns > 2 {
      return None;
    }
    Some(Self { value, columns: columns as u8 })
  }

  pub fn value(self) -> char {
    self.value
  }

  pub fn columns(self) -> u8 {
    self.columns
  }
}

/// Ordered cell drawing, independent of transport.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Op<'a> {
  /// Absolute, zero-based cell coordinates.
  Move {
    column: usize,
    row: usize,
  },
  Theme(Colors),
  Style(Role),
  Text(Text<'a>),
  Character(Character),
  /// Paint blanks and advance the cursor.
  Spaces(usize),
  /// Paint blanks without moving the cursor.
  Erase(u16),
  SaveCursor,
  RestoreCursor,
  HideCursor,
  ShowCursor,
  /// Editable row bounds for native composition.
  InputArea {
    masked: bool,
    column: usize,
    columns: usize,
  },
  BlockCursor,
  UnderlineOff,
  Fill(Box<Fill>),
  Polygon(Box<Polygon>),
  /// Page tiles below fills and text.
  Tile(Box<TilePlacement>),
  /// Pictures above fills and text.
  Picture(Box<Picture>),
}

pub type Scene<'a> = Vec<Op<'a>>;

pub trait Draw<'a> {
  fn push(&mut self, op: Op<'a>);

  fn text(&mut self, text: &str) {
    self.push(Op::Text(Text::new(text.to_owned())));
  }

  fn fill(
    &mut self,
    rect: Bounds,
    color: impl Into<Paint>,
    layer: Layer,
    stage: Stage,
  ) {
    if let Some(fill) = Fill::new(rect, color, layer, stage) {
      self.push(Op::Fill(Box::new(fill)));
    }
  }

  fn quad(
    &mut self,
    quad: Quad,
    clip: Bounds,
    color: impl Into<Paint>,
    layer: Layer,
    stage: Stage,
  ) {
    let color = color.into();
    if quad.is_rect() {
      if let Some(rect) = crate::geometry::clip(quad.bounds(), clip) {
        self.fill(rect, color, layer, stage);
      }
    } else if let Some(polygon) = Polygon::new(quad, clip, color, layer, stage)
    {
      self.push(Op::Polygon(Box::new(polygon)));
    }
  }

  fn stretch(
    &mut self,
    color: impl Into<Paint>,
    layer: Layer,
    cells: (u16, u16, u16),
    stage: Stage,
  ) {
    self.push(Op::Fill(Box::new(Fill::stretched(color, layer, cells, stage))));
  }

  fn extend(&mut self, ops: impl IntoIterator<Item = Op<'a>>) {
    for op in ops {
      self.push(op);
    }
  }
}

pub trait Surface<'a>: Draw<'a> {
  type Error;

  fn has(&mut self, id: TileId) -> Result<bool, Self::Error>;

  fn picture(
    &mut self,
    id: PictureId,
    image: &Rgb,
    origin: (f32, f32),
    stage: Stage,
  ) -> Result<(), Self::Error> {
    self.push(Op::Picture(Box::new(Picture::new(id, image, origin, stage))));
    Ok(())
  }

  fn tile(
    &mut self,
    id: TileId,
    origin: (f32, f32),
    stage: Stage,
  ) -> Result<bool, Self::Error> {
    let ready = self.has(id)?;
    self.push(Op::Tile(Box::new(TilePlacement::new(id, origin, stage))));
    Ok(ready)
  }
}

pub struct Recorder<'a, R> {
  scene: Scene<'a>,
  ready: R,
}

impl<'a, R> Recorder<'a, R> {
  pub fn new(ready: R) -> Self {
    Self { scene: Scene::new(), ready }
  }
  pub fn into_scene(self) -> Scene<'a> {
    self.scene
  }
}

impl<'a, R> Draw<'a> for Recorder<'a, R> {
  fn push(&mut self, op: Op<'a>) {
    self.scene.push(op);
  }
}

impl<'a, R: FnMut(TileId) -> bool> Surface<'a> for Recorder<'a, R> {
  type Error = std::convert::Infallible;

  fn has(&mut self, id: TileId) -> Result<bool, Self::Error> {
    Ok((self.ready)(id))
  }
}

impl<'a> Draw<'a> for () {
  fn push(&mut self, _op: Op<'a>) {}

  fn text(&mut self, _text: &str) {}
}

impl<'a> Draw<'a> for Scene<'a> {
  fn push(&mut self, op: Op<'a>) {
    Vec::push(self, op);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tile_visibility_clips_pane_edges() {
    use crate::tile::{TileKey, TileOwner};
    let screen = Screen {
      columns: 4,
      rows: 3,
      cell_width: 10,
      cell_height: 10,
      origin: (7, 3),
    };
    let id = TileId {
      owner: TileOwner::new(1),
      key: TileKey { scale: 1024, width: 16, height: 16, ..TileKey::default() },
    };
    for (origin, area) in [
      ((40.0, 0.0), 0.0),
      ((0.0, 20.0), 0.0),
      ((-16.0, 0.0), 0.0),
      ((-8.2, 15.8), 35.0),
      ((0.0, 0.0), 256.0),
    ] {
      let stage = Stage { screen, left: 2 };
      let visible = TilePlacement::new(id, origin, stage).visible();
      assert_eq!(visible.map_or(0.0, |r| r.width() * r.height()), area);
    }
  }

  #[test]
  fn stretched_fills_keep_geometry() {
    let screen = Screen {
      columns: 5,
      rows: 3,
      cell_width: 4,
      cell_height: 8,
      origin: (2, 1),
    };
    let stage = Stage { screen, left: 3 };
    let fill =
      Fill::stretched([23, 45, 67, 89], Layer::BelowText, (4, 1, 2), stage);
    assert_eq!(fill.area(), FillArea::Cells { column: 4, columns: 1, rows: 2 });
    assert_eq!(fill.rect(), Bounds { x0: 16.0, y0: 0.0, x1: 20.0, y1: 16.0 });
    assert_eq!(fill.color().resolve(Colors::Terminal), [23, 45, 67, 89]);
    assert_eq!(fill.layer(), Layer::BelowText);
    assert_eq!(fill.stage(), stage);
  }

  #[test]
  fn characters_keep_cell_advances() {
    for (value, columns) in [('界', 2), ('e', 1), ('\u{301}', 0)] {
      let character = Character::new(value).unwrap();
      assert_eq!(character.value(), value);
      assert_eq!(character.columns(), columns);
    }
    for value in ['\x1b', '\n', '\t'] {
      assert_eq!(Character::new(value), None);
    }
  }

  #[test]
  fn recording_retains_temporary_text() {
    let mut scene = Scene::new();
    {
      let text = String::from("界\na\x1b");
      Draw::text(&mut scene, &text);
    }
    assert_eq!(scene, [Op::Text(Text::new("界 a"))]);
  }

  #[test]
  fn text_cannot_deliver_controls() {
    for (raw, want) in [
      ("a\x1b[31mb", "a[31mb"),
      ("a\nb\tc", "a b c"),
      ("a\x07\x7fb", "ab"),
      ("界e\u{301}", "界e\u{301}"),
    ] {
      let text = Text::new(raw);
      assert_eq!(text.as_str(), want);
      assert!(!text.as_str().chars().any(char::is_control));
    }
  }

  #[test]
  fn printable_text_stays_borrowed() {
    assert!(matches!(Text::new("界e\u{301}").0, Cow::Borrowed(_)));
  }
}
