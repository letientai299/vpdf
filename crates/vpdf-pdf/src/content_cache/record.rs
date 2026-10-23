use super::store::{BUDGET, publish, room, touch, trim};
use super::{Alias, Version, alias_with, digest, hex};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const LIMIT: u64 = 16 << 20;

/// What a record holds, which names its file extension.
#[derive(Clone, Copy)]
pub enum Kind {
  Figures,
  Outline,
  Annotations,
  Links,
  Recent,
  Ocr,
}

impl Kind {
  pub(super) const ALL: [Self; 6] = [
    Self::Figures,
    Self::Outline,
    Self::Annotations,
    Self::Links,
    Self::Recent,
    Self::Ocr,
  ];

  pub(super) fn ext(self) -> &'static str {
    match self {
      Self::Figures => "figures",
      Self::Outline => "outline",
      Self::Annotations => "annotations",
      Self::Links => "links",
      Self::Recent => "recent",
      Self::Ocr => "ocr",
    }
  }
}

/// A checksummed blob under a key.
pub struct Record {
  root: PathBuf,
  path: PathBuf,
  /// The PDF the key derives from, as hashed; a change misses.
  source: Option<(PathBuf, Version)>,
}

impl Record {
  /// None for passwords; plaintext stays off disk.
  pub fn open(
    root: PathBuf,
    source: &Path,
    password: &str,
    kind: Kind,
    code: &str,
    ready: impl FnMut() -> bool,
  ) -> Option<Self> {
    if !super::cacheable(source, password) {
      return None;
    }
    let mut file = File::open(source).ok()?;
    let version = Version::of(&file.metadata().ok()?);
    let content = alias_with(&mut file, ready).ok()?;
    if Version::read(source)? != version {
      return None;
    }
    let key: Alias = digest([
      format!("{} 1", kind.ext()).as_bytes(),
      content.as_slice(),
      password.as_bytes(),
      env!("VPDF_INDEX_CODE").as_bytes(),
      env!("VPDF_INDEX_CRATES").as_bytes(),
      code.as_bytes(),
    ])
    .finalize()
    .into();
    let source = Some((source.to_owned(), version));
    Some(Self { source, ..Self::keyed(root, kind, &key) })
  }

  pub(super) fn keyed(root: PathBuf, kind: Kind, key: &Alias) -> Self {
    let path = root.join(format!("{}.{}", hex(key), kind.ext()));
    Self { root, path, source: None }
  }

  fn current(&self) -> bool {
    self
      .source
      .as_ref()
      .is_none_or(|(path, version)| Version::read(path) == Some(*version))
  }

  #[cfg(test)]
  pub(crate) fn path(&self) -> &Path {
    &self.path
  }

  pub fn read(&self) -> Option<Vec<u8>> {
    if !self.current() {
      return None;
    }
    let file = File::open(&self.path).ok()?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
      return None;
    }
    let (digest, payload) = bytes.split_at_checked(32)?;
    if Sha256::digest(payload).as_slice() != digest {
      return None;
    }
    touch(&self.path);
    Some(payload.to_vec())
  }

  pub fn write(&self, bytes: &[u8]) -> crate::Result<()> {
    if !self.current()
      || (bytes.len() + 32) as u64 > LIMIT
      || !room(&self.root)?
    {
      return Ok(());
    }
    let mut record = Sha256::digest(bytes).to_vec();
    record.extend_from_slice(bytes);
    publish(&self.path, &record)?;
    trim(&self.root, BUDGET)
  }

  pub fn load<T: DeserializeOwned>(&self) -> Option<T> {
    serde_json::from_slice(&self.read()?).ok()
  }

  /// Stores `value`; a failed write only costs a later miss.
  pub fn store(&self, value: &impl Serialize) {
    if let Ok(bytes) = serde_json::to_vec(value) {
      let _ = self.write(&bytes);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use anyhow::Context;

  /// Records answer their source's content and code; copies share them;
  /// passwords, changes, damage, and trimming miss.
  #[test]
  fn records_round_trip_and_invalidate() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"first PDF bytes")?;
    let open = |source: &Path, password: &str, code: &str| {
      Record::open(
        root.path().into(),
        source,
        password,
        Kind::Outline,
        code,
        || true,
      )
    };
    let record = open(&source, "", "code").context("record")?;
    record.store(&("rows", 2));
    assert_eq!(record.load::<(String, usize)>(), Some(("rows".into(), 2)));
    let copy = root.path().join("copy.pdf");
    std::fs::copy(&source, &copy)?;
    let copied = open(&copy, "", "code").context("copy")?;
    assert_eq!(record.path(), copied.path());
    assert!(copied.read().is_some());
    assert!(open(&source, "", "other code").context("code")?.read().is_none());
    assert!(open(&source, "secret", "code").is_none());
    let bytes = std::fs::read(record.path())?;
    for len in 0..bytes.len() {
      std::fs::write(record.path(), &bytes[..len])?;
      assert!(record.read().is_none(), "{len}");
    }
    let mut damaged = bytes.clone();
    damaged[32] ^= 1;
    std::fs::write(record.path(), damaged)?;
    assert!(record.read().is_none());
    std::fs::write(record.path(), &bytes)?;
    std::fs::write(&source, b"other PDF bytes")?;
    assert!(record.read().is_none());
    let changed = open(&source, "", "code").context("changed")?;
    assert!(changed.read().is_none());
    record.store(&("stale", 1));
    assert!(changed.read().is_none());
    changed.store(&("rows", 2));
    assert!(changed.read().is_some());
    trim(root.path(), 0)?;
    assert!(!changed.path().exists());
    Ok(())
  }
}
