use crate::Result;
use crate::cache::Cache;
use crate::document::MIB;
use crate::layout::{Layout, Screen};
use crate::terminal::graphics;
use crate::text::Bounds;
use crate::tiles::{Tile, TileKey, encode};
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::sync::{Arc, OnceLock};

const BLOCK: u32 = 1 << 16;
const OVERLAYS: u32 = 1024;
/// Last overlay id holds the link preview across frames.
const PREVIEW: u32 = OVERLAYS - 1;
// Four readers fit Kitty's 320 MB quota.
const BUDGET: usize = 64 * MIB;
/// Encoded tiles kept after rendering: prefetched tiles wait here until
/// they scroll into view, and evicted or lost images upload again without
/// rendering. Text tiles compress to about 4% of their RGB size.
const STORED: usize = 48 * MIB;

/// Tiles of one document; tabs share one Kitty budget.
type Key = (u32, TileKey);

pub struct Graphics {
  /// Document whose tiles `upload` and `place` address.
  pub doc: u32,
  resident: Cache<Key, u32>,
  stored: Cache<Key, Arc<str>>,
  ids: HashMap<Key, u32>,
  free: Vec<u32>,
  stale: Vec<u32>,
  /// Serial of the uploaded preview; zero when none.
  preview: u64,
  /// Whether the last frame placed the preview.
  pictured: bool,
  base: u32,
  next: u32,
  overlay: u32,
  pub uploaded: u64,
}

impl Default for Graphics {
  fn default() -> Self {
    let base = session();
    Self {
      doc: 0,
      resident: Cache::new(BUDGET),
      stored: Cache::new(STORED),
      ids: HashMap::new(),
      free: Vec::new(),
      stale: Vec::new(),
      preview: 0,
      pictured: false,
      base,
      next: base + OVERLAYS,
      overlay: 0,
      uploaded: 0,
    }
  }
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

/// Deletes this process's images and placements.
pub fn release(out: &mut impl Write) -> Result<()> {
  let base = session();
  graphics(out, &format!("a=d,d=R,x={base},y={},q=2", base + (BLOCK - 1)), "")
}

impl Graphics {
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
    *self = Self { doc: self.doc, stored, ..Self::default() };
  }

  /// Keeps an encoded tile until a frame places it.
  pub fn keep(&mut self, tile: Tile) {
    let weight = tile.encoded.len();
    self.stored.insert((self.doc, tile.key), Arc::from(tile.encoded), weight);
  }

  pub fn upload(&mut self, tile: &Tile, out: &mut impl Write) -> Result<()> {
    if !self.contains(&tile.key) {
      self.send((self.doc, tile.key), &tile.encoded, out)?;
    }
    Ok(())
  }

  /// Transmits a tile and returns its image id.
  fn send(
    &mut self,
    key: Key,
    encoded: &str,
    out: &mut impl Write,
  ) -> Result<u32> {
    let id = self.free.pop().unwrap_or_else(|| {
      self.next += 1;
      self.next - 1
    });
    let tile = key.1;
    transmit(out, id, tile.width, tile.height, 24, encoded)?;
    // Weight floor keeps residency within tile ids.
    let weight = ((tile.width * tile.height * 3) as usize)
      .max(BUDGET.div_ceil((BLOCK - OVERLAYS) as usize));
    for key in self.resident.insert(key, id, weight) {
      if let Some(id) = self.ids.remove(&key) {
        graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
        self.free.push(id);
      }
    }
    self.ids.insert(key, id);
    self.uploaded += encoded.len() as u64;
    Ok(id)
  }

  pub fn begin(&mut self, out: &mut impl Write) -> Result<()> {
    let tiles = self.base + OVERLAYS;
    let last = self.base + (BLOCK - 1);
    graphics(out, &format!("a=d,d=r,x={tiles},y={last},q=2"), "")?;
    for id in self.stale.drain(..) {
      graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
      self.free.push(id);
    }
    if self.overlay > 0 {
      let (first, last) = (self.base, self.base + self.overlay - 1);
      graphics(out, &format!("a=d,d=R,x={first},y={last},q=2"), "")?;
    }
    self.overlay = 0;
    if self.preview != 0 {
      // Keep the pixels while shown; free them once a frame skips it.
      let id = self.base + PREVIEW;
      let delete = if self.pictured { "i" } else { "I" };
      graphics(out, &format!("a=d,d={delete},i={id},q=2"), "")?;
      if !self.pictured {
        self.preview = 0;
      }
    }
    self.pictured = false;
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
    let Some(rect) = clip(rect, screen) else { return Ok(()) };
    let control = format!(
      "a=p,i={id},q=2,C=1,z=2,w={},h={}",
      rect.width() as u32,
      rect.height() as u32
    );
    placement(out, &(control + &offsets(rect, screen)), rect, left, screen)
  }

  pub fn place(
    &mut self,
    key: &TileKey,
    layout: &Layout,
    scroll: f32,
    left: u16,
    out: &mut impl Write,
  ) -> Result<bool> {
    let x = key.x as f32 - layout.pan;
    let y = layout.pages[key.page.0].top + key.y as f32 - scroll;
    self.place_at(key, (x, y), layout.screen, left, out)
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
    let id = match self.resident.get(&(self.doc, *key)) {
      Some(&id) => id,
      None => {
        let kept = self.stored.get(&(self.doc, *key)).map(Arc::clone);
        let Some(encoded) = kept else { return Ok(false) };
        self.send((self.doc, *key), &encoded, out)?
      }
    };
    let rect = Bounds {
      x0: x,
      y0: y,
      x1: x + key.width as f32,
      y1: y + key.height as f32,
    };
    let Some(clip) = clip(rect, screen) else { return Ok(true) };
    placement(
      out,
      &(format!(
        "a=p,i={id},p=1,q=2,C=1,z=-2,x={},y={},w={},h={}",
        (clip.x0 - x) as u32,
        (clip.y0 - y) as u32,
        clip.width() as u32,
        clip.height() as u32,
      ) + &offsets(clip, screen)),
      clip,
      left,
      screen,
    )?;
    Ok(true)
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
    if self.overlay >= PREVIEW {
      return Ok(());
    }
    let id = self.base + self.overlay;
    self.overlay += 1;
    transmit(out, id, 1, 1, 32, &encode(&color)?)?;
    let rows = screen.rows.saturating_sub(1);
    let control =
      format!("a=p,i={id},q=2,C=1,z=1,c={},r={rows}", screen.columns);
    let rect = Bounds { x0: 0.0, y0: 0.0, x1: 1.0, y1: 1.0 };
    placement(out, &control, rect, left, screen)
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
    if self.overlay >= PREVIEW {
      return Ok(());
    }
    let width = rect.width().ceil().max(1.0) as i32;
    let height = rect.height().ceil().max(1.0) as i32;
    let pixels = color.repeat((width * height) as usize);
    let id = self.base + self.overlay;
    self.overlay += 1;
    transmit(out, id, width, height, 32, &encode(&pixels)?)?;
    placement(
      out,
      &(format!("a=p,i={id},q=2,C=1,z={z}") + &offsets(rect, screen)),
      rect,
      left,
      screen,
    )?;
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

fn offsets(rect: Bounds, screen: Screen) -> String {
  format!(
    ",X={},Y={}",
    rect.x0 as u32 % u32::from(screen.cell_width),
    rect.y0 as u32 % u32::from(screen.cell_height)
  )
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
  let mut chunks = encoded.as_bytes().chunks(4096).peekable();
  let mut first = true;
  while let Some(chunk) = chunks.next() {
    let more = u8::from(chunks.peek().is_some());
    let control = if first {
      format!("a=t,f={format},o=z,i={id},q=2,s={width},v={height},m={more}")
    } else {
      format!("m={more},q=2")
    };
    graphics(out, &control, std::str::from_utf8(chunk)?)?;
    first = false;
  }
  Ok(())
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
      resident: Cache::new(2 * BUDGET.div_ceil((BLOCK - OVERLAYS) as usize)),
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
        &Tile { key, encoded: encode(&[255; 3])?, render_ms: 0.0 },
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
        assert!(["I", "r", "R"].contains(&delete.as_str()), "{delete}");
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
        &Tile { key, encoded: encode(&[255; 3])?, render_ms: 0.0 },
        &mut output,
      )?;
    }
    graphics.forget(1);
    output.clear();
    graphics.begin(&mut output)?;
    let deleted: Vec<_> = commands(&output)
      .into_iter()
      .filter(|c| c.get("d").is_some_and(|d| d == "I"))
      .collect();
    assert_eq!(deleted.len(), 1);
    assert_eq!(graphics.free.len(), 1);
    assert!(graphics.ids.keys().all(|key| key.0 == 2));
    Ok(())
  }
}
