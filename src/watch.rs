use crate::Result;
use crate::app::App;
use std::path::Path;
use std::time::SystemTime;

pub type Stamp = (SystemTime, u64);

pub fn stamp(path: &Path) -> Option<Stamp> {
  let meta = std::fs::metadata(path).ok()?;
  Some((meta.modified().ok()?, meta.len()))
}

impl App {
  /// Reloads after the file changes and holds still for one poll, so
  /// a build that is still writing the PDF is not read half-done.
  pub fn watch(&mut self) -> Result<bool> {
    let now = stamp(&self.path);
    if now.is_none() || now == self.stamp {
      self.changed = None;
      return Ok(false);
    }
    if self.changed != now {
      self.changed = now;
      return Ok(false);
    }
    self.changed = None;
    self.stamp = now;
    self.reload_with(&self.password.clone())?;
    self.reader.message = "Reloaded after change".into();
    Ok(true)
  }
}
