//! Tile pixels in a file the terminal reads, so neither side compresses
//! or base64-encodes them.
//!
//! Kitty reads an image from a file region named in the command
//! (`t=f`, `S=`, `O=`). Tiles take fixed slots of one file in turn, so
//! a slot is reused only after a full ring of later tiles; Kitty reads a
//! slot when it parses the command, long before. The file lives in the
//! cache directory, which is on disk where `/tmp` may be RAM, and goes
//! away on exit; files of readers that died are removed at start.
use crate::Result;
use crate::caps::{Graphics, Host};
use crate::decode::Reply;
use crate::document::MIB;
use crate::tiles::{TILE_HEIGHT, TILE_WIDTH};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fs::File;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes of the largest tile.
const SLOT: u64 = (TILE_WIDTH * TILE_HEIGHT * 3) as u64;
/// Size of the ring: about ten screens of 1969x2142 pixels.
pub const BYTES: usize = 128 * MIB;
/// A tile's region of the spool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Slot {
  serial: u64,
  pub len: usize,
}

pub struct Spool {
  file: File,
  path: PathBuf,
  /// Base64 of the path, every transmission's payload.
  pub payload: String,
  /// Slots written so far.
  head: AtomicU64,
}

impl Spool {
  fn create(dir: &Path) -> Result<Self> {
    std::fs::create_dir_all(dir)?;
    remove_stale(dir);
    let path = dir.join(format!("tiles-{}", std::process::id()));
    let file = File::options()
      .read(true)
      .write(true)
      .create(true)
      .truncate(true)
      .mode(0o600)
      .open(&path)?;
    let payload = STANDARD.encode(path.as_os_str().as_encoded_bytes());
    Ok(Self { file, path, payload, head: AtomicU64::new(0) })
  }

  /// Writes `pixels` to the next slot.
  pub fn write(&self, pixels: &[u8]) -> Result<Slot> {
    anyhow::ensure!(pixels.len() as u64 <= SLOT, "Tile larger than a slot");
    let serial = self.head.fetch_add(1, Ordering::Relaxed);
    self.file.write_all_at(pixels, offset(serial))?;
    Ok(Slot { serial, len: pixels.len() })
  }

  /// Whether a later tile has not taken `slot`'s place.
  pub fn holds(&self, slot: Slot) -> bool {
    slot.serial + slots() > self.head.load(Ordering::Relaxed)
  }

  /// Transmission keys that point the terminal at `slot`.
  pub fn keys(slot: Slot) -> String {
    format!("t=f,S={},O={}", slot.len, offset(slot.serial))
  }
}

fn slots() -> u64 {
  BYTES as u64 / SLOT
}

fn offset(serial: u64) -> u64 {
  serial % slots() * SLOT
}

static SPOOL: OnceLock<Spool> = OnceLock::new();

/// The spool, once `start` found that the terminal reads it.
pub fn get() -> Option<&'static Spool> {
  SPOOL.get()
}

/// Uses a spool when the terminal reads it: a one-pixel probe written to
/// it must load before the terminal answers a later device query. Other
/// hosts and terminals without file transmission keep sending encoded
/// tiles. Only a terminal that answered the graphics query directly gets
/// the probe: tmux answers the device query itself, so the terminal's
/// late answer would reach the active pane as keys.
pub fn start(id: u32) -> Result<bool> {
  let direct = Some(&Host::Direct(Graphics::Ok));
  if crate::caps::get().map(|caps| &caps.host) != direct {
    return Ok(false);
  }
  let Some(dir) = directory() else { return Ok(false) };
  let spool = Spool::create(&dir)?;
  let slot = spool.write(&[255; 3])?;
  let control = format!("a=q,i={id},f=24,s=1,v=1,{}", Spool::keys(slot));
  let mut request = Vec::new();
  crate::terminal::graphics(&mut request, &control, &spool.payload)?;
  let replies =
    crate::input::ask(&mut std::io::stdout(), &request, crate::caps::WAIT)?;
  if !loaded(&replies, id) {
    drop(std::fs::remove_file(&spool.path));
    return Ok(false);
  }
  Ok(SPOOL.set(spool).is_ok())
}

/// Removes the spool file; the terminal has read every slot by exit.
pub fn remove() {
  if let Some(spool) = get() {
    drop(std::fs::remove_file(&spool.path));
  }
}

/// Whether `replies` hold Kitty's OK for image `id`.
fn loaded(replies: &[Reply], id: u32) -> bool {
  replies.iter().any(|reply| {
    matches!(reply, Reply::Graphics { id: Some(answered), message, .. }
      if *answered == id && message == "OK")
  })
}

/// `$XDG_CACHE_HOME/vpdf`, else under `~/.cache`.
fn directory() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_CACHE_HOME")
    .filter(|dir| !dir.is_empty())
    .map(PathBuf::from)
    .or_else(|| {
      std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"))
    })?;
  Some(base.join("vpdf"))
}

/// Removes spool files whose reader no longer runs.
fn remove_stale(dir: &Path) {
  let Ok(entries) = std::fs::read_dir(dir) else { return };
  for entry in entries.flatten() {
    let name = entry.file_name();
    let pid = name
      .to_str()
      .and_then(|name| name.strip_prefix("tiles-"))
      .and_then(|pid| pid.parse::<i32>().ok())
      .and_then(rustix::process::Pid::from_raw);
    let gone = |pid| {
      rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH)
    };
    if pid.is_some_and(gone) {
      drop(std::fs::remove_file(entry.path()));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn slots_hold_until_the_ring_wraps() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let spool = Spool::create(dir.path())?;
    let first = spool.write(&[1, 2, 3])?;
    assert_eq!(Spool::keys(first), "t=f,S=3,O=0");
    let mut buffer = [0; 3];
    spool.file.read_exact_at(&mut buffer, 0)?;
    assert_eq!(buffer, [1, 2, 3]);
    for _ in 1..slots() {
      assert!(spool.holds(first));
      spool.write(&[0; 3])?;
    }
    assert!(!spool.holds(first), "the ring reused the first slot");
    assert_eq!(Spool::keys(spool.write(&[0; 3])?), "t=f,S=3,O=0");
    Ok(())
  }

  #[test]
  fn probe_needs_the_ok_for_its_image() {
    let answer = |id, message: &str| Reply::Graphics {
      id: Some(id),
      placement: None,
      message: message.into(),
    };
    let da1 = Reply::Attributes(vec![62, 22]);
    assert!(loaded(&[answer(7, "OK"), da1.clone()], 7));
    assert!(!loaded(&[answer(7, "EBADF:x"), da1.clone()], 7));
    assert!(!loaded(&[answer(70, "OK")], 7));
    assert!(!loaded(&[da1], 7));
  }

  #[test]
  fn stale_files_go_live_ones_stay() -> Result<()> {
    let dir = tempfile::tempdir()?;
    // Pid 1 runs as long as the system does; i32::MAX never runs.
    for pid in [1, i32::MAX] {
      File::create(dir.path().join(format!("tiles-{pid}")))?;
    }
    remove_stale(dir.path());
    assert!(dir.path().join("tiles-1").exists());
    assert!(!dir.path().join(format!("tiles-{}", i32::MAX)).exists());
    Ok(())
  }
}
