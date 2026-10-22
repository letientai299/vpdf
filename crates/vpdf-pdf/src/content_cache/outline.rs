use super::record::Record;
use crate::document::{Heading, Target};
use std::path::{Path, PathBuf};

type Row = (String, usize, Target);

pub(crate) struct OutlineCache(Record);

impl OutlineCache {
  pub(crate) fn open(
    root: PathBuf,
    source: &Path,
    password: &str,
    ready: impl FnMut() -> bool,
  ) -> Option<Self> {
    Record::open(
      root,
      source,
      password,
      "outline",
      env!("VPDF_OUTLINE_CODE"),
      ready,
    )
    .map(Self)
  }

  pub fn read(&self) -> Option<(Vec<Heading>, bool)> {
    let bytes = self.0.read()?;
    let (rows, inferred): (Vec<Row>, bool) =
      serde_json::from_slice(&bytes).ok()?;
    if rows.len() > 10_000 || rows.iter().any(|row| row.1 > 64) {
      return None;
    }
    let headings = rows
      .into_iter()
      .map(|(title, depth, target)| Heading { title, depth, target })
      .collect();
    Some((headings, inferred))
  }

  pub fn save(&self, headings: &[Heading], inferred: bool) {
    let rows: Vec<_> =
      headings.iter().map(|h| (&h.title, h.depth, &h.target)).collect();
    if let Ok(bytes) = serde_json::to_vec(&(rows, inferred)) {
      let _ = self.0.write(&bytes);
    }
  }

  #[cfg(test)]
  fn path(&self) -> PathBuf {
    self.0.path()
  }
}

#[cfg(test)]
mod tests {
  use super::super::store::trim;
  use super::*;
  use crate::content_cache::alias;
  use crate::text::PageId;
  use anyhow::Context;
  use std::fs::File;

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
