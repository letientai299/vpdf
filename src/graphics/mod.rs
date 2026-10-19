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
  cell_offsets: bool,
  /// `q` of placements: 1 reports `ENOENT` for an evicted image, which
  /// tmux would hand to the active pane as keys.
  quiet: u8,
  pub uploaded: u64,
}

impl Default for Graphics {
  fn default() -> Self {
    let base = session();
    let (quirk, cell_offsets, quiet) = traits();
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
      cell_offsets,
      quiet,
      uploaded: 0,
    }
  }
}

/// iTerm2's crop quirk and the placements' `q`, from the probe. Panes
/// lend `Graphics` through `Default` on every loop, so this runs once.
fn traits() -> (bool, bool, u8) {
  static TRAITS: OnceLock<(bool, bool, u8)> = OnceLock::new();
  // Before the probe, as in benches, nothing is cached.
  let Some(caps) = crate::caps::get() else { return (false, false, 2) };
  *TRAITS.get_or_init(|| {
    let quirk = caps.profile() == Profile::CropQuirk;
    let quiet = if matches!(caps.host, Host::Direct(_)) { 1 } else { 2 };
    (quirk, caps.cell_offsets(), quiet)
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

use placement::clip;
use protocol::transmit;

mod images;
mod overlays;
mod placement;
mod protocol;
#[cfg(test)]
mod tests;
