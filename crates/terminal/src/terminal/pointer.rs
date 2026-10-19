use super::{Mutex, PoisonError, Result, Support, Write, passthrough, tmux};

/// Pointer shape the reader last wrote.
static SHAPE: Mutex<Shapes> = Mutex::new(Shapes { owner: None, shown: None });

/// OSC 22 pointer shapes ([spec][spec]).
///
/// [spec]: https://sw.kovidgoyal.net/kitty/pointer-shapes/
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Shape {
  #[default]
  Default,
  Pointer,
  Text,
  Crosshair,
  Grabbing,
  ColResize,
  RowResize,
}

impl Shape {
  pub(super) fn name(self) -> &'static str {
    match self {
      Self::Default => "default",
      Self::Pointer => "pointer",
      Self::Text => "text",
      Self::Crosshair => "crosshair",
      Self::Grabbing => "grabbing",
      Self::ColResize => "col-resize",
      Self::RowResize => "row-resize",
    }
  }
}

/// How the reader holds the pointer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Owner {
  /// Kitty: pushed on its shape stack.
  Stack,
  /// Ghostty: set by name, starting at `text`.
  Named,
}

#[derive(Debug)]
pub(super) struct Shapes {
  /// None without shapes or the screen.
  pub(super) owner: Option<Owner>,
  /// None when unknown: the next shape writes.
  pub(super) shown: Option<Shape>,
}

impl Shapes {
  /// Takes over the pointer where supported.
  pub(super) fn push(
    &mut self,
    support: Support,
    out: &mut impl Write,
    tmux: bool,
  ) -> Result<()> {
    if self.owner.is_some() {
      return Ok(());
    }
    let (owner, body) = match support {
      Support::Answered => (Owner::Stack, ">default"),
      Support::Named => (Owner::Named, "default"),
      Support::Missing => return Ok(()),
    };
    osc22(out, body, tmux)?;
    self.owner = Some(owner);
    self.shown = Some(Shape::Default);
    Ok(())
  }

  /// Writes `shape` when it changed.
  pub(super) fn point(
    &mut self,
    shape: Shape,
    out: &mut impl Write,
    tmux: bool,
  ) -> Result<()> {
    if self.owner.is_none() || self.shown == Some(shape) {
      return Ok(());
    }
    osc22(out, shape.name(), tmux)?;
    self.shown = Some(shape);
    Ok(())
  }

  /// Another program may have set one.
  pub(super) fn forget(&mut self) {
    self.shown = None;
  }

  /// Restores the terminal's own shape.
  pub(super) fn pop(&mut self, out: &mut impl Write, tmux: bool) -> Result<()> {
    self.shown = None;
    match self.owner.take() {
      Some(Owner::Stack) => osc22(out, "<", tmux),
      // Ghostty 1.3 ignores an empty reset.
      Some(Owner::Named) => osc22(out, Shape::Text.name(), tmux),
      None => Ok(()),
    }
  }
}

pub(super) fn osc22(
  out: &mut impl Write,
  body: &str,
  tmux: bool,
) -> Result<()> {
  passthrough(out, format!("\x1b]22;{body}\x1b\\").as_bytes(), tmux)
}

pub(super) fn shapes() -> std::sync::MutexGuard<'static, Shapes> {
  SHAPE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Sets the pointer shape; writes only changes.
pub fn point(out: &mut impl Write, shape: Shape) -> Result<()> {
  shapes().point(shape, out, tmux())
}

/// Sets `default` once; the next shape always writes.
pub fn hand_over(out: &mut impl Write) -> Result<()> {
  let mut shapes = shapes();
  if shapes.shown.is_none() {
    return Ok(());
  }
  shapes.point(Shape::Default, out, tmux())?;
  shapes.forget();
  Ok(())
}

/// Claims pointer ownership without polling input.
pub fn claim_pointer(
  out: &mut impl Write,
  support: Support,
  tmux: bool,
) -> Result<()> {
  shapes().push(support, out, tmux)
}
