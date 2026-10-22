use super::store::{BUDGET, publish, room, touch, trim};
use super::{Alias, Version, alias_with, hex};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

const LIMIT: u64 = 16 << 20;

pub(super) struct Record {
  source: PathBuf,
  version: Version,
  root: PathBuf,
  key: Alias,
  kind: &'static str,
}

impl Record {
  pub(super) fn open(
    root: PathBuf,
    source: &Path,
    password: &str,
    kind: &'static str,
    code: &str,
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
      format!("{kind} 1").as_bytes(),
      content.as_slice(),
      password.as_bytes(),
      env!("VPDF_INDEX_CODE").as_bytes(),
      env!("VPDF_INDEX_CRATES").as_bytes(),
      code.as_bytes(),
    ] {
      digest.update((part.len() as u64).to_le_bytes());
      digest.update(part);
    }
    Some(Self {
      source: source.to_owned(),
      version,
      root,
      key: digest.finalize().into(),
      kind,
    })
  }

  pub(super) fn path(&self) -> PathBuf {
    self.root.join(format!("{}.{}", hex(&self.key), self.kind))
  }

  pub(super) fn read(&self) -> Option<Vec<u8>> {
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
    touch(&path);
    Some(payload.to_vec())
  }

  pub(super) fn write(&self, bytes: &[u8]) -> crate::Result<()> {
    if Version::read(&self.source) != Some(self.version)
      || (bytes.len() + 32) as u64 > LIMIT
      || !room(&self.root)?
    {
      return Ok(());
    }
    let mut record = Sha256::digest(bytes).to_vec();
    record.extend_from_slice(bytes);
    publish(&self.path(), &record)?;
    trim(&self.root, BUDGET)
  }
}
