//! Tile pixels in a file the terminal reads, so neither side compresses
//! or base64-encodes them.
//!
//! Kitty reads an image from a file region named in the command
//! (`t=f`, `S=`, `O=`). Tiles take fixed slots of one file in turn, so
//! a slot is reused only after a full ring of later tiles; Kitty reads a
//! slot when it parses the command, long before. The file lives in the
//! cache directory, which is on disk where `/tmp` may be RAM, and goes
//! away on exit; files of readers that died are removed at start.
use crate::caps::{Graphics, Host};
use crate::payload::{TILE_HEIGHT, TILE_WIDTH};
use crate::{MIB, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fs::File;
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use vpdf_protocol::decode::Reply;

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

#[derive(Debug)]
pub struct Spool {
  file: RwLock<Option<File>>,
  path: PathBuf,
  /// Base64 of the path, every transmission's payload.
  pub payload: String,
  /// Slots written so far.
  head: AtomicU64,
}

impl Spool {
  pub fn create(dir: &Path) -> Result<Self> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    std::fs::create_dir_all(dir)?;
    remove_stale(dir);
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!("tiles-{}-{serial}", std::process::id()));
    let file = File::options()
      .read(true)
      .write(true)
      .create_new(true)
      .mode(0o600)
      .open(&path)?;
    let payload = STANDARD.encode(path.as_os_str().as_encoded_bytes());
    Ok(Self {
      file: RwLock::new(Some(file)),
      path,
      payload,
      head: AtomicU64::new(0),
    })
  }

  /// Writes `pixels` to the next slot.
  pub fn write(&self, pixels: &[u8]) -> Result<Slot> {
    anyhow::ensure!(pixels.len() as u64 <= SLOT, "Tile larger than a slot");
    let file = self.file.read().unwrap_or_else(PoisonError::into_inner);
    let file = file.as_ref().ok_or_else(|| anyhow::anyhow!("Spool closed"))?;
    let serial = self.head.fetch_add(1, Ordering::Relaxed);
    file.write_all_at(pixels, offset(serial))?;
    Ok(Slot { serial, len: pixels.len() })
  }

  /// Whether a later tile has not taken `slot`'s place.
  pub fn holds(&self, slot: Slot) -> bool {
    self.file.read().unwrap_or_else(PoisonError::into_inner).is_some()
      && slot.serial + slots() > self.head.load(Ordering::Relaxed)
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

static SPOOL: Mutex<Option<Arc<Spool>>> = Mutex::new(None);

pub fn get() -> Option<Arc<Spool>> {
  SPOOL.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

impl Spool {
  /// Waits for writers, then removes the file.
  pub fn close(&self) {
    let mut file = self.file.write().unwrap_or_else(PoisonError::into_inner);
    if file.take().is_some() {
      drop(std::fs::remove_file(&self.path));
    }
  }
}

impl Drop for Spool {
  fn drop(&mut self) {
    self.close();
  }
}

/// Requires valid reads and rejected out-of-bounds offsets. Other
/// hosts and terminals without file transmission keep sending encoded
/// tiles. Only a terminal that answered the graphics query directly gets
/// the probe: tmux answers the device query itself, so the terminal's
/// late answer would reach the active pane as keys.
pub fn start(id: u32) -> Result<bool> {
  if get().is_some() {
    return Ok(true);
  }
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
  // Detect terminals that ignore file offsets.
  let control = format!("a=q,i={},f=24,s=1,v=1,t=f,S=3,O=3", id ^ 1);
  crate::terminal::graphics(&mut request, &control, &spool.payload)?;
  let replies =
    crate::input::ask(&mut std::io::stdout(), &request, crate::caps::WAIT)?;
  if !regions(&replies, id) {
    return Ok(false);
  }
  *SPOOL.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(spool));
  Ok(true)
}

/// Removes the spool file; the terminal has read every slot by exit.
pub fn remove() {
  if let Some(spool) =
    SPOOL.lock().unwrap_or_else(PoisonError::into_inner).take()
  {
    spool.close();
  }
}

fn regions(replies: &[Reply], id: u32) -> bool {
  let loaded = replies.iter().any(|reply| {
    matches!(reply, Reply::Graphics { id: Some(answered), message, .. }
      if *answered == id && message == "OK")
  });
  let rejected = replies.iter().any(|reply| {
    matches!(reply, Reply::Graphics { id: Some(answered), message, .. }
      if *answered == (id ^ 1) && message.starts_with('E'))
  });
  loaded && rejected
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
      .and_then(|suffix| suffix.split('-').next())
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
  fn close_waits_for_writers() -> Result<()> {
    use std::sync::mpsc;
    use std::time::Duration;
    let dir = tempfile::tempdir()?;
    let spool = Arc::new(Spool::create(dir.path())?);
    let slot = spool.write(&[1, 2, 3])?;
    let writing = spool.file.read().unwrap();
    let (ready, started) = mpsc::channel();
    let (done, closed) = mpsc::channel();
    let worker = Arc::clone(&spool);
    let thread = std::thread::spawn(move || {
      ready.send(()).unwrap();
      worker.close();
      done.send(()).unwrap();
    });
    started.recv_timeout(Duration::from_secs(5))?;
    assert!(closed.recv_timeout(Duration::from_millis(20)).is_err());
    assert!(spool.path.exists());
    drop(writing);
    closed.recv_timeout(Duration::from_secs(5))?;
    thread.join().unwrap();
    assert!(!spool.path.exists());
    assert!(!spool.holds(slot));
    assert!(spool.write(&[4, 5, 6]).is_err());
    Ok(())
  }

  #[test]
  fn payload_retains_its_spool() -> Result<()> {
    use crate::payload::Encoder;
    let dir = tempfile::tempdir()?;
    let spool = Arc::new(Spool::create(dir.path())?);
    let path = spool.path.clone();
    let encoder = Encoder::new(Some(spool));
    let payload = encoder.encode(&[1, 2, 3])?;
    drop(encoder);
    assert!(path.exists());
    drop(payload);
    assert!(!path.exists());
    Ok(())
  }

  #[test]
  fn spools_keep_separate_files() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let first = Spool::create(dir.path())?;
    first.write(&[1, 2, 3])?;
    let second = Spool::create(dir.path())?;
    second.write(&[4, 5, 6])?;
    assert_ne!(first.path, second.path);
    assert_eq!(std::fs::read(&first.path)?, [1, 2, 3]);
    drop(second);
    assert!(first.path.exists());
    Ok(())
  }

  #[test]
  fn slots_hold_until_the_ring_wraps() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let spool = Spool::create(dir.path())?;
    let first = spool.write(&[1, 2, 3])?;
    assert_eq!(Spool::keys(first), "t=f,S=3,O=0");
    let mut buffer = [0; 3];
    spool
      .file
      .read()
      .unwrap()
      .as_ref()
      .unwrap()
      .read_exact_at(&mut buffer, 0)?;
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
  fn probe_checks_file_offsets() {
    let answer = |id, message: &str| Reply::Graphics {
      id: Some(id),
      placement: None,
      message: message.into(),
    };
    let da1 = Reply::Attributes(vec![62, 22]);
    let cases = [
      (vec![answer(7, "OK"), answer(6, "EINVAL:short file")], true),
      (vec![answer(7, "OK"), answer(6, "OK")], false),
      (vec![answer(7, "OK")], false),
      (vec![answer(7, "EBADF:x"), answer(6, "EINVAL:x")], false),
      (vec![answer(70, "OK"), answer(6, "EINVAL:x")], false),
      (vec![answer(7, "OK"), answer(60, "EINVAL:x")], false),
      (vec![da1], false),
    ];
    for (replies, expected) in cases {
      assert_eq!(regions(&replies, 7), expected, "{replies:?}");
    }
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
