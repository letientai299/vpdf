use super::record::Record;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
pub enum Kind {
  Annotations,
  Links,
  Recent,
}

pub struct Snapshot(Record);

impl Snapshot {
  pub fn open(
    root: PathBuf,
    source: &Path,
    password: &str,
    kind: Kind,
    code: &str,
    ready: impl FnMut() -> bool,
  ) -> Option<Self> {
    let kind = match kind {
      Kind::Annotations => "annotations",
      Kind::Links => "links",
      Kind::Recent => "recent",
    };
    Record::open(root, source, password, kind, code, ready).map(Self)
  }

  pub fn read(&self) -> Option<Vec<u8>> {
    self.0.read()
  }

  pub fn write(&self, bytes: &[u8]) -> crate::Result<()> {
    self.0.write(bytes)
  }
}
