use super::store::{BUDGET, publish, room, touch, trim};
use super::{Alias, Version, alias_with, hex};
use crate::document::{Heading, Target};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const LIMIT: u64 = 16 << 20;
type Row = (String, usize, Target);

pub(crate) struct OutlineCache {
  source: PathBuf,
  version: Version,
  root: PathBuf,
  key: Alias,
}

impl OutlineCache {
  pub(crate) fn open(
    root: PathBuf,
    source: &Path,
    password: &str,
    ready: impl FnMut() -> bool,
  ) -> Option<Self> {
    if !password.is_empty() {
      return None;
    }
    let mut file = File::open(source).ok()?;
    let version = Version::of(&file.metadata().ok()?);
    let content = alias_with(&mut file, ready).ok()?;
    if Version::read(source)? != version {
      return None;
    }
    let mut digest = Sha256::new();
    for part in [
      b"outline 1".as_slice(),
      content.as_slice(),
      password.as_bytes(),
      env!("VPDF_INDEX_CODE").as_bytes(),
      env!("VPDF_INDEX_CRATES").as_bytes(),
      env!("VPDF_OUTLINE_CODE").as_bytes(),
    ] {
      digest.update((part.len() as u64).to_le_bytes());
      digest.update(part);
    }
    Some(Self {
      source: source.to_owned(),
      version,
      root,
      key: digest.finalize().into(),
    })
  }

  fn path(&self) -> PathBuf {
    self.root.join(format!("{}.outline", hex(&self.key)))
  }

  pub fn read(&self) -> Option<(Vec<Heading>, bool)> {
    if Version::read(&self.source)? != self.version {
      return None;
    }
    let path = self.path();
    let file = File::open(&path).ok()?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
      return None;
    }
    let (digest, payload) = bytes.split_at_checked(32)?;
    if Sha256::digest(payload).as_slice() != digest {
      return None;
    }
    let (rows, inferred): (Vec<Row>, bool) =
      serde_json::from_slice(payload).ok()?;
    if rows.len() > 10_000 || rows.iter().any(|row| row.1 > 64) {
      return None;
    }
    let headings = rows
      .into_iter()
      .map(|(title, depth, target)| Heading { title, depth, target })
      .collect();
    touch(&path);
    Some((headings, inferred))
  }

  pub fn save(&self, headings: &[Heading], inferred: bool) {
    let _ = self.write(headings, inferred);
  }

  fn write(&self, headings: &[Heading], inferred: bool) -> crate::Result<()> {
    if Version::read(&self.source) != Some(self.version) || !room(&self.root)? {
      return Ok(());
    }
    let rows: Vec<_> =
      headings.iter().map(|h| (&h.title, h.depth, &h.target)).collect();
    let bytes = serde_json::to_vec(&(rows, inferred))?;
    if (bytes.len() + 32) as u64 <= LIMIT {
      let mut record = Sha256::digest(&bytes).to_vec();
      record.extend_from_slice(&bytes);
      publish(&self.path(), &record)?;
      trim(&self.root, BUDGET)?;
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::content_cache::alias;
  use crate::text::PageId;
  use anyhow::Context;

  #[test]
  fn outlines_roundtrip_and_invalidate() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"first PDF bytes")?;
    let cache =
      OutlineCache::open(root.path().to_owned(), &source, "", || true)
        .context("cache")?;
    let headings: Vec<_> = [
      Target::Point { page: PageId(7), x: 2.125, y: -3.5 },
      Target::Page(PageId(3)),
      Target::Uri("chapter.pdf".into()),
    ]
    .into_iter()
    .enumerate()
    .map(|(depth, target)| Heading {
      title: format!("Chapter 章 {depth}"),
      depth,
      target,
    })
    .collect();
    cache.save(&headings, false);
    let (found, inferred) = cache.read().context("outline")?;
    assert!(!inferred);
    assert_eq!(found.len(), headings.len());
    for (actual, expected) in found.iter().zip(&headings) {
      assert_eq!(actual.title, expected.title);
      assert_eq!(actual.target, expected.target);
      assert_eq!(actual.depth, expected.depth);
    }
    assert!(
      OutlineCache::open(root.path().to_owned(), &source, "wrong", || true)
        .is_none()
    );
    std::fs::write(&source, b"changed PDF bytes")?;
    assert!(cache.read().is_none());
    let changed =
      OutlineCache::open(root.path().to_owned(), &source, "", || true)
        .context("changed key")?;
    assert!(changed.read().is_none());
    Ok(())
  }

  #[test]
  fn encrypted_outlines_stay_private() -> crate::Result<()> {
    let source = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(source.path(), &["Secret title"], "")?;
    let mut doc = crate::document::Handle::open(source.path(), "")?;
    let file = crate::encryption::encrypt(&mut doc.native)?;
    let before = alias(&mut File::open(file.path())?)?;
    let root = tempfile::tempdir()?;
    let opened = crate::document::Handle::open(file.path(), "test-reader")?;
    assert!(!opened.text(PageId(0))?.lines.is_empty());
    assert!(
      OutlineCache::open(
        root.path().into(),
        file.path(),
        "test-reader",
        || true
      )
      .is_none()
    );
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    assert_eq!(alias(&mut File::open(file.path())?)?, before);
    Ok(())
  }

  #[test]
  fn outlines_share_disk_budget() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"source")?;
    let cache = OutlineCache::open(root.path().into(), &source, "", || true)
      .context("cache")?;
    cache.save(&[], true);
    assert!(cache.path().exists());
    trim(root.path(), 0)?;
    assert!(!cache.path().exists());
    Ok(())
  }

  #[test]
  fn damaged_outline_cache_misses() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"source")?;
    let cache =
      OutlineCache::open(root.path().to_owned(), &source, "", || true)
        .context("cache")?;
    cache.save(&[], true);
    assert_eq!(cache.read().map(|(_, flag)| flag), Some(true));
    let mut bytes = std::fs::read(cache.path())?;
    bytes[32] ^= 1;
    std::fs::write(cache.path(), bytes)?;
    assert!(cache.read().is_none());
    Ok(())
  }
}
