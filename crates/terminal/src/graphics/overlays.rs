use super::Tracking;
use super::{
  Bounds, Graphics, OVERLAY_END, Ordering, PREVIEW, Result, SWATCH, Screen,
  Solid, Write, clip, encode, transmit,
};

impl Graphics {
  /// Tints `rect` above the page and its text.
  pub fn highlight(
    &mut self,
    rect: Bounds,
    color: [u8; 4],
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    self.overlay(rect, color, 1, screen, left, out)
  }

  /// Fills `rect` above the page but below terminal text, as a label
  /// background.
  pub fn backdrop(
    &mut self,
    rect: Bounds,
    color: [u8; 4],
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    self.overlay(rect, color, -1, screen, left, out)
  }

  /// Tints the whole page area by stretching one pixel over its cells.
  pub fn veil(
    &mut self,
    color: [u8; 4],
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    let rows = screen.rows.saturating_sub(1);
    let cells = (0, screen.columns, rows);
    self.stretch(color, 1, cells, screen, left, out)
  }

  /// One stretched pixel over cells.
  pub fn stretch(
    &mut self,
    color: [u8; 4],
    z: i32,
    (column, columns, rows): (u16, u16, u16),
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    let Some(id) = self.solid((color, 1, 1), out)? else { return Ok(()) };
    self.placed += 1;
    let control = format_args!("z={z},c={columns},r={rows}");
    let x = f32::from(column) * f32::from(screen.cell_width);
    let rect = Bounds { x0: x, y0: 0.0, x1: x + 1.0, y1: 1.0 };
    self.put(out, (id, self.placed), control, rect, left, screen)
  }

  /// The overlay image of `solid`, sent on first use; None when this
  /// frame already holds every overlay id.
  pub(super) fn solid(
    &mut self,
    solid: Solid,
    out: &mut impl Write,
  ) -> Result<Option<u32>> {
    if let Some(&id) = self.solids.get(&solid) {
      return Ok(Some(id));
    }
    if self.spare.is_empty() && self.next_overlay() >= self.base + PREVIEW {
      self.placing.sort_unstable();
      self.flush(out)?;
    }
    let id = match self.spare.pop() {
      Some(id) => id,
      None => self.next_overlay(),
    };
    if id >= self.base + PREVIEW {
      return Ok(None);
    }
    self.overlay_end = self.overlay_end.max(id + 1);
    if self.tracking == Tracking::Process {
      OVERLAY_END.fetch_max(id + 1, Ordering::Relaxed);
    }
    let (color, width, height) = solid;
    let pixels = color.repeat((width * height) as usize);
    transmit(self.transport, out, id, width, height, 32, &encode(&pixels)?)?;
    self.solids.insert(solid, id);
    Ok(Some(id))
  }

  /// The overlay id after every one handed out: each is in use, spare,
  /// or retired.
  pub(super) fn next_overlay(&self) -> u32 {
    let used = self.solids.len() + self.spare.len() + self.retired.len();
    self.base + used as u32
  }

  /// Deletes the overlay images this frame has not placed, which only
  /// iTerm2's sized overlays use up; `placing` must be sorted.
  pub(super) fn flush(&mut self, out: &mut impl Write) -> Result<()> {
    let placing = &self.placing;
    let placed = |id: u32| {
      let at = placing.partition_point(|pair| pair.0 < id);
      placing.get(at).is_some_and(|pair| pair.0 == id)
    };
    let unused: Vec<_> = self
      .solids
      .iter()
      .filter(|(_, id)| !placed(**id))
      .map(|(solid, id)| (*solid, *id))
      .collect();
    for (solid, id) in unused {
      self.delete_image(out, id)?;
      self.solids.remove(&solid);
      self.spare.push(id);
    }
    Ok(())
  }

  pub(super) fn overlay(
    &mut self,
    rect: Bounds,
    color: [u8; 4],
    z: i32,
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    let Some(rect) = clip(rect, screen) else { return Ok(()) };
    let width = rect.width().ceil().max(1.0) as i32;
    let height = rect.height().ceil().max(1.0) as i32;
    if self.quirk {
      // A whole image of the rect's size, since iTerm2 stretches crops.
      let Some(id) = self.solid((color, width, height), out)? else {
        return Ok(());
      };
      self.placed += 1;
      let control = format_args!("z={z}");
      return self.put(out, (id, self.placed), control, rect, left, screen);
    }
    let Some(id) = self.solid((color, SWATCH.0, SWATCH.1), out)? else {
      return Ok(());
    };
    // Rects larger than the swatch take several crops.
    let (cw, ch) = if self.cell_offsets && z < 0 {
      (i32::from(screen.cell_width), i32::from(screen.cell_height))
    } else {
      (1, 1)
    };
    for (top, h) in segments(height, rect.y0 as i32, ch, SWATCH.1) {
      for (left_px, w) in segments(width, rect.x0 as i32, cw, SWATCH.0) {
        let offset = (left_px as f32, top as f32);
        let part = rect.part(offset, (w as f32, h as f32));
        self.placed += 1;
        let control = format_args!("z={z},x=0,y=0,w={w},h={h}");
        self.put(out, (id, self.placed), control, part, left, screen)?;
      }
    }
    Ok(())
  }
}

// WezTerm paints negative layers cell by cell.
pub(super) fn segments(
  length: i32,
  origin: i32,
  cell: i32,
  limit: i32,
) -> Vec<(i32, i32)> {
  let mut parts = Vec::new();
  let mut start = 0;
  while start < length {
    let offset = (origin + start) % cell;
    let available =
      if offset == 0 { (limit / cell).max(1) * cell } else { cell - offset };
    let size = available.min(limit).min(length - start);
    parts.push((start, size));
    start += size;
  }
  parts
}
