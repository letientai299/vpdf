use crate::Result;
use crate::cache::Cache;
use crate::caps::{Host, Profile};
use crate::document::MIB;
use crate::layout::{Layout, Screen};
use crate::spool::Spool;
use crate::terminal::graphics;
use crate::text::Bounds;
use crate::tiles::{Payload, TILE_HEIGHT, TILE_WIDTH, Tile, TileKey, encode};
use base64::{Engine, engine::general_purpose::STANDARD};
use flate2::read::ZlibDecoder;
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const BLOCK: u32 = 1 << 16;
const OVERLAYS: u32 = 1024;
/// Last overlay id holds the link preview across frames.
const PREVIEW: u32 = OVERLAYS - 1;
/// Preview serial after an `ENOENT`: the next picture uploads again, and
/// a frame without one deletes the id, which a late reply may name while
/// a newer upload still shows.
const LOST: u64 = u64::MAX;
/// Solid-color image an overlay crops to its size, so frames place
/// highlights without sending pixels; about 1 MiB per color in Kitty.
const SWATCH: (i32, i32) = (4096, 64);
/// One past the highest overlay and tile ids any frame used, so exit
/// deletes each owned image.
static OVERLAY_END: AtomicU32 = AtomicU32::new(0);
static TILE_END: AtomicU32 = AtomicU32::new(0);
// Four readers fit Kitty's 320 MB quota.
const BUDGET: usize = 64 * MIB;
/// Resident budget when tiles come from the spool: an upload costs Kitty
/// a file read, so scrolling back uploads instead of keeping 64 MiB of
/// textures, which take about three times their RGB size in Kitty.
const SPOOLED: usize = 24 * MIB;
/// Encoded tiles kept after rendering: prefetched tiles wait here until
/// they scroll into view, and evicted or lost images upload again without
/// rendering. Text tiles compress to about 4% of their RGB size.
const STORED: usize = 48 * MIB;

fn budget() -> usize {
  if crate::spool::get().is_some() { SPOOLED } else { BUDGET }
}

/// Tiles of one document; tabs share one Kitty budget.
type Key = (u32, TileKey);

/// An image id and a placement id.
type Pair = (u32, u32);

/// A solid overlay image: its color and size in pixels.
type Solid = ([u8; 4], i32, i32);

pub struct Graphics {
  /// Document whose tiles `upload` and `place` address.
  pub doc: u32,
  resident: Cache<Key, u32>,
  stored: Cache<Key, Payload>,
  ids: HashMap<Key, u32>,
  free: Vec<u32>,
  stale: Vec<u32>,
  /// Serial of the uploaded preview; zero when none, `LOST` after an
  /// `ENOENT`.
  preview: u64,
  /// Whether the last frame placed the preview.
  pictured: bool,
  base: u32,
  next: u32,
  /// Overlay image ids, kept across frames.
  solids: HashMap<Solid, u32>,
  /// Overlay ids freed for reuse.
  spare: Vec<u32>,
  /// Overlay ids the next frame deletes before they are reused.
  retired: Vec<u32>,
  /// Overlay placements in the current frame.
  placed: u32,
  /// Placements the last frame left on screen, sorted.
  shown: Vec<Pair>,
  /// Placements made so far in this frame.
  placing: Vec<Pair>,
  /// iTerm2 draws a crop at the image's size unless `c,r` size it.
  quirk: bool,
  /// `q` of placements: 1 reports `ENOENT` for an evicted image, which
  /// tmux would hand to the active pane as keys.
  quiet: u8,
  pub uploaded: u64,
}

impl Default for Graphics {
  fn default() -> Self {
    let base = session();
    let (quirk, quiet) = traits();
    Self {
      doc: 0,
      resident: Cache::new(budget()),
      stored: Cache::new(STORED),
      ids: HashMap::new(),
      free: Vec::new(),
      stale: Vec::new(),
      preview: 0,
      pictured: false,
      base,
      next: base + OVERLAYS,
      solids: HashMap::new(),
      spare: Vec::new(),
      retired: Vec::new(),
      placed: 0,
      shown: Vec::new(),
      placing: Vec::new(),
      quirk,
      quiet,
      uploaded: 0,
    }
  }
}

/// iTerm2's crop quirk and the placements' `q`, from the probe. Panes
/// lend `Graphics` through `Default` on every loop, so this runs once.
fn traits() -> (bool, u8) {
  static TRAITS: OnceLock<(bool, u8)> = OnceLock::new();
  // Before the probe, as in benches, nothing is cached.
  let Some(caps) = crate::caps::get() else { return (false, 2) };
  *TRAITS.get_or_init(|| {
    let quirk = caps.profile() == Profile::CropQuirk;
    let quiet = if matches!(caps.host, Host::Direct(_)) { 1 } else { 2 };
    (quirk, quiet)
  })
}

/// Random id block; sibling panes share Kitty ids.
pub fn session() -> u32 {
  static BASE: OnceLock<u32> = OnceLock::new();
  *BASE.get_or_init(|| {
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    let blocks = u64::from(u32::MAX / BLOCK);
    (hasher.finish() % blocks) as u32 * BLOCK + BLOCK
  })
}

/// A fresh serial for `Graphics::picture`; every picture shares one id.
pub fn serial() -> u64 {
  static SERIAL: AtomicU64 = AtomicU64::new(1);
  SERIAL.fetch_add(1, Ordering::Relaxed)
}

/// Deletes this process's images and placements, one id at a time:
/// range deletes are dropped by `WezTerm`, ignored by Konsole, and delete
/// other programs' images in Ghostty 1.3.1 and Warp. The ids run up to
/// the highest ever used, freed ones included.
pub fn release(out: &mut impl Write) -> Result<()> {
  let base = session();
  let overlays = base..OVERLAY_END.load(Ordering::Relaxed).max(base);
  let tiles = base + OVERLAYS..TILE_END.load(Ordering::Relaxed);
  for id in overlays.chain([base + PREVIEW]).chain(tiles) {
    graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
  }
  Ok(())
}

impl Graphics {
  /// Whether overlays need exact-size images.
  pub fn crop_quirk(&self) -> bool {
    self.quirk
  }

  pub fn contains(&mut self, key: &TileKey) -> bool {
    self.resident.get(&(self.doc, *key)).is_some()
  }

  /// Whether `key` is resident or kept for upload.
  pub fn has(&mut self, key: &TileKey) -> bool {
    let key = (self.doc, *key);
    self.resident.get(&key).is_some() || self.stored.get(&key).is_some()
  }

  /// Deletes a document's tiles at the next frame.
  pub fn forget(&mut self, doc: u32) {
    let keys: Vec<_> =
      self.ids.keys().filter(|key| key.0 == doc).copied().collect();
    for key in keys {
      self.resident.remove(&key);
      self.stale.extend(self.ids.remove(&key));
    }
    let kept: Vec<_> = self
      .stored
      .entries()
      .map(|(key, _)| *key)
      .filter(|k| k.0 == doc)
      .collect();
    for key in kept {
      self.stored.remove(&key);
    }
  }

  /// Forgets terminal images after another program had the screen; kept
  /// tiles upload again when next placed.
  pub fn lost(&mut self) {
    let stored = std::mem::replace(&mut self.stored, Cache::new(0));
    let (doc, uploaded) = (self.doc, self.uploaded);
    *self = Self { doc, stored, uploaded, ..Self::default() };
  }

  /// Handles the terminal's `ENOENT` for image `id`: it evicted the
  /// image, so it uploads again. A late reply can name an id uploaded
  /// again since, so the next frame deletes the id before its reuse.
  /// Returns whether `id` was one in use.
  pub fn missing(&mut self, id: u32) -> bool {
    self.unplace(id);
    if id == self.base + PREVIEW {
      self.preview = LOST;
      return true;
    }
    if let Some(solid) = find(&self.solids, id) {
      self.solids.remove(&solid);
      self.retired.push(id);
      return true;
    }
    let Some(key) = find(&self.ids, id) else { return false };
    self.ids.remove(&key);
    self.resident.remove(&key);
    self.stale.push(id);
    true
  }

  /// Forgets placements of `id`, whose image is gone.
  fn unplace(&mut self, id: u32) {
    self.shown.retain(|pair| pair.0 != id);
    self.placing.retain(|pair| pair.0 != id);
  }

  /// Keeps a tile until a frame places it.
  pub fn keep(&mut self, tile: Tile) {
    let weight = match &tile.payload {
      Payload::Encoded(encoded) => encoded.len(),
      // The spool holds the pixels; this keeps about one entry per slot.
      Payload::Spooled(slot) => slot.len * STORED / crate::spool::BYTES,
    };
    self.stored.insert((self.doc, tile.key), tile.payload, weight);
  }

  pub fn upload(&mut self, tile: &Tile, out: &mut impl Write) -> Result<()> {
    if !self.contains(&tile.key) {
      self.send((self.doc, tile.key), &tile.payload, out)?;
    }
    Ok(())
  }

  /// Transmits a tile and returns its image id.
  fn send(
    &mut self,
    key: Key,
    payload: &Payload,
    out: &mut impl Write,
  ) -> Result<u32> {
    let id = self.free.pop().unwrap_or_else(|| {
      self.next += 1;
      TILE_END.fetch_max(self.next, Ordering::Relaxed);
      self.next - 1
    });
    let tile = key.1;
    let bytes = match (payload, crate::spool::get()) {
      (Payload::Encoded(encoded), _) => {
        transmit(out, id, tile.width, tile.height, 24, encoded)?;
        encoded.len()
      }
      (Payload::Spooled(slot), Some(spool)) => {
        let control = format!(
          "a=t,f=24,i={id},q=2,s={},v={},{}",
          tile.width,
          tile.height,
          Spool::keys(*slot)
        );
        graphics(out, &control, &spool.payload)?;
        slot.len
      }
      (Payload::Spooled(_), None) => {
        anyhow::bail!("Spooled tile without spool")
      }
    };
    // Weight floor keeps residency within tile ids.
    let weight = ((tile.width * tile.height * 3) as usize)
      .max(budget().div_ceil((BLOCK - OVERLAYS) as usize));
    for key in self.resident.insert(key, id, weight) {
      if let Some(id) = self.ids.remove(&key) {
        graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
        self.unplace(id);
        self.free.push(id);
      }
    }
    self.ids.insert(key, id);
    self.uploaded += bytes as u64;
    Ok(id)
  }

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
  fn put(
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
  fn solid(
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
    OVERLAY_END.fetch_max(id + 1, Ordering::Relaxed);
    let (color, width, height) = solid;
    let pixels = color.repeat((width * height) as usize);
    transmit(out, id, width, height, 32, &encode(&pixels)?)?;
    self.solids.insert(solid, id);
    Ok(Some(id))
  }

  /// The overlay id after every one handed out: each is in use, spare,
  /// or retired.
  fn next_overlay(&self) -> u32 {
    let used = self.solids.len() + self.spare.len() + self.retired.len();
    self.base + used as u32
  }

  /// Deletes the overlay images this frame has not placed, which only
  /// iTerm2's sized overlays use up; `placing` must be sorted.
  fn flush(&mut self, out: &mut impl Write) -> Result<()> {
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
      graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
      self.solids.remove(&solid);
      self.shown.retain(|pair| pair.0 != id);
      self.spare.push(id);
    }
    Ok(())
  }

  fn overlay(
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
    for top in (0..height).step_by(SWATCH.1 as usize) {
      for left_px in (0..width).step_by(SWATCH.0 as usize) {
        let (w, h) =
          ((width - left_px).min(SWATCH.0), (height - top).min(SWATCH.1));
        let part = Bounds {
          x0: rect.x0 + left_px as f32,
          y0: rect.y0 + top as f32,
          x1: rect.x0 + (left_px + w) as f32,
          y1: rect.y0 + (top + h) as f32,
        };
        self.placed += 1;
        let control = format_args!("z={z},x=0,y=0,w={w},h={h}");
        self.put(out, (id, self.placed), control, part, left, screen)?;
      }
    }
    Ok(())
  }

  pub fn bytes(&self) -> usize {
    self.resident.used()
  }
}

fn clip(rect: Bounds, screen: Screen) -> Option<Bounds> {
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

/// The key of `map` whose value is `id`.
fn find<K: Copy>(map: &HashMap<K, u32>, id: u32) -> Option<K> {
  map.iter().find_map(|(key, value)| (*value == id).then_some(*key))
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
fn pieces(length: u32, cell: u32) -> Vec<(u32, u32)> {
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

fn transmit(
  out: &mut impl Write,
  id: u32,
  width: i32,
  height: i32,
  format: u8,
  encoded: &str,
) -> Result<()> {
  // iTerm2 rejects short compressed payloads.
  let (compression, encoded) = if encoded.len() <= 20 {
    let compressed = STANDARD.decode(encoded)?;
    let mut pixels = Vec::new();
    ZlibDecoder::new(compressed.as_slice()).read_to_end(&mut pixels)?;
    ("", Cow::Owned(STANDARD.encode(pixels)))
  } else {
    ("o=z,", Cow::Borrowed(encoded))
  };
  let control =
    format!("a=t,f={format},{compression}i={id},q=2,s={width},v={height}");
  crate::terminal::transmit(out, &control, &encoded)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::PageId;

  fn commands(output: &[u8]) -> Vec<HashMap<String, String>> {
    let text = String::from_utf8_lossy(output);
    text
      .split("_G")
      .skip(1)
      .map(|command| {
        let control = command.split(';').next().unwrap_or_default();
        control
          .split(',')
          .filter_map(|pair| pair.split_once('='))
          .map(|(key, value)| (key.to_owned(), value.to_owned()))
          .collect()
      })
      .collect()
  }

  #[test]
  fn commands_stay_in_session_block() -> Result<()> {
    let mut graphics = Graphics {
      resident: Cache::new(2 * budget().div_ceil((BLOCK - OVERLAYS) as usize)),
      ..Graphics::default()
    };
    let block = session()..=session() + (BLOCK - 1);
    let mut output = Vec::new();
    for page in 0..4 {
      let key = TileKey {
        page: PageId(page),
        scale: 1,
        rotation: 0,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        crop: crate::layout::Crop::default(),
      };
      graphics.upload(
        &Tile {
          key,
          payload: Payload::Encoded(encode(&[255; 3])?.into()),
          render_ms: 0.0,
        },
        &mut output,
      )?;
    }
    let screen = Screen {
      columns: 10,
      rows: 10,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
    graphics.highlight(rect, [0; 4], screen, 0, &mut output)?;
    graphics.begin(&mut output)?;
    release(&mut output)?;
    let commands = commands(&output);
    assert!(commands.iter().any(|c| c.get("d").is_some_and(|d| d == "I")));
    for command in commands {
      if let Some(delete) = command.get("d") {
        assert!(["I", "i"].contains(&delete.as_str()), "{delete}");
      }
      let ids = ["i", "x", "y"].iter().filter(|key| {
        *key == &"i" || command.get("a").is_some_and(|a| a == "d")
      });
      for key in ids {
        if let Some(id) = command.get(*key) {
          assert!(block.contains(&id.parse()?), "{key}={id}");
        }
      }
    }
    assert_eq!(graphics.free.len(), 1);
    assert_eq!(graphics.next, session() + OVERLAYS + 3);
    Ok(())
  }

  const SCREEN: Screen = Screen {
    columns: 20,
    rows: 10,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };

  /// A resident 64x64 tile of page `page`.
  fn tile(graphics: &mut Graphics, page: usize) -> Result<TileKey> {
    tile_sized(graphics, page, (64, 64))
  }

  fn tile_sized(
    graphics: &mut Graphics,
    page: usize,
    (width, height): (i32, i32),
  ) -> Result<TileKey> {
    let key = TileKey {
      page: PageId(page),
      scale: 1,
      rotation: 0,
      x: 0,
      y: 0,
      width,
      height,
      crop: crate::layout::Crop::default(),
    };
    let pixels = vec![255; (width * height * 3) as usize];
    let payload = Payload::Encoded(encode(&pixels)?.into());
    graphics.upload(&Tile { key, payload, render_ms: 0.0 }, &mut Vec::new())?;
    Ok(key)
  }

  fn has(command: &HashMap<String, String>, pairs: &[(&str, &str)]) -> bool {
    pairs
      .iter()
      .all(|(key, value)| command.get(*key).map(String::as_str) == Some(value))
  }

  /// Deletes in `output` as `(d, i, p)`.
  fn deletes(output: &[u8]) -> Vec<(String, u32, Option<u32>)> {
    commands(output)
      .into_iter()
      .filter(|c| c.get("a").is_some_and(|a| a == "d"))
      .map(|c| {
        let number = |key: &str| c.get(key).and_then(|v| v.parse().ok());
        (c["d"].clone(), number("i").unwrap_or(0), number("p"))
      })
      .collect()
  }

  #[test]
  fn centering_moves_resident_tiles() -> Result<()> {
    use crate::document::PageInfo;
    use crate::layout::PageLayout;
    let mut graphics = Graphics::default();
    let key = tile(&mut graphics, 0)?;
    let bounds = Bounds { x0: 0.0, y0: 0.0, x1: 64.0, y1: 64.0 };
    let label = std::cell::OnceCell::new();
    let mut layout = Layout::new(&[PageInfo { bounds, label }], SCREEN);
    layout.pages[0] = PageLayout {
      top: 0.0,
      width: 64.0,
      height: 64.0,
      scale: key.scale,
      matrix: mupdf::Matrix::IDENTITY,
    };
    let mut output = Vec::new();
    graphics.begin(&mut output)?;
    assert!(graphics.place(&key, &layout, 0.0, SCREEN, 0, &mut output)?);
    graphics.end(&mut output)?;
    let first =
      commands(&output).into_iter().find(|c| has(c, &[("a", "p")])).unwrap();
    let tiles = crate::tiles::visible(&layout, 0.0);
    layout.screen.columns += 1;
    assert_eq!(crate::tiles::visible(&layout, 0.0), tiles);
    output.clear();
    graphics.begin(&mut output)?;
    assert!(graphics.place(
      &key,
      &layout,
      0.0,
      layout.screen,
      0,
      &mut output
    )?);
    graphics.end(&mut output)?;
    let placed = commands(&output);
    assert_eq!(placed.len(), 1);
    assert!(has(
      &placed[0],
      &[("a", "p"), ("i", &first["i"]), ("p", &first["p"])]
    ));
    assert_ne!(placed[0]["X"], first["X"]);
    assert_eq!(graphics.ids.len(), 1);
    assert_eq!(deletes(&output), []);
    Ok(())
  }

  #[test]
  fn frames_delete_only_what_they_leave() -> Result<()> {
    let mut graphics = Graphics::default();
    let (a, b) = (tile(&mut graphics, 0)?, tile(&mut graphics, 1)?);
    let (id_a, id_b) = (graphics.ids[&(0, a)], graphics.ids[&(0, b)]);
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
    let mut output = Vec::new();
    graphics.begin(&mut output)?;
    graphics.place_at(&a, (0.0, 0.0), SCREEN, 0, &mut output)?;
    graphics.place_at(&b, (0.0, 64.0), SCREEN, 0, &mut output)?;
    graphics.highlight(rect, [1; 4], SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    assert_eq!(deletes(&output), []);
    let swatch = graphics.solids[&([1; 4], SWATCH.0, SWATCH.1)];
    // The next frame moves `a` and drops `b` and the highlight.
    output.clear();
    graphics.begin(&mut output)?;
    graphics.place_at(&a, (0.0, 3.0), SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    let mut deleted = deletes(&output);
    deleted.sort();
    let want = [("i".into(), id_b, Some(1)), ("i".into(), swatch, Some(1))];
    let mut want = want.to_vec();
    want.sort();
    assert_eq!(deleted, want);
    let placed: Vec<_> =
      commands(&output).into_iter().filter(|c| has(c, &[("a", "p")])).collect();
    assert_eq!(placed.len(), 1);
    assert!(has(
      &placed[0],
      &[("i", &id_a.to_string()), ("p", "1"), ("Y", "3")]
    ));
    // An unchanged frame deletes nothing.
    output.clear();
    graphics.begin(&mut output)?;
    graphics.place_at(&a, (0.0, 3.0), SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    assert_eq!(deletes(&output), []);
    Ok(())
  }

  #[test]
  fn preview_keeps_pixels_while_shown() -> Result<()> {
    let mut graphics = Graphics::default();
    let id = graphics.base + PREVIEW;
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 2.0, y1: 2.0 };
    let mut output = Vec::new();
    for _ in 0..2 {
      output.clear();
      graphics.begin(&mut output)?;
      graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
      graphics.end(&mut output)?;
      assert_eq!(deletes(&output), []);
    }
    assert!(
      commands(&output).iter().any(|c| has(c, &[("a", "p"), ("p", "1")]))
    );
    output.clear();
    graphics.begin(&mut output)?;
    graphics.end(&mut output)?;
    assert_eq!(deletes(&output), [("I".into(), id, None)]);
    assert_eq!(graphics.preview, 0);
    // A late `ENOENT` while a newer upload shows: the next picture
    // uploads again, and a frame without one deletes the id.
    for pictured in [true, false] {
      graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
      graphics.missing(id);
      output.clear();
      graphics.begin(&mut output)?;
      if pictured {
        graphics.picture(7, (2, 2, &[0; 12]), rect, SCREEN, 0, &mut output)?;
        assert!(commands(&output).iter().any(|c| has(c, &[("a", "t")])));
      }
      graphics.end(&mut output)?;
      let deleted = deletes(&output).contains(&("I".into(), id, None));
      assert_eq!(deleted, !pictured);
    }
    Ok(())
  }

  #[test]
  fn missing_images_upload_again() -> Result<()> {
    let mut graphics = Graphics::default();
    let a = tile(&mut graphics, 0)?;
    let id = graphics.ids[&(0, a)];
    let mut output = Vec::new();
    graphics.highlight(
      Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 },
      [1; 4],
      SCREEN,
      0,
      &mut output,
    )?;
    let swatch = graphics.solids[&([1; 4], SWATCH.0, SWATCH.1)];
    graphics.preview = 3;
    assert!(graphics.missing(id));
    assert!(!graphics.contains(&a));
    assert!(graphics.missing(swatch));
    assert!(graphics.solids.is_empty());
    assert!(graphics.missing(graphics.base + PREVIEW));
    assert_eq!(graphics.preview, LOST);
    assert!(!graphics.missing(1));
    Ok(())
  }

  #[test]
  fn placements_report_errors_only_when_asked() -> Result<()> {
    for quiet in [1, 2] {
      let mut graphics = Graphics { quiet, ..Graphics::default() };
      let a = tile(&mut graphics, 0)?;
      let mut output = Vec::new();
      graphics.place_at(&a, (0.0, 0.0), SCREEN, 0, &mut output)?;
      let q = quiet.to_string();
      assert!(has(&commands(&output)[0], &[("a", "p"), ("q", &q)]));
    }
    Ok(())
  }

  #[test]
  fn tiny_images_skip_compression() -> Result<()> {
    for (count, compressed) in [(1, false), (14, false), (1000, true)] {
      let pixels = [255, 0, 0, 128].repeat(count);
      let mut output = Vec::new();
      transmit(&mut output, 99, count as i32, 1, 32, &encode(&pixels)?)?;
      let controls = commands(&output);
      assert_eq!(controls[0].contains_key("o"), compressed);
      let text = String::from_utf8(output)?;
      let payload = text.split_once(';').unwrap().1.trim_end_matches("\x1b\\");
      let bytes = STANDARD.decode(payload)?;
      let actual = if compressed {
        let mut decoded = Vec::new();
        ZlibDecoder::new(bytes.as_slice()).read_to_end(&mut decoded)?;
        decoded
      } else {
        bytes
      };
      assert_eq!(actual, pixels);
    }
    Ok(())
  }

  #[test]
  fn crop_quirk_sizes_overlays() -> Result<()> {
    let mut graphics = Graphics { quirk: true, ..Graphics::default() };
    let mut output = Vec::new();
    let rect = Bounds { x0: 10.0, y0: 20.0, x1: 50.0, y1: 27.0 };
    graphics.highlight(rect, [1; 4], SCREEN, 0, &mut output)?;
    let sent = commands(&output);
    assert!(has(&sent[0], &[("a", "t"), ("s", "40"), ("v", "7")]));
    let placed = &sent[1];
    assert!(has(placed, &[("a", "p"), ("X", "2"), ("Y", "4")]));
    let crop = ["x", "y", "w", "h", "c", "r"];
    assert!(crop.iter().all(|key| !placed.contains_key(*key)));
    Ok(())
  }

  #[test]
  fn pieces_cover_with_whole_cells() {
    // (length, cell) -> pieces
    let cases = [
      (64, 8, vec![(0, 64)]),
      (59, 8, vec![(0, 56), (51, 8)]),
      (512, 14, vec![(0, 504), (498, 14)]),
      (7, 8, vec![]),
    ];
    for (length, cell, want) in cases {
      assert_eq!(pieces(length, cell), want, "{length} {cell}");
    }
  }

  /// Edge tiles on iTerm2 cover exactly what shows, in whole cells.
  #[test]
  fn crop_quirk_edge_tiles_leave_no_gap() -> Result<()> {
    let mut graphics = Graphics { quirk: true, ..Graphics::default() };
    let screen = Screen {
      columns: 80,
      rows: 20,
      cell_width: 14,
      cell_height: 30,
      origin: (0, 0),
    };
    let full = tile_sized(&mut graphics, 0, (512, 256))?;
    // Page-end tiles shorter than a cell.
    let low = tile_sized(&mut graphics, 1, (512, 20))?;
    let thin = tile_sized(&mut graphics, 2, (10, 256))?;
    // (tile, origin) -> shown x and y ranges in tile pixels; None is uncut.
    let cases = [
      (full, (0.0, 0.0), None),
      (full, (-5.0, -21.0), Some(((5, 512), (0, 235)))),
      (full, (900.0, 400.0), Some(((0, 220), (86, 256)))),
      (low, (-5.0, 100.0), Some(((5, 512), (0, 20)))),
      (thin, (300.0, -21.0), Some(((0, 10), (0, 235)))),
    ];
    let mut output = Vec::new();
    for (key, origin, want) in cases {
      output.clear();
      graphics.place_at(&key, origin, screen, 0, &mut output)?;
      let placed = commands(&output);
      let Some((xs, ys)) = want else {
        assert_eq!(placed.len(), 1);
        assert!(!placed[0].contains_key("c"));
        continue;
      };
      let mut spans = (Vec::new(), Vec::new());
      for command in &placed {
        let n = |key: &str| command[key].parse::<u32>().unwrap_or(0);
        // Whole cells, or one stretched cell for a side under a cell.
        let fits = |pixels, cells, size| {
          pixels == cells * size || (cells == 1 && pixels < size)
        };
        assert!(fits(n("w"), n("c"), 14), "{command:?}");
        assert!(fits(n("h"), n("r"), 30), "{command:?}");
        spans.0.push((n("x"), n("x") + n("w")));
        spans.1.push((n("y"), n("y") + n("h")));
      }
      assert_eq!(covered(spans.0), xs, "{origin:?}");
      assert_eq!(covered(spans.1), ys, "{origin:?}");
    }
    // A cut side under a cell draws nothing.
    output.clear();
    graphics.place_at(&full, (0.0, -250.0), screen, 0, &mut output)?;
    assert_eq!(commands(&output).len(), 0);
    Ok(())
  }

  #[test]
  fn preview_crop_matches_the_image() -> Result<()> {
    let pixels = [0; 2 * 40 * 3];
    // (quirk, origin) -> (w, h, y, cells)
    let cases = [
      (false, (0.5, 0.5), vec![("2", "40", None, None)]),
      // The screen's 144 px cut 24 rows: a cell, then one ending there.
      (
        true,
        (0.0, 120.0),
        vec![
          ("2", "16", Some("24"), Some(("1", "1"))),
          ("2", "16", Some("16"), Some(("1", "1"))),
        ],
      ),
    ];
    for (quirk, (x, y), want) in cases {
      let mut graphics = Graphics { quirk, ..Graphics::default() };
      let rect = Bounds { x0: x, y0: y, x1: x + 2.0, y1: y + 40.0 };
      let mut output = Vec::new();
      graphics.picture(1, (2, 40, &pixels), rect, SCREEN, 0, &mut output)?;
      let placed: Vec<_> = commands(&output)
        .into_iter()
        .filter(|c| has(c, &[("a", "p")]))
        .collect();
      assert_eq!(placed.len(), want.len(), "{placed:?}");
      for (command, (w, h, y, cells)) in placed.iter().zip(want) {
        assert!(has(command, &[("w", w), ("h", h)]), "{command:?}");
        assert_eq!(command.get("y").map(String::as_str), y);
        let keys = cells.map(|(c, r)| [("c", c), ("r", r)]);
        match keys {
          Some(keys) => assert!(has(command, &keys), "{command:?}"),
          None => assert!(!command.contains_key("c"), "{command:?}"),
        }
      }
    }
    Ok(())
  }

  /// The range `spans` cover, which must leave no gap.
  fn covered(mut spans: Vec<(u32, u32)>) -> (u32, u32) {
    spans.sort_unstable();
    let mut range = spans[0];
    for (start, end) in spans {
      assert!(start <= range.1, "gap before {start}");
      range.1 = range.1.max(end);
    }
    range
  }

  /// Every remembered placement names an image vpdf holds.
  fn consistent(graphics: &Graphics) {
    let held = |id: u32| {
      graphics.ids.values().any(|&held| held == id)
        || graphics.solids.values().any(|&held| held == id)
        || id == graphics.base + PREVIEW
    };
    for &(id, p) in &graphics.shown {
      assert!(held(id), "{id},{p}");
    }
  }

  #[test]
  fn late_missing_replies_keep_placements_known() -> Result<()> {
    let mut graphics = Graphics::default();
    let key = tile(&mut graphics, 0)?;
    let payload = Payload::Encoded(encode(&[255; 64 * 64 * 3])?.into());
    graphics.keep(Tile { key, payload, render_ms: 0.0 });
    let id = graphics.ids[&(0, key)];
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 };
    let mut output = Vec::new();
    let frame = |graphics: &mut Graphics| -> Result<Vec<u8>> {
      let mut output = Vec::new();
      graphics.begin(&mut output)?;
      graphics.place_at(&key, (0.0, 0.0), SCREEN, 0, &mut output)?;
      graphics.highlight(rect, [3; 4], SCREEN, 0, &mut output)?;
      graphics.end(&mut output)?;
      Ok(output)
    };
    frame(&mut graphics)?;
    let swatch = graphics.solids[&([3; 4], SWATCH.0, SWATCH.1)];
    // One eviction, two replies each, with a frame in between.
    for _ in 0..2 {
      assert!(graphics.missing(id));
      assert!(graphics.missing(swatch));
      output = frame(&mut graphics)?;
      // Deleted before the ids upload again.
      let deleted = deletes(&output);
      for id in [id, swatch] {
        assert!(deleted.contains(&("I".into(), id, None)), "{id}");
      }
      consistent(&graphics);
    }
    assert!(commands(&output).iter().any(|c| has(c, &[("a", "t")])));
    Ok(())
  }

  #[test]
  fn overlay_ids_recycle_and_run_out_quietly() -> Result<()> {
    let mut graphics = Graphics { quirk: true, ..Graphics::default() };
    let mut output = Vec::new();
    let mut sizes = 0;
    graphics.begin(&mut output)?;
    // One frame wants more sizes than there are overlay ids.
    for size in 0..PREVIEW + 10 {
      let (x1, y1) = ((size % 32 + 1) as f32, (size / 32 + 1) as f32);
      let rect = Bounds { x0: 0.0, y0: 0.0, x1, y1 };
      graphics.highlight(rect, [4; 4], SCREEN, 0, &mut output)?;
      sizes = graphics.solids.len();
    }
    graphics.end(&mut output)?;
    assert!(sizes <= PREVIEW as usize);
    // The next frame frees what it does not place, and ids stay dense.
    graphics.begin(&mut output)?;
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 3.0, y1: 1.0 };
    graphics.highlight(rect, [5; 4], SCREEN, 0, &mut output)?;
    graphics.end(&mut output)?;
    assert_eq!(graphics.solids.len(), 1);
    let id = graphics.solids[&([5; 4], 3, 1)];
    assert!((graphics.base..graphics.base + PREVIEW).contains(&id));
    assert_eq!(graphics.next_overlay(), graphics.base + PREVIEW);
    consistent(&graphics);
    Ok(())
  }

  #[test]
  fn release_deletes_each_owned_id() -> Result<()> {
    let mut graphics = Graphics::default();
    let a = tile(&mut graphics, 0)?;
    let id = graphics.ids[&(0, a)];
    let mut output = Vec::new();
    graphics.highlight(
      Bounds { x0: 0.0, y0: 0.0, x1: 4.0, y1: 4.0 },
      [2; 4],
      SCREEN,
      0,
      &mut output,
    )?;
    let swatch = graphics.solids[&([2; 4], SWATCH.0, SWATCH.1)];
    output.clear();
    release(&mut output)?;
    let deleted = deletes(&output);
    assert!(deleted.iter().all(|(d, _, p)| d == "I" && p.is_none()));
    for id in [id, swatch, graphics.base + PREVIEW] {
      assert!(deleted.iter().any(|(_, i, _)| *i == id), "{id}");
    }
    Ok(())
  }

  #[test]
  fn forgetting_a_document_frees_only_its_tiles() -> Result<()> {
    let mut graphics = Graphics::default();
    let mut output = Vec::new();
    for doc in [1, 2] {
      graphics.doc = doc;
      let key = TileKey {
        page: PageId(0),
        scale: 1,
        rotation: 0,
        x: 0,
        y: 0,
        width: 1,
        height: 1,
        crop: crate::layout::Crop::default(),
      };
      graphics.upload(
        &Tile {
          key,
          payload: Payload::Encoded(encode(&[255; 3])?.into()),
          render_ms: 0.0,
        },
        &mut output,
      )?;
    }
    graphics.forget(1);
    output.clear();
    graphics.begin(&mut output)?;
    let deleted = commands(&output)
      .into_iter()
      .filter(|c| c.get("d").is_some_and(|d| d == "I"))
      .count();
    assert_eq!(deleted, 1);
    assert_eq!(graphics.free.len(), 1);
    assert!(graphics.ids.keys().all(|key| key.0 == 2));
    Ok(())
  }
}
