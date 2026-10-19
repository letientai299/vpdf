use super::{
  Bounds, Graphics, Layout, PREVIEW, Pair, Payload, Result, Screen,
  TILE_HEIGHT, TILE_WIDTH, TileKey, Write, encode, fmt, graphics, transmit,
};

impl Graphics {
  /// Starts a frame: deletes forgotten and missing images before their
  /// ids are reused. After a resize, placing a pair again moves it as
  /// ever; `WezTerm`, which loses placement ids on resize, keeps images
  /// in cells, which the full clear after a resize erases.
  pub fn begin(&mut self, out: &mut impl Write) -> Result<()> {
    for id in std::mem::take(&mut self.stale) {
      graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
      self.unplace(id);
      self.free.push(id);
    }
    for id in std::mem::take(&mut self.retired) {
      graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
      self.unplace(id);
      self.spare.push(id);
    }
    self.placed = 0;
    self.pictured = false;
    Ok(())
  }

  /// Deletes every placement and the picture: an empty frame.
  pub fn clear(&mut self, out: &mut impl Write) -> Result<()> {
    self.begin(out)?;
    self.end(out)?;
    out.flush()?;
    Ok(())
  }

  /// Ends a frame: deletes the placements it did not make again, since
  /// placing an id pair again moves that placement.
  pub fn end(&mut self, out: &mut impl Write) -> Result<()> {
    if self.preview != 0 && !self.pictured {
      // Frees the pixels once a frame skips the preview.
      let id = self.base + PREVIEW;
      graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
      self.unplace(id);
      self.preview = 0;
    }
    // Sorting beats hashing the few placements of a frame.
    self.placing.sort_unstable();
    self.placing.dedup();
    if self.quirk {
      // Sized overlays would pile up as selections grow; their pairs
      // leave `shown` with them.
      self.flush(out)?;
    }
    for &(id, p) in &self.shown {
      if self.placing.binary_search(&(id, p)).is_err() {
        graphics(out, &format!("a=d,d=i,i={id},p={p},q=2"), "")?;
      }
    }
    std::mem::swap(&mut self.shown, &mut self.placing);
    self.placing.clear();
    Ok(())
  }

  /// Shows a picture above everything; uploads only when `serial` changes.
  pub fn picture(
    &mut self,
    serial: u64,
    (width, height, rgb): (i32, i32, &[u8]),
    rect: Bounds,
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<()> {
    let id = self.base + PREVIEW;
    self.pictured = true;
    if self.preview != serial {
      transmit(out, id, width, height, 24, &encode(rgb)?)?;
      self.preview = serial;
    }
    // Whole pixels, so the crop matches the image unless the screen cuts it.
    let (x, y) = (rect.x0.floor(), rect.y0.floor());
    let rect =
      Bounds { x0: x, y0: y, x1: x + width as f32, y1: y + height as f32 };
    let Some(clip) = clip(rect, screen) else { return Ok(()) };
    if self.quirk && cut(rect, clip) {
      return self.place_cells(out, id, rect, clip, 2, (left, screen));
    }
    let (width, height) = (clip.width() as u32, clip.height() as u32);
    let control = format_args!("z=2,w={width},h={height}");
    self.put(out, (id, 1), control, clip, left, screen)
  }

  /// Places a page tile, cut to the visible area `clip`.
  pub fn place(
    &mut self,
    key: &TileKey,
    layout: &Layout,
    scroll: f32,
    clip: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<bool> {
    let (x, y) = layout.origin(key.page, scroll);
    self.place_at(key, (x + key.x as f32, y + key.y as f32), clip, left, out)
  }

  /// Places a tile with its top-left at `origin` pixels, uploading a
  /// kept one first; returns false when the tile still needs rendering.
  pub fn place_at(
    &mut self,
    key: &TileKey,
    (x, y): (f32, f32),
    screen: Screen,
    left: u16,
    out: &mut impl Write,
  ) -> Result<bool> {
    // Whole pixels, so neighboring tiles meet without a seam.
    let (x, y) = (x.floor(), y.floor());
    let rect = Bounds {
      x0: x,
      y0: y,
      x1: x + key.width as f32,
      y1: y + key.height as f32,
    };
    let id = match self.resident.get(&(self.doc, *key)) {
      Some(&id) => id,
      None => {
        let kept = self.stored.get(&(self.doc, *key)).cloned();
        let Some(payload) = kept else { return Ok(false) };
        if let Payload::Spooled(slot) = payload
          && crate::spool::get().is_none_or(|spool| !spool.holds(slot))
        {
          self.stored.remove(&(self.doc, *key));
          return Ok(false);
        }
        // Kept tiles upload as they come within a tile of the screen,
        // spreading a scroll's uploads over its frames.
        if !near(rect, screen) {
          return Ok(true);
        }
        self.send((self.doc, *key), &payload, out)?
      }
    };
    let Some(clip) = clip(rect, screen) else { return Ok(true) };
    if self.quirk && cut(rect, clip) {
      self.place_cells(out, id, rect, clip, -2, (left, screen))?;
      return Ok(true);
    }
    let control = format_args!(
      "z=-2,x={},y={},w={},h={}",
      (clip.x0 - x) as u32,
      (clip.y0 - y) as u32,
      clip.width() as u32,
      clip.height() as u32,
    );
    self.put(out, (id, 1), control, clip, left, screen)?;
    Ok(true)
  }

  /// Places `clip`, the visible part of image `id` drawn at `rect`, at
  /// layer `z` for iTerm2, which sizes a crop only by `c,r`: in pieces
  /// of whole cells, which overlap rather than leave a gap. An uncut
  /// side shorter than a cell, as at a page's end, stretches to a cell
  /// rather than vanish. A cut side showing less than a cell draws
  /// nothing: a strip under a cell at the pane's edge stays blank.
  fn place_cells(
    &mut self,
    out: &mut impl Write,
    id: u32,
    rect: Bounds,
    clip: Bounds,
    z: i32,
    (left, screen): (u16, Screen),
  ) -> Result<()> {
    let (width, height) = cell(screen);
    let span = |shown: f32, whole: f32, cell: u32| {
      let shown = shown as u32;
      if shown < cell && shown == whole as u32 {
        vec![(0, shown)]
      } else {
        pieces(shown, cell)
      }
    };
    let columns = span(clip.width(), rect.width(), width);
    let mut p = 0;
    for (top, h) in span(clip.height(), rect.height(), height) {
      for &(start, w) in &columns {
        p += 1;
        let part = Bounds {
          x0: clip.x0 + start as f32,
          y0: clip.y0 + top as f32,
          x1: clip.x0 + (start + w) as f32,
          y1: clip.y0 + (top + h) as f32,
        };
        let control = format_args!(
          "z={z},x={},y={},w={w},h={h},c={},r={}",
          (part.x0 - rect.x0) as u32,
          // iTerm2 measures crop rows from the bottom.
          (rect.y1 - part.y1) as u32,
          w.div_ceil(width),
          h.div_ceil(height)
        );
        self.put(out, (id, p), control, part, left, screen)?;
      }
    }
    Ok(())
  }

  /// Places image `pair` at `rect` with the rest of its keys in
  /// `control`, and remembers the pair for `end`.
  pub(super) fn put(
    &mut self,
    out: &mut impl Write,
    (id, p): Pair,
    control: fmt::Arguments<'_>,
    rect: Bounds,
    left: u16,
    screen: Screen,
  ) -> Result<()> {
    self.placing.push((id, p));
    let (width, height) = cell(screen);
    let (x, y) = (rect.x0 as u32 % width, rect.y0 as u32 % height);
    let control =
      format!("a=p,i={id},p={p},q={},C=1,{control},X={x},Y={y}", self.quiet);
    placement(out, &control, rect, left, screen)
  }
}

pub(super) fn clip(rect: Bounds, screen: Screen) -> Option<Bounds> {
  let clipped = Bounds {
    x0: rect.x0.max(0.0).floor(),
    y0: rect.y0.max(0.0).floor(),
    x1: rect.x1.min(screen.width()).ceil(),
    y1: rect.y1.min(screen.height()).ceil(),
  };
  (clipped.width() > 0.0 && clipped.height() > 0.0).then_some(clipped)
}

/// Whether `rect` lies within a tile of the screen.
fn near(rect: Bounds, screen: Screen) -> bool {
  let (dx, dy) = (TILE_WIDTH as f32, TILE_HEIGHT as f32);
  rect.x1 > -dx
    && rect.y1 > -dy
    && rect.x0 < screen.width() + dx
    && rect.y0 < screen.height() + dy
}

fn cell(screen: Screen) -> (u32, u32) {
  (u32::from(screen.cell_width), u32::from(screen.cell_height))
}

/// Whether the screen cuts `rect` to `clip`.
fn cut(rect: Bounds, clip: Bounds) -> bool {
  clip.width() < rect.width() || clip.height() < rect.height()
}

/// Whole-cell pieces `(start, length)` that cover `length` pixels: the
/// whole cells from the start, then a cell ending at the end when part
/// of one is left. None when less than a cell shows.
pub(super) fn pieces(length: u32, cell: u32) -> Vec<(u32, u32)> {
  let whole = length / cell * cell;
  match whole {
    0 => Vec::new(),
    _ if whole == length => vec![(0, whole)],
    _ => vec![(0, whole), (length - cell, cell)],
  }
}

fn placement(
  out: &mut impl Write,
  control: &str,
  rect: Bounds,
  left: u16,
  screen: Screen,
) -> Result<()> {
  let col = rect.x0 as u32 / u32::from(screen.cell_width) + u32::from(left) + 1;
  let row = rect.y0 as u32 / u32::from(screen.cell_height) + 1;
  crate::terminal::placement(
    out,
    control,
    col + u32::from(screen.origin.0),
    row + u32::from(screen.origin.1),
  )
}
