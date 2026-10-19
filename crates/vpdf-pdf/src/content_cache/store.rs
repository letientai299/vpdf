use super::path;
use crate::Result;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Writes so far, naming each writer's temporary pack.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// Packs and OCR output kept before the least recently used go.
pub(super) const BUDGET: u64 = 1 << 30;

/// Free disk space below which nothing is written.
const RESERVE: u64 = 2 << 30;

/// Age after which a temporary pack counts as abandoned.
pub(super) const ABANDONED: Duration = Duration::from_secs(3600);

/// Age after which an unused memo goes.
pub(super) const FORGOTTEN: Duration = Duration::from_hours(30 * 24);

/// Makes `root` and tells whether it has room to write.
pub(super) fn room(root: &Path) -> Result<bool> {
  create(root)?;
  let stats = rustix::fs::statvfs(root)?;
  Ok(stats.f_bavail.saturating_mul(stats.f_frsize) >= RESERVE)
}

/// Marks `path` recently used, for trimming.
pub(super) fn touch(path: &Path) {
  drop(File::open(path).and_then(|file| file.set_modified(SystemTime::now())));
}

/// Makes `root`; pages hold the PDF's text, so only its owner reads them.
pub(super) fn create(root: &Path) -> Result<()> {
  std::fs::DirBuilder::new().recursive(true).mode(0o700).create(root)?;
  Ok(())
}

/// Writes `bytes` to `target` whole: writers race by renaming whole
/// files, so readers never see a partial one.
pub(super) fn publish(target: &Path, bytes: &[u8]) -> Result<()> {
  let write = WRITES.fetch_add(1, Ordering::Relaxed);
  let temp =
    target.with_extension(format!("{}-{write}.tmp", std::process::id()));
  let written = File::options()
    .write(true)
    .create_new(true)
    .mode(0o600)
    .open(&temp)
    .and_then(|mut file| file.write_all(bytes))
    .and_then(|()| std::fs::rename(&temp, target));
  if let Err(error) = written {
    drop(std::fs::remove_file(&temp));
    return Err(error.into());
  }
  Ok(())
}

/// Deletes the least recently used packs and OCR output until the rest
/// fit `budget`, memos of deleted packs or unused for a month, and
/// temporary files a crashed writer left.
pub(super) fn trim(root: &Path, budget: u64) -> Result<()> {
  let mut packs = Vec::new();
  let mut memos = Vec::new();
  let now = SystemTime::now();
  for entry in std::fs::read_dir(root)?.flatten() {
    let Ok(meta) = entry.metadata() else { continue };
    let modified = meta.modified().unwrap_or(now);
    let path = entry.path();
    match path.extension().and_then(|extension| extension.to_str()) {
      Some("pack" | "ocr") => packs.push((modified, meta.len(), path)),
      Some("id") => memos.push((modified, path)),
      Some("tmp")
        if now.duration_since(modified).unwrap_or_default() > ABANDONED =>
      {
        drop(std::fs::remove_file(path));
      }
      _ => {}
    }
  }
  let mut total: u64 = packs.iter().map(|(_, len, _)| len).sum();
  packs.sort_by_key(|(modified, ..)| *modified);
  for (_, len, path) in packs {
    if total <= budget {
      break;
    }
    if std::fs::remove_file(path).is_ok() {
      total -= len;
    }
  }
  for (modified, memo) in memos {
    let bytes = std::fs::read(&memo).unwrap_or_default();
    let alias = bytes.last_chunk::<32>();
    let unused = now.duration_since(modified).unwrap_or_default() > FORGOTTEN;
    if unused || alias.is_none_or(|alias| !path(root, alias).exists()) {
      drop(std::fs::remove_file(memo));
    }
  }
  Ok(())
}
