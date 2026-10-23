use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

fn paths() -> &'static Mutex<BTreeSet<PathBuf>> {
  static PATHS: OnceLock<Mutex<BTreeSet<PathBuf>>> = OnceLock::new();
  PATHS.get_or_init(Mutex::default)
}

pub(crate) struct Private(PathBuf);

impl Private {
  pub fn new(path: &Path) -> crate::Result<Self> {
    let path = path.canonicalize()?;
    paths()
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .insert(path.clone());
    Ok(Self(path))
  }
}

impl Drop for Private {
  fn drop(&mut self) {
    paths()
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .remove(&self.0);
  }
}

pub(crate) fn cacheable(path: &Path, password: &str) -> bool {
  password.is_empty()
    && path.canonicalize().is_ok_and(|path| {
      !paths()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&path)
    })
}
