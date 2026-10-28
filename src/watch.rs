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
    Self::waking(vpdf_terminal::input::wake)
  }
}

impl Watcher {
  /// Calls `wake` after each event, from the notify thread.
  pub fn waking(wake: impl Fn() + Send + 'static) -> Self {
    let (send, events) = mpsc::channel();
    let inner = notify::recommended_watcher(
      move |event: notify::Result<notify::Event>| {
        for path in event.into_iter().flat_map(|event| event.paths) {
          let _ = send.send(path);
          wake();
        }
      },
    )
    .ok();
    Self { inner, events, directories: HashMap::new() }
  }

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

/// Watches the config file that existed at startup.
///
/// The parent directory watch survives atomic saves; a missing file
/// keeps the live settings until a version reappears.
pub struct ConfigWatch {
  path: PathBuf,
  watcher: Option<Watcher>,
  seen: Option<Stamp>,
  contents: Option<Vec<u8>>,
  settling: Option<(Stamp, Instant)>,
}

impl ConfigWatch {
  pub fn new(path: &Path) -> Option<Self> {
    path.is_file().then(|| Self {
      path: path.to_owned(),
      watcher: None,
      seen: stamp(path),
      contents: std::fs::read(path).ok(),
      settling: None,
    })
  }

  /// Starts OS events; until then only fallback polls run.
  pub fn start(&mut self, wake: impl Fn() + Send + 'static) {
    if self.watcher.is_none() {
      let mut watcher = Watcher::waking(wake);
      watcher.add(&self.path);
      self.watcher = Some(watcher);
    }
  }

  /// Whether a changed version held still for `SETTLE`.
  pub fn due(&mut self, force: bool, now: Instant) -> bool {
    let touched = self
      .watcher
      .as_ref()
      .is_some_and(|watcher| !watcher.touched().is_empty());
    if !(force || touched || self.settling.is_some()) {
      return false;
    }
    let Some(current) = stamp(&self.path).filter(|now| Some(*now) != self.seen)
    else {
      self.settling = None;
      return false;
    };
    match self.settling {
      Some((seen, since))
        if seen == current && now.duration_since(since) >= SETTLE => {}
      Some((seen, _)) if seen == current => return false,
      _ => {
        self.settling = Some((current, now));
        return false;
      }
    }
    self.settling = None;
    self.seen = Some(current);
    self.changed()
  }

  /// Whether contents differ from the last applied version; drains events.
  pub fn reconcile(&mut self) -> bool {
    if let Some(watcher) = &self.watcher {
      watcher.touched();
    }
    self.settling = None;
    self.seen = stamp(&self.path);
    self.seen.is_some() && self.changed()
  }

  fn changed(&mut self) -> bool {
    let contents = std::fs::read(&self.path).ok();
    if contents.is_some() && contents == self.contents {
      return false;
    }
    self.contents = contents;
    true
  }

  /// Records the version a manual reload applied.
  pub fn applied(&mut self) {
    self.seen = stamp(&self.path);
    self.contents = std::fs::read(&self.path).ok();
    self.settling = None;
  }

  pub fn settling(&self) -> bool {
    self.settling.is_some()
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

  /// Polls as the fallback timer would, past the settle time.
  fn poll(watch: &mut ConfigWatch) -> bool {
    let start = Instant::now();
    watch.due(true, start) || watch.due(false, start + SETTLE)
  }

  #[test]
  fn config_watch_follows_saves() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.toml");
    assert!(ConfigWatch::new(&path).is_none());
    std::fs::write(&path, "a = 1\n")?;
    let mut watch = ConfigWatch::new(&path).unwrap();
    assert!(!poll(&mut watch));
    std::fs::write(directory.path().join("other.toml"), "x")?;
    assert!(!poll(&mut watch));
    std::fs::write(&path, "a = 2\n")?;
    let start = Instant::now();
    assert!(!watch.due(true, start));
    std::fs::write(&path, "a = 33\n")?;
    assert!(!watch.due(false, start + SETTLE));
    assert!(watch.due(false, start + SETTLE * 3));
    let temporary = directory.path().join(".config.toml.swp");
    std::fs::write(&temporary, "a = 4\n")?;
    std::fs::rename(&temporary, &path)?;
    assert!(poll(&mut watch));
    std::fs::remove_file(&path)?;
    assert!(!poll(&mut watch));
    std::fs::write(&path, "a = 4\n")?;
    assert!(!poll(&mut watch), "same contents reloaded");
    std::fs::write(&path, "a = 5\n")?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))?;
    if std::fs::read(&path).is_err() {
      assert!(poll(&mut watch));
      assert!(!poll(&mut watch), "unreadable file warned twice");
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    std::fs::write(&path, "a = 6\n")?;
    assert!(poll(&mut watch));
    std::fs::write(&path, "a = 7\n")?;
    watch.applied();
    assert!(!poll(&mut watch), "manual reload repeated");
    Ok(())
  }

  #[test]
  fn config_watch_wakes_on_events() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("config.toml");
    std::fs::write(&path, "")?;
    let mut watch = ConfigWatch::new(&path).unwrap();
    let (send, woken) = mpsc::channel();
    watch.start(move || {
      let _ = send.send(());
    });
    // FSEvents may report changes made just before the stream starts.
    std::thread::sleep(Duration::from_millis(200));
    while woken.try_recv().is_ok() {}
    std::fs::write(&path, "a = 1\n")?;
    woken.recv_timeout(Duration::from_secs(10))?;
    let start = Instant::now();
    assert!(!watch.due(false, start));
    assert!(watch.settling());
    assert!(watch.due(false, start + SETTLE));
    Ok(())
  }
}
