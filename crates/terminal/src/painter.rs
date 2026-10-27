use crate::graphics::Graphics;
use std::io::{self, Write};
use vpdf_model::scene::{Draw, FillArea, Layer, Op, Stage, Surface, Text};
use vpdf_model::theme::{Colors, Paint};
use vpdf_model::{text::Bounds, tile::TileId};

pub fn paint(out: &mut impl Write, scene: &[Op<'_>]) -> io::Result<()> {
  let mut painter = Painter::new(out);
  painter.paint(scene);
  painter.finish()
}

/// Call finish to report output errors.
pub struct Painter<'out, W> {
  out: &'out mut W,
  error: Option<io::Error>,
  theme: Colors,
  graphics: Option<&'out mut Graphics>,
}

impl<'out, W: Write> Painter<'out, W> {
  pub fn new(out: &'out mut W) -> Self {
    Self { out, error: None, graphics: None, theme: Colors::Terminal }
  }

  #[must_use]
  pub fn with_graphics(mut self, graphics: &'out mut Graphics) -> Self {
    self.graphics = Some(graphics);
    self
  }

  pub fn graphics(&mut self) -> Option<&mut Graphics> {
    self.graphics.as_deref_mut()
  }

  /// Reports errors before the next component draws.
  pub fn checkpoint(&mut self) -> io::Result<()> {
    self.error.take().map_or(Ok(()), Err)
  }

  pub fn paint(&mut self, scene: &[Op<'_>]) {
    for op in scene {
      self.output(|out, graphics, theme| command(out, op, graphics, theme));
    }
  }

  pub fn finish(mut self) -> io::Result<()> {
    self.checkpoint()
  }

  fn output<F>(&mut self, paint: F)
  where
    F: FnOnce(&mut W, Option<&mut Graphics>, &mut Colors) -> io::Result<()>,
  {
    if self.error.is_none() {
      self.error =
        paint(self.out, self.graphics.as_deref_mut(), &mut self.theme).err();
    }
  }

  fn ready(&mut self) -> crate::Result<(&mut W, &mut Graphics)> {
    if let Some(error) = self.error.take() {
      return Err(error.into());
    }
    let graphics = self.graphics.as_deref_mut().ok_or_else(missing_graphics)?;
    Ok((self.out, graphics))
  }
}

impl<'a, W: Write> Draw<'a> for Painter<'_, W> {
  fn push(&mut self, op: Op<'a>) {
    self.output(|out, graphics, theme| command(out, &op, graphics, theme));
  }

  fn text(&mut self, text: &str) {
    self.output(|out, graphics, theme| {
      command(out, &Op::Text(Text::new(text)), graphics, theme)
    });
  }

  fn fill(
    &mut self,
    rect: Bounds,
    color: impl Into<Paint>,
    layer: Layer,
    stage: Stage,
  ) {
    let color = color.into();
    self.output(|out, graphics, theme| {
      paint_fill(out, graphics, rect, resolve(color, *theme), layer, stage)
    });
  }

  fn stretch(
    &mut self,
    color: impl Into<Paint>,
    layer: Layer,
    cells: (u16, u16, u16),
    stage: Stage,
  ) {
    let color = color.into();
    self.output(|out, graphics, theme| {
      paint_stretch(out, graphics, resolve(color, *theme), layer, cells, stage)
    });
  }
}

impl<W: Write> Surface<'_> for Painter<'_, W> {
  type Error = anyhow::Error;

  fn has(&mut self, id: TileId) -> crate::Result<bool> {
    let (_, graphics) = self.ready()?;
    Ok(graphics.has(&(id.owner.get(), id.key)))
  }

  fn picture(
    &mut self,
    id: vpdf_model::image::PictureId,
    image: &vpdf_model::image::Rgb,
    origin: (f32, f32),
    stage: Stage,
  ) -> crate::Result<()> {
    let (out, graphics) = self.ready()?;
    picture(graphics, out, id, image, origin, stage)
  }

  fn tile(
    &mut self,
    id: TileId,
    origin: (f32, f32),
    Stage { screen, left }: Stage,
  ) -> crate::Result<bool> {
    let (out, graphics) = self.ready()?;
    graphics.place_at((id.owner.get(), id.key), origin, screen, left, out)
  }
}

fn resolve(paint: Paint, theme: Colors) -> [u8; 4] {
  let theme = match theme {
    Colors::Terminal => Colors::Palette(
      crate::caps::get().map_or_default(crate::caps::Caps::palette),
    ),
    theme @ Colors::Palette(_) => theme,
  };
  paint.resolve(theme)
}

fn command(
  out: &mut impl Write,
  op: &Op<'_>,
  mut graphics: Option<&mut Graphics>,
  theme: &mut Colors,
) -> io::Result<()> {
  match op {
    Op::Theme(selected) => {
      *theme = *selected;
      crate::style::write(out, vpdf_model::style::Role::Reset, *theme)?;
    }
    Op::Move { column, row } => {
      let row = position(*row)?;
      let column = position(*column)?;
      write!(out, "\x1b[{row};{column}H")?;
    }
    Op::Style(role) => crate::style::write(out, *role, *theme)?,
    Op::Text(text) => out.write_all(text.as_str().as_bytes())?,
    Op::Character(character) => {
      let mut bytes = [0; 4];
      out.write_all(character.value().encode_utf8(&mut bytes).as_bytes())?;
    }
    Op::Spaces(count) => spaces(out, *count)?,
    Op::Erase(0) | Op::InputArea { .. } => {}
    Op::Erase(count) => write!(out, "\x1b[{count}X")?,
    Op::SaveCursor => out.write_all(b"\x1b7")?,
    Op::RestoreCursor => out.write_all(b"\x1b8")?,
    Op::HideCursor => {
      out.write_all(crate::terminal::HIDE_CURSOR.as_bytes())?;
    }
    Op::ShowCursor => out.write_all(b"\x1b[?25h")?,
    Op::BlockCursor => out.write_all(b"\x1b[2 q")?,
    Op::UnderlineOff => out.write_all(b"\x1b[24m")?,
    Op::Picture(placement) => {
      graphics_output(graphics, |graphics| {
        picture(
          graphics,
          out,
          placement.id(),
          placement.image(),
          placement.origin(),
          placement.stage(),
        )
      })?;
    }
    Op::Tile(placement) => {
      let id = placement.id();
      let Stage { screen, left } = placement.stage();
      graphics_output(graphics, |graphics| {
        let key = (id.owner.get(), id.key);
        graphics.place_at(key, placement.origin(), screen, left, out)
      })?;
    }
    Op::Polygon(polygon) => {
      let fill = polygon.fill();
      for rect in polygon.strips() {
        paint_fill(
          out,
          graphics.as_deref_mut(),
          rect,
          resolve(fill.color(), *theme),
          fill.layer(),
          fill.stage(),
        )?;
      }
    }
    Op::Fill(fill) => match fill.area() {
      FillArea::Pixels(_) => paint_fill(
        out,
        graphics,
        fill.rect(),
        resolve(fill.color(), *theme),
        fill.layer(),
        fill.stage(),
      )?,
      FillArea::Cells { column, columns, rows } => paint_stretch(
        out,
        graphics,
        resolve(fill.color(), *theme),
        fill.layer(),
        (column, columns, rows),
        fill.stage(),
      )?,
    },
  }
  Ok(())
}

fn picture(
  graphics: &mut Graphics,
  out: &mut impl Write,
  id: vpdf_model::image::PictureId,
  image: &vpdf_model::image::Rgb,
  (x, y): (f32, f32),
  Stage { screen, left }: Stage,
) -> crate::Result<()> {
  let size = (image.width() as f32, image.height() as f32);
  let rect = Bounds::placed((x, y), size);
  graphics.picture(
    id.get(),
    (image.width(), image.height(), image.pixels()),
    rect,
    screen,
    left,
    out,
  )
}

fn paint_fill(
  out: &mut impl Write,
  graphics: Option<&mut Graphics>,
  rect: Bounds,
  color: [u8; 4],
  layer: Layer,
  Stage { screen, left }: Stage,
) -> io::Result<()> {
  graphics_output(graphics, |graphics| match layer {
    Layer::BelowText => graphics.backdrop(rect, color, screen, left, out),
    Layer::AboveText => graphics.highlight(rect, color, screen, left, out),
  })
}

fn paint_stretch(
  out: &mut impl Write,
  graphics: Option<&mut Graphics>,
  color: [u8; 4],
  layer: Layer,
  cells: (u16, u16, u16),
  Stage { screen, left }: Stage,
) -> io::Result<()> {
  let z = match layer {
    Layer::BelowText => -1,
    Layer::AboveText => 1,
  };
  graphics_output(graphics, |graphics| {
    graphics.stretch(color, z, cells, screen, left, out)
  })
}

fn graphics_output<T>(
  graphics: Option<&mut Graphics>,
  paint: impl FnOnce(&mut Graphics) -> crate::Result<T>,
) -> io::Result<T> {
  let graphics = graphics.ok_or_else(missing_graphics)?;
  paint(graphics).map_err(output_error)
}

fn missing_graphics() -> io::Error {
  io::Error::new(io::ErrorKind::Unsupported, "Graphics painter missing")
}

fn output_error(error: anyhow::Error) -> io::Error {
  error.downcast::<io::Error>().unwrap_or_else(io::Error::other)
}

fn position(cell: usize) -> io::Result<usize> {
  cell.checked_add(1).ok_or_else(|| {
    io::Error::new(io::ErrorKind::InvalidInput, "Cell position overflow")
  })
}

fn spaces(out: &mut impl Write, mut count: usize) -> io::Result<()> {
  const BLANKS: [u8; 128] = [b' '; 128];
  while count > 0 {
    let chunk = count.min(BLANKS.len());
    out.write_all(&BLANKS[..chunk])?;
    count -= chunk;
  }
  Ok(())
}

#[cfg(test)]
#[path = "../../../tests/support/replay.rs"]
mod replay;

#[cfg(test)]
mod tests {
  use super::replay::{framed, replay_frame};
  use super::*;
  use vpdf_model::geometry::Screen;
  use vpdf_model::scene::Text;
  use vpdf_model::style::Role;

  /// Every transport, crop, and offset combination.
  fn profiles() -> impl Iterator<Item = crate::graphics::Presentation> {
    (0..8).map(|bits| crate::graphics::Presentation {
      transport: crate::terminal::Transport { tmux: bits & 1 != 0 },
      crop_quirk: bits & 2 != 0,
      cell_offsets: bits & 4 != 0,
      report_missing: false,
    })
  }

  #[test]
  fn rgb_surfaces_use_palettes() {
    use vpdf_model::theme::{Overlay, Palette};
    for theme in [
      Colors::Terminal,
      Colors::Palette(Palette::light()),
      Colors::Palette(Palette::dark()),
    ] {
      let preview = resolve(Paint::Overlay(Overlay::Preview), theme);
      assert_eq!(preview[..3], theme.palette().background);
      assert_eq!(resolve([1, 2, 3, 4].into(), theme), [1, 2, 3, 4]);
    }
  }

  #[test]
  fn pictures_keep_protocol_bytes() -> crate::Result<()> {
    use crate::payload::Encoder;
    use vpdf_model::image::{PictureId, Rgb};
    use vpdf_model::scene::{Picture, Recorder};
    let image = Rgb::new(24, 24, vec![255; 24 * 24 * 3]).unwrap();
    let id = PictureId::fresh();
    let screen = Screen { origin: (2, 1), ..crate::screen::pixels(5, 3, 4, 8) };
    let stage = Stage { screen, left: 3 };
    for profile in profiles() {
      let mut direct = Graphics::new(profile, &Encoder::default());
      let mut stream = Graphics::new(profile, &Encoder::default());
      let mut replay = Graphics::new(profile, &Encoder::default());
      for origin in [(-0.25, -1.25), (-0.25, -1.25), (2.0, 3.0)] {
        let mut expected = Vec::new();
        direct.begin(&mut expected)?;
        direct.picture(
          id.get(),
          (24, 24, image.pixels()),
          Bounds {
            x0: origin.0,
            y0: origin.1,
            x1: origin.0 + 24.0,
            y1: origin.1 + 24.0,
          },
          screen,
          3,
          &mut expected,
        )?;
        direct.end(&mut expected)?;
        let actual = framed(&mut stream, |painter| {
          painter.picture(id, &image, origin, stage)
        })?;
        assert_eq!(actual, expected);
        let mut recorder = Recorder::new(|_| false);
        recorder.picture(id, &image, origin, stage).unwrap();
        let scene = recorder.into_scene();
        assert_eq!(
          scene,
          vec![Op::Picture(Box::new(Picture::new(id, &image, origin, stage)))]
        );
        assert_eq!(replay_frame(&mut replay, &scene)?, expected);
      }
    }
    Ok(())
  }

  #[test]
  fn tiles_keep_protocol_bytes() -> crate::Result<()> {
    use crate::payload::{Encoder, Tile};
    use vpdf_model::scene::TilePlacement;
    use vpdf_model::tile::{TileKey, TileOwner};
    let screen = Screen { origin: (2, 1), ..crate::screen::pixels(5, 3, 4, 8) };
    let key =
      TileKey { scale: 1024, width: 24, height: 24, ..TileKey::default() };
    let payload = Encoder::default().encode(&[255; 24 * 24 * 3])?;
    let id = TileId { owner: TileOwner::new(1), key };
    let missing = TileId { owner: TileOwner::new(2), key };
    let stage = Stage { screen, left: 3 };
    let placement = TilePlacement::new(id, (-0.25, -1.25), stage);
    let absent = TilePlacement::new(missing, (1.0, 2.0), stage);
    let scene = [Op::Tile(Box::new(placement)), Op::Tile(Box::new(absent))];
    for profile in profiles() {
      let mut direct = Graphics::new(profile, &Encoder::default());
      let mut streaming = Graphics::new(profile, &Encoder::default());
      let mut replay = Graphics::new(profile, &Encoder::default());
      for graphics in [&mut direct, &mut streaming, &mut replay] {
        graphics
          .keep(1, Tile { key, payload: payload.clone(), render_ms: 0.0 });
      }
      for _ in 0..2 {
        let mut expected = Vec::new();
        direct.begin(&mut expected)?;
        assert!(direct.place_at(
          (1, key),
          placement.origin(),
          screen,
          3,
          &mut expected
        )?);
        assert!(!direct.place_at(
          (2, key),
          absent.origin(),
          screen,
          3,
          &mut expected
        )?);
        direct.end(&mut expected)?;
        assert_eq!(direct.missing_tiles, 1);
        let actual = framed(&mut streaming, |painter| {
          assert!(painter.has(id)?);
          assert!(!painter.has(missing)?);
          assert!(painter.tile(id, placement.origin(), stage)?);
          assert!(!painter.tile(missing, absent.origin(), stage)?);
          Ok(())
        })?;
        assert_eq!(streaming.missing_tiles, 1);
        assert_eq!(actual, expected);
        assert_eq!(replay_frame(&mut replay, &scene)?, expected);
        assert_eq!(replay.missing_tiles, 1);
      }
    }
    Ok(())
  }

  #[test]
  fn fills_keep_protocol_bytes() -> crate::Result<()> {
    use crate::payload::Encoder;
    let screen = Screen { origin: (2, 1), ..crate::screen::pixels(5, 3, 4, 8) };
    let rect = Bounds { x0: -0.25, y0: 1.25, x1: 19.2, y1: 14.2 };
    let color = [23, 45, 67, 89];
    let stage = Stage { screen, left: 3 };
    for profile in profiles() {
      let mut direct = Graphics::new(profile, &Encoder::default());
      let mut replay = Graphics::new(profile, &Encoder::default());
      let mut scene = vpdf_model::scene::Scene::new();
      scene.text("before");
      scene.fill(rect, color, Layer::BelowText, stage);
      scene.text("middle");
      scene.fill(rect, color, Layer::AboveText, stage);
      scene.text("after");
      scene.stretch(color, Layer::BelowText, (4, 1, 2), stage);
      scene.stretch(color, Layer::AboveText, (0, 5, 2), stage);
      for _ in 0..2 {
        let mut expected = Vec::new();
        direct.begin(&mut expected)?;
        expected.write_all(b"before")?;
        direct.backdrop(rect, color, screen, 3, &mut expected)?;
        expected.write_all(b"middle")?;
        direct.highlight(rect, color, screen, 3, &mut expected)?;
        expected.write_all(b"after")?;
        direct.stretch(color, -1, (4, 1, 2), screen, 3, &mut expected)?;
        direct.veil(color, screen, 3, &mut expected)?;
        direct.end(&mut expected)?;
        assert_eq!(replay_frame(&mut replay, &scene)?, expected);
      }
    }
    Ok(())
  }

  #[test]
  fn polygon_strip_bytes() -> crate::Result<()> {
    use crate::payload::Encoder;
    use vpdf_model::text::Quad;
    let screen = Screen { origin: (3, 2), ..crate::screen::cells(12, 6) };
    let quad = Quad([(5.0, 20.0), (85.0, 10.0), (85.0, 25.0), (5.0, 35.0)]);
    let viewport = Bounds { x0: 10.0, y0: 12.0, x1: 80.0, y1: 30.0 };
    let color = [20, 80, 160, 90];
    let layers = [Layer::AboveText, Layer::BelowText];
    let stage = Stage { screen, left: 1 };
    for profile in profiles() {
      let mut strips = Graphics::new(profile, &Encoder::default());
      let mut polygons = Graphics::new(profile, &Encoder::default());
      let mut streaming = Graphics::new(profile, &Encoder::default());
      let mut scene = Vec::new();
      for layer in layers {
        scene.quad(quad, viewport, color, layer, stage);
      }
      let mut flat = Vec::new();
      for op in &scene {
        let Op::Polygon(polygon) = op else { panic!("Expected polygon") };
        let fill = polygon.fill();
        for rect in polygon.strips() {
          flat.fill(rect, fill.color(), fill.layer(), stage);
        }
      }
      assert_eq!(scene.len(), 2);
      for _ in 0..2 {
        let expected = replay_frame(&mut strips, &flat)?;
        assert_eq!(replay_frame(&mut polygons, &scene)?, expected);
        let actual = framed(&mut streaming, |painter| {
          painter.quad(quad, viewport, color, layers[0], stage);
          painter.quad(quad, viewport, color, layers[1], stage);
          Ok(())
        })?;
        assert_eq!(actual, expected);
      }
    }
    Ok(())
  }

  #[test]
  fn fills_require_graphics() {
    let screen = crate::screen::pixels(5, 3, 4, 8);
    let mut scene = vpdf_model::scene::Scene::new();
    scene.fill(
      Bounds { x0: 0.0, y0: 0.0, x1: 2.0, y1: 2.0 },
      [0; 4],
      Layer::AboveText,
      Stage { screen, left: 0 },
    );
    let error = paint(&mut Vec::new(), &scene).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
  }

  #[test]
  fn surface_returns_pending_draw_error() {
    let mut out = io::Cursor::new([0u8; 1]);
    let mut painter = Painter::new(&mut out);
    painter.push(Op::Spaces(2));
    let id = TileId {
      owner: vpdf_model::tile::TileOwner::new(1),
      key: vpdf_model::tile::TileKey {
        scale: 1024,
        width: 1,
        height: 1,
        ..Default::default()
      },
    };
    let error = painter.has(id).unwrap_err();
    assert_eq!(
      error.downcast_ref::<io::Error>().unwrap().kind(),
      io::ErrorKind::WriteZero
    );
  }

  #[test]
  fn streaming_keeps_the_first_error() {
    let mut out = io::Cursor::new([0u8; 1]);
    let mut painter = Painter::new(&mut out);
    painter.extend([Op::Spaces(2), Op::Move { column: usize::MAX, row: 0 }]);
    painter.text("after failure");
    let error = painter.finish().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WriteZero);
    assert_eq!(out.position(), 1);
    assert_eq!(*out.get_ref(), [b' ']);
  }

  #[test]
  fn cursor_commands_keep_order() -> io::Result<()> {
    let scene = [
      Op::SaveCursor,
      Op::Move { row: 4, column: 7 },
      Op::Style(Role::Error),
      Op::Text(Text::new("界e\u{301}")),
      Op::Character(vpdf_model::scene::Character::new('界').unwrap()),
      Op::Character(vpdf_model::scene::Character::new('\u{301}').unwrap()),
      Op::Spaces(2),
      Op::Erase(3),
      Op::Style(Role::Reset),
      Op::RestoreCursor,
      Op::HideCursor,
      Op::BlockCursor,
      Op::ShowCursor,
    ];
    let mut out = Vec::new();
    paint(&mut out, &scene)?;
    let want = concat!(
      "\x1b7\x1b[5;8H\x1b[1;31m界e\u{301}界\u{301}  ",
      "\x1b[3X\x1b[0m\x1b8\x1b[?25l\x1b[2 q\x1b[?25h"
    );
    assert_eq!(out, want.as_bytes());
    Ok(())
  }

  #[test]
  fn style_sequences_keep_bytes() -> io::Result<()> {
    let cases: &[(&[Role], &str)] = &[
      (&[Role::Bar, Role::Heading, Role::Key], "\x1b[0;100m\x1b[1;34m\x1b[33m"),
      (
        &[Role::Selected, Role::Muted, Role::Error],
        "\x1b[0;1;100m\x1b[90m\x1b[1;31m",
      ),
      (
        &[Role::TextSelection, Role::Mode, Role::Reset, Role::Argument],
        "\x1b[0;7;35m\x1b[36m\x1b[0m\x1b[35m",
      ),
      (
        &[Role::HoverDim, Role::File, Role::Dim, Role::Reset],
        "\x1b[0;2;4m\x1b[1;34m\x1b[0;2m\x1b[0m",
      ),
    ];
    for &(roles, expected) in cases {
      let mut out = Vec::new();
      let mut painter = Painter::new(&mut out);
      for role in roles {
        painter.extend([Op::Style(*role)]);
      }
      painter.extend([Op::UnderlineOff]);
      painter.finish()?;
      assert_eq!(out, format!("{expected}\x1b[24m").as_bytes());
    }
    Ok(())
  }

  #[test]
  fn blank_runs_cross_chunks() -> io::Result<()> {
    for count in [0, 1, 127, 128, 129, 3840] {
      let mut out = Vec::new();
      paint(&mut out, &[Op::Spaces(count)])?;
      assert_eq!(out, vec![b' '; count]);
    }
    Ok(())
  }

  #[test]
  fn empty_erases_write_nothing() -> io::Result<()> {
    let mut out = Vec::new();
    paint(&mut out, &[Op::Erase(0)])?;
    assert_eq!(out, [] as [u8; 0]);
    Ok(())
  }

  #[test]
  fn overflowing_positions_return_errors() {
    for (column, row) in [(usize::MAX, 0), (0, usize::MAX)] {
      let mut out = Vec::new();
      let error = paint(&mut out, &[Op::Move { column, row }]).unwrap_err();
      assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
      assert_eq!(out, [] as [u8; 0]);
    }
  }
}
