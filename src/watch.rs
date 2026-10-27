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
pub const SETTLE: Duration = Duration::from_millis(100);

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
          vpdf_terminal::input::wake();
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
    self.watch_at(Instant::now())
  }

  pub(crate) fn watch_at(&mut self, at: Instant) -> Result<bool> {
    // Generated help has no backing file.
    if self.manual.is_some() {
      self.settling = None;
      return Ok(false);
    }
    let Some(now) = stamp(&self.path).filter(|now| Some(*now) != self.stamp)
    else {
      self.settling = None;
      return Ok(false);
    };
    match self.settling {
      Some((seen, since))
        if seen == now && at.duration_since(since) >= SETTLE => {}
      Some((seen, _)) if seen == now => return Ok(false),
      _ => {
        self.settling = Some((now, at));
        return Ok(false);
      }
    }
    self.settling = None;
    self.reload_with(&self.password.clone())?;
    self.reader.notice("Reloaded after change");
    Ok(true)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn failed_reload_retries() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("live.pdf");
    crate::fixtures::pdf(&path, &["Original"], "")?;
    let screen = crate::screen::cells(80, 24);
    let mut app = App::with_screen(&path, "", false, screen)?;
    let loaded = app.stamp;
    std::fs::write(&path, "incomplete PDF")?;
    for _ in 0..2 {
      assert!(!app.watch()?);
      assert!(app.settling.is_some(), "failed reload must retry");
      app.settling = Some((
        stamp(&path).unwrap(),
        Instant::now().checked_sub(SETTLE).unwrap(),
      ));
      assert!(app.watch().is_err());
      assert_eq!(app.stamp, loaded);
    }
    crate::fixtures::pdf(&path, &["Recovered", "Second"], "")?;
    assert!(!app.watch()?);
    app.settling = Some((
      stamp(&path).unwrap(),
      Instant::now().checked_sub(SETTLE).unwrap(),
    ));
    assert!(app.watch()?);
    assert_eq!(app.doc.pages.len(), 2);
    assert!(!app.watch()?);
    Ok(())
  }
}
