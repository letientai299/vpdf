use crate::Result;
use crate::app::App;
use notify::{RecursiveMode, Watcher as _};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

pub type Stamp = (SystemTime, u64);

/// Quiet time before reading a changed file, so a build that is still
/// writing the PDF is not read half-done.
const SETTLE: Duration = Duration::from_millis(100);

pub fn stamp(path: &Path) -> Option<Stamp> {
  let meta = std::fs::metadata(path).ok()?;
  Some((meta.modified().ok()?, meta.len()))
}

/// OS file events (`FSEvents`, inotify, kqueue) for open documents.
///
/// Parent directories are watched because builds often replace the PDF
/// by rename, which ends a watch on the old inode. Network mounts may
/// emit no events, so the session still polls slowly as a fallback.
pub struct Watcher {
  inner: Option<notify::RecommendedWatcher>,
  events: mpsc::Receiver<PathBuf>,
  directories: HashMap<PathBuf, usize>,
}

impl Default for Watcher {
  fn default() -> Self {
    let (send, events) = mpsc::channel();
    let inner = notify::recommended_watcher(
      move |event: notify::Result<notify::Event>| {
        for path in event.into_iter().flat_map(|event| event.paths) {
          let _ = send.send(path);
        }
      },
    )
    .ok();
    Self { inner, events, directories: HashMap::new() }
  }
}

impl Watcher {
  pub fn add(&mut self, file: &Path) {
    let Some(directory) = file.parent() else { return };
    let count = self.directories.entry(directory.to_owned()).or_default();
    *count += 1;
    if *count == 1
      && let Some(inner) = &mut self.inner
    {
      let _ = inner.watch(directory, RecursiveMode::NonRecursive);
    }
  }

  pub fn remove(&mut self, file: &Path) {
    let Some(directory) = file.parent() else { return };
    let Some(count) = self.directories.get_mut(directory) else { return };
    *count -= 1;
    if *count == 0 {
      self.directories.remove(directory);
      if let Some(inner) = &mut self.inner {
        let _ = inner.unwatch(directory);
      }
    }
  }

  /// Drains events; the files named since the last call.
  pub fn touched(&self) -> HashSet<PathBuf> {
    self.events.try_iter().collect()
  }
}

impl App {
  /// Reloads once the file differs from the loaded one and has held
  /// still for `SETTLE`; returns whether it reloaded.
  pub fn watch(&mut self) -> Result<bool> {
    let Some(now) = stamp(&self.path).filter(|now| Some(*now) != self.stamp)
    else {
      self.settling = None;
      return Ok(false);
    };
    match self.settling {
      Some((seen, since)) if seen == now && since.elapsed() >= SETTLE => {}
      Some((seen, _)) if seen == now => return Ok(false),
      _ => {
        self.settling = Some((now, Instant::now()));
        return Ok(false);
      }
    }
    self.settling = None;
    self.stamp = Some(now);
    self.reload_with(&self.password.clone())?;
    self.reader.message = "Reloaded after change".into();
    Ok(true)
  }
}
