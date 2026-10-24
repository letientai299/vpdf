use super::{Kind, MEMO};
use crate::Result;
use std::cmp::Reverse;
use std::fs::File;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Disk records share one eviction budget.
pub(super) const BUDGET: u64 = 1 << 30;

/// Free disk space below which nothing is written.
const RESERVE: u64 = 2 << 30;

/// Age after which a temporary pack counts as abandoned.
pub(super) const ABANDONED: Duration = Duration::from_secs(3600);

/// Age after which an unused memo goes.
pub(super) const FORGOTTEN: Duration = Duration::from_hours(30 * 24);

/// Most memos kept, most recently used first.
pub(super) const MEMOS: usize = 4096;

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
  let mut temp = tempfile::Builder::new()
    .suffix(".tmp")
    .tempfile_in(target.parent().unwrap_or(Path::new(".")))?;
  temp.write_all(bytes)?;
  temp.persist(target).map_err(|error| error.error)?;
  Ok(())
}

/// Removes old records and abandoned writes.
pub(super) fn trim(root: &Path, budget: u64) -> Result<()> {
  let mut packs = Vec::new();
  let mut memos = Vec::new();
  let now = SystemTime::now();
  for entry in std::fs::read_dir(root)?.flatten() {
    let Ok(meta) = entry.metadata() else { continue };
    let modified = meta.modified().unwrap_or(now);
    let path = entry.path();
    match path.extension().and_then(|extension| extension.to_str()) {
      Some("id") => memos.push((modified, meta.len(), path)),
      Some("tmp")
        if now.duration_since(modified).unwrap_or_default() > ABANDONED =>
      {
        drop(std::fs::remove_file(path));
      }
      Some(ext)
        if ext == "pack" || Kind::ALL.iter().any(|kind| kind.ext() == ext) =>
      {
        packs.push((modified, meta.len(), path));
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
  // Records name their PDF only by a digest, so memos outlive packs.
  memos.sort_by_key(|(modified, ..)| Reverse(*modified));
  for (kept, (modified, len, memo)) in memos.into_iter().enumerate() {
    let unused = now.duration_since(modified).unwrap_or_default() > FORGOTTEN;
    if unused || len != MEMO || kept >= MEMOS {
      drop(std::fs::remove_file(memo));
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Read;
  use std::os::unix::fs::PermissionsExt;

  #[test]
  fn replacement_cleanup() -> Result<()> {
    let root = tempfile::tempdir()?;
    let cache = root.path().join("cache");
    create(&cache)?;
    assert_eq!(cache.metadata()?.permissions().mode() & 0o777, 0o700);
    let target = cache.join("record.pack");
    publish(&target, b"old")?;
    let mut retained = File::open(&target)?;
    publish(&target, b"new record")?;
    assert_eq!(std::fs::read(&target)?, b"new record");
    let mut bytes = Vec::new();
    retained.read_to_end(&mut bytes)?;
    assert_eq!(bytes, b"old");
    assert_eq!(target.metadata()?.permissions().mode() & 0o777, 0o600);
    std::fs::remove_file(&target)?;
    std::fs::create_dir(&target)?;
    assert!(publish(&target, b"failed").is_err());
    assert!(target.is_dir());
    assert_eq!(std::fs::read_dir(&cache)?.count(), 1);
    Ok(())
  }
}
