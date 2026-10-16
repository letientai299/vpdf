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
use std::sync::OnceLock;

const BLOCK: u32 = 1 << 16;
const OVERLAYS: u32 = 1024;
// Four readers fit Kitty's 320 MB quota.
const BUDGET: usize = 64 * MIB;

/// Tiles of one document; tabs share one Kitty budget.
type Key = (u32, TileKey);

pub struct Graphics {
  /// Document whose tiles `upload` and `place` address.
  pub doc: u32,
  resident: Cache<Key, u32>,
  ids: HashMap<Key, u32>,
  free: Vec<u32>,
  stale: Vec<u32>,
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
      ids: HashMap::new(),
      free: Vec::new(),
      stale: Vec::new(),
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

  /// Deletes a document's tiles at the next frame.
  pub fn forget(&mut self, doc: u32) {
    let keys: Vec<_> =
      self.ids.keys().filter(|key| key.0 == doc).copied().collect();
    for key in keys {
      self.resident.remove(&key);
      self.stale.extend(self.ids.remove(&key));
    }
  }

  pub fn upload(&mut self, tile: &Tile, out: &mut impl Write) -> Result<()> {
    if self.contains(&tile.key) {
      return Ok(());
    }
    let id = self.free.pop().unwrap_or_else(|| {
      self.next += 1;
      self.next - 1
    });
    transmit(out, id, tile.key.width, tile.key.height, 24, &tile.encoded)?;
    // Weight floor keeps residency within tile ids.
    let weight = ((tile.key.width * tile.key.height * 3) as usize)
      .max(BUDGET.div_ceil((BLOCK - OVERLAYS) as usize));
    let key = (self.doc, tile.key);
    for key in self.resident.insert(key, id, weight) {
      if let Some(id) = self.ids.remove(&key) {
        graphics(out, &format!("a=d,d=I,i={id},q=2"), "")?;
        self.free.push(id);
      }
    }
    self.ids.insert(key, id);
    self.uploaded += tile.encoded.len() as u64;
    Ok(())
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
    Ok(())
  }

  pub fn place(
    &mut self,
    key: &TileKey,
    layout: &Layout,
    scroll: f32,
    left: u16,
    out: &mut impl Write,
  ) -> Result<bool> {
    let Some(&id) = self.resident.get(&(self.doc, *key)) else {
      return Ok(false);
    };
    let x = key.x as f32 - layout.pan;
    let y = layout.pages[key.page.0].top + key.y as f32 - scroll;
    let rect = Bounds {
      x0: x,
      y0: y,
      x1: x + key.width as f32,
      y1: y + key.height as f32,
    };
    let Some(clip) = clip(rect, layout.screen) else { return Ok(true) };
    placement(
      out,
      &(format!(
        "a=p,i={id},p=1,q=2,C=1,z=-2,x={},y={},w={},h={}",
        (clip.x0 - x) as u32,
        (clip.y0 - y) as u32,
        clip.width() as u32,
        clip.height() as u32,
      ) + &offsets(clip, layout.screen)),
      clip,
      left,
      layout.screen,
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
    if self.overlay >= OVERLAYS {
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
