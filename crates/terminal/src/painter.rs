use crate::graphics::Graphics;
use std::io::{self, Write};
use vpdf_model::scene::{Draw, FillArea, Layer, Op, Text};
use vpdf_model::{geometry::Screen, text::Bounds};

pub fn paint(out: &mut impl Write, scene: &[Op<'_>]) -> io::Result<()> {
  let mut painter = Painter::new(out);
  painter.paint(scene);
  painter.finish()
}

/// Call finish to report output errors.
pub struct Painter<'out, W> {
  out: &'out mut W,
  error: Option<io::Error>,
  graphics: Option<&'out mut Graphics>,
}

impl<'out, W: Write> Painter<'out, W> {
  pub fn new(out: &'out mut W) -> Self {
    Self { out, error: None, graphics: None }
  }

  #[must_use]
  pub fn with_graphics(mut self, graphics: &'out mut Graphics) -> Self {
    self.graphics = Some(graphics);
    self
  }

  pub fn paint(&mut self, scene: &[Op<'_>]) {
    for op in scene {
      if self.error.is_some() {
        break;
      }
      self.error = command(self.out, op, self.graphics.as_deref_mut()).err();
    }
  }

  pub fn finish(self) -> io::Result<()> {
    self.error.map_or(Ok(()), Err)
  }
}

impl<'a, W: Write> Draw<'a> for Painter<'_, W> {
  fn push(&mut self, op: Op<'a>) {
    if self.error.is_none() {
      self.error = command(self.out, &op, self.graphics.as_deref_mut()).err();
    }
  }

  fn text(&mut self, text: &str) {
    if self.error.is_none() {
      self.error = command(
        self.out,
        &Op::Text(Text::new(text)),
        self.graphics.as_deref_mut(),
      )
      .err();
    }
  }

  fn fill(
    &mut self,
    rect: Bounds,
    color: [u8; 4],
    layer: Layer,
    screen: Screen,
    left: u16,
  ) {
    if self.error.is_some() {
      return;
    }
    self.error = paint_fill(
      self.out,
      self.graphics.as_deref_mut(),
      rect,
      color,
      layer,
      (screen, left),
    )
    .err();
  }

  fn stretch(
    &mut self,
    color: [u8; 4],
    layer: Layer,
    cells: (u16, u16, u16),
    screen: Screen,
    left: u16,
  ) {
    if self.error.is_some() {
      return;
    }
    self.error = paint_stretch(
      self.out,
      self.graphics.as_deref_mut(),
      color,
      layer,
      cells,
      (screen, left),
    )
    .err();
  }
}

fn command(
  out: &mut impl Write,
  op: &Op<'_>,
  graphics: Option<&mut Graphics>,
) -> io::Result<()> {
  match op {
    Op::Move { column, row } => {
      let row = position(*row)?;
      let column = position(*column)?;
      write!(out, "\x1b[{row};{column}H")?;
    }
    Op::Style(role) => out.write_all(crate::style::sgr(*role).as_bytes())?,
    Op::Text(text) => out.write_all(text.as_str().as_bytes())?,
    Op::Character(character) => {
      let mut bytes = [0; 4];
      out.write_all(character.value().encode_utf8(&mut bytes).as_bytes())?;
    }
    Op::Spaces(count) => spaces(out, *count)?,
    Op::Erase(0) => {}
    Op::Erase(count) => write!(out, "\x1b[{count}X")?,
    Op::SaveCursor => out.write_all(b"\x1b7")?,
    Op::RestoreCursor => out.write_all(b"\x1b8")?,
    Op::HideCursor => {
      out.write_all(crate::terminal::HIDE_CURSOR.as_bytes())?;
    }
    Op::ShowCursor => out.write_all(b"\x1b[?25h")?,
    Op::BlockCursor => out.write_all(b"\x1b[2 q")?,
    Op::UnderlineOff => out.write_all(b"\x1b[24m")?,
    Op::Fill(fill) => match fill.area() {
      FillArea::Pixels(_) => paint_fill(
        out,
        graphics,
        fill.rect(),
        fill.color(),
        fill.layer(),
        fill.frame(),
      )?,
      FillArea::Cells { column, columns, rows } => paint_stretch(
        out,
        graphics,
        fill.color(),
        fill.layer(),
        (column, columns, rows),
        fill.frame(),
      )?,
    },
  }
  Ok(())
}

fn paint_fill(
  out: &mut impl Write,
  graphics: Option<&mut Graphics>,
  rect: Bounds,
  color: [u8; 4],
  layer: Layer,
  (screen, left): (Screen, u16),
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
  (screen, left): (Screen, u16),
) -> io::Result<()> {
  let z = match layer {
    Layer::BelowText => -1,
    Layer::AboveText => 1,
  };
  graphics_output(graphics, |graphics| {
    graphics.stretch(color, z, cells, screen, left, out)
  })
}

fn graphics_output(
  graphics: Option<&mut Graphics>,
  paint: impl FnOnce(&mut Graphics) -> crate::Result<()>,
) -> io::Result<()> {
  let graphics = graphics.ok_or_else(|| {
    io::Error::new(io::ErrorKind::Unsupported, "Graphics painter missing")
  })?;
  paint(graphics).map_err(|error| {
    error.downcast::<io::Error>().unwrap_or_else(io::Error::other)
  })
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
mod tests {
  use super::*;
  use vpdf_model::scene::Text;
  use vpdf_model::style::Role;

  #[test]
  fn fills_keep_protocol_bytes() -> crate::Result<()> {
    use crate::graphics::Presentation;
    use crate::payload::Encoder;
    use crate::terminal::Transport;
    let screen = Screen {
      columns: 5,
      rows: 3,
      cell_width: 4,
      cell_height: 8,
      origin: (2, 1),
    };
    let rect = Bounds { x0: -0.25, y0: 1.25, x1: 19.2, y1: 14.2 };
    let color = [23, 45, 67, 89];
    for bits in 0..8 {
      let tmux = bits & 1 != 0;
      let crop_quirk = bits & 2 != 0;
      let cell_offsets = bits & 4 != 0;
      let profile = Presentation {
        transport: Transport { tmux },
        crop_quirk,
        cell_offsets,
        report_missing: false,
      };
      let mut direct = Graphics::new(profile, &Encoder::default());
      let mut replay = Graphics::new(profile, &Encoder::default());
      let mut scene = vpdf_model::scene::Scene::new();
      scene.text("before");
      scene.fill(rect, color, Layer::BelowText, screen, 3);
      scene.text("middle");
      scene.fill(rect, color, Layer::AboveText, screen, 3);
      scene.text("after");
      scene.stretch(color, Layer::BelowText, (4, 1, 2), screen, 3);
      scene.stretch(color, Layer::AboveText, (0, 5, 2), screen, 3);
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
        let mut actual = Vec::new();
        replay.begin(&mut actual)?;
        let mut painter = Painter::new(&mut actual).with_graphics(&mut replay);
        painter.paint(&scene);
        painter.finish()?;
        replay.end(&mut actual)?;
        assert_eq!(actual, expected);
      }
    }
    Ok(())
  }

  #[test]
  fn fills_require_graphics() {
    let screen = Screen {
      columns: 5,
      rows: 3,
      cell_width: 4,
      cell_height: 8,
      origin: (0, 0),
    };
    let mut scene = vpdf_model::scene::Scene::new();
    scene.fill(
      Bounds { x0: 0.0, y0: 0.0, x1: 2.0, y1: 2.0 },
      [0; 4],
      Layer::AboveText,
      screen,
      0,
    );
    let error = paint(&mut Vec::new(), &scene).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
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
  fn blank_runs_cross_chunks() -> io::Result<()> {
    for count in [0, 1, 127, 128, 129, 3840] {
      let mut out = Vec::new();
      paint(&mut out, &[Op::Spaces(count)])?;
      assert_eq!(out, vec![b' '; count]);
    }
    Ok(())
  }

  #[test]
  fn reports_output_failure() {
    let mut out = io::Cursor::new([0u8; 1]);
    let error = paint(&mut out, &[Op::Spaces(2)]).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::WriteZero);
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
