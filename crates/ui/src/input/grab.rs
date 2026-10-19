#[derive(Default)]
pub struct Grab {
  held: Option<Held>,
}

struct Held {
  last: (f32, f32),
  moved: bool,
}

impl Grab {
  pub fn begin(&mut self, at: (f32, f32)) {
    self.held = Some(Held { last: at, moved: false });
  }

  pub fn moved(&self) -> bool {
    self.held.as_ref().is_some_and(|held| held.moved)
  }

  pub fn cancel(&mut self) -> bool {
    let moved = self.moved();
    self.held = None;
    moved
  }

  pub fn release(&mut self) -> bool {
    self.held.take().is_some_and(|held| !held.moved)
  }

  /// Horizontal pan and vertical scroll subtraction.
  pub fn pixels(&mut self, at: (f32, f32)) -> Option<(f32, f32)> {
    let held = self.held.as_mut()?;
    held.moved = true;
    let last = std::mem::replace(&mut held.last, at);
    Some((last.0 - at.0, at.1 - last.1))
  }

  pub fn cells(
    &mut self,
    at: (f32, f32),
    cell: (u16, u16),
  ) -> Option<(isize, isize)> {
    let held = self.held.as_mut()?;
    held.moved = true;
    let (width, height) = (f32::from(cell.0), f32::from(cell.1));
    let columns = ((at.0 - held.last.0) / width).trunc();
    let rows = ((at.1 - held.last.1) / height).trunc();
    if columns == 0.0 && rows == 0.0 {
      return None;
    }
    held.last.0 += columns * width;
    held.last.1 += rows * height;
    Some((-columns as isize, -rows as isize))
  }
}
