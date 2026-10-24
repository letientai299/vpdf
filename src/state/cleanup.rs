use super::{Entry, Mark, State};
use std::collections::{BTreeMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use vpdf_pdf::jobs::{OneShot, Wake};

#[derive(Clone, Debug, Default)]
pub(crate) struct Stale {
  recent: BTreeMap<PathBuf, Entry>,
  marks: BTreeMap<char, Mark>,
}

impl Stale {
  fn scan(state: &State, stop: &AtomicBool) -> Option<Self> {
    let paths: HashSet<_> = state
      .recent
      .iter()
      .map(|entry| &entry.path)
      .chain(state.marks.values().map(|mark| &mark.path))
      .collect();
    let mut removed = HashSet::new();
    for path in paths {
      if stop.load(Ordering::Relaxed) {
        return None;
      }
      if missing(path) {
        removed.insert(path);
      }
    }
    Some(Self {
      recent: state
        .recent
        .iter()
        .filter(|e| removed.contains(&e.path))
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect(),
      marks: state
        .marks
        .iter()
        .filter(|(_, m)| removed.contains(&m.path))
        .map(|(name, mark)| (*name, mark.clone()))
        .collect(),
    })
  }
}

fn missing(path: &Path) -> bool {
  std::fs::metadata(path)
    .is_err_and(|error| error.kind() == ErrorKind::NotFound)
}

#[derive(Default)]
pub(crate) enum Cleaner {
  #[default]
  Ready,
  Running(OneShot<Stale>),
  Finished,
}

impl Cleaner {
  pub fn poll(&mut self, state: &mut State, wake: &Wake) -> bool {
    if matches!(self, Self::Ready) {
      if state.recent.is_empty() && state.marks.is_empty() {
        *self = Self::Finished;
        return false;
      }
      let snapshot = state.clone();
      *self = Self::Running(OneShot::spawn(wake, move |stop| {
        Stale::scan(&snapshot, stop)
      }));
    }
    let Self::Running(job) = self else { return false };
    match job.result.try_recv() {
      Ok(missing) => {
        *self = Self::Finished;
        state.stale = Arc::new(missing);
        state.prune()
      }
      Err(mpsc::TryRecvError::Empty) => false,
      Err(mpsc::TryRecvError::Disconnected) => {
        *self = Self::Finished;
        false
      }
    }
  }
}

impl State {
  pub(super) fn prune(&mut self) -> bool {
    let before = (self.recent.len(), self.marks.len());
    self
      .recent
      .retain(|entry| self.stale.recent.get(&entry.path) != Some(entry));
    self.marks.retain(|name, mark| self.stale.marks.get(name) != Some(mark));
    before != (self.recent.len(), self.marks.len())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Result;
  use crate::state::tests::entry;
  use std::time::Duration;
  use vpdf_model::text::Position;

  fn scanned(state: &State) -> Stale {
    Stale::scan(state, &AtomicBool::new(false)).unwrap()
  }

  #[test]
  fn cancelled_scans_return_nothing() {
    let mut state = State::default();
    state.recent.push(entry("missing.pdf", 0, 1));
    let stop = AtomicBool::new(true);
    assert!(Stale::scan(&state, &stop).is_none());
    assert_eq!(state.recent.len(), 1);
  }

  #[test]
  fn metadata_errors_are_preserved() -> Result<()> {
    let root = tempfile::tempdir()?;
    let link = root.path().join("loop.pdf");
    std::os::unix::fs::symlink(&link, &link)?;
    assert!(std::fs::metadata(&link).is_err());
    assert!(!missing(&link));
    Ok(())
  }

  #[test]
  fn cleanup_removes_missing_paths() -> Result<()> {
    let root = tempfile::tempdir()?;
    let existing = root.path().join("existing.pdf");
    let missing = root.path().join("missing.pdf");
    let removed = root.path().join("removed/book.pdf");
    std::fs::write(&existing, b"preserved")?;
    let mut state = State::default();
    for path in [&existing, &missing, &removed] {
      state.recent.push(entry(path.to_str().unwrap(), 0, 1));
    }
    state
      .marks
      .insert('A', Mark { path: missing.clone(), cursor: Position::default() });
    state
      .marks
      .insert('B', Mark { path: removed.clone(), cursor: Position::default() });
    let (wake, woken) = crate::fixtures::wake();
    let mut cleaner = Cleaner::default();
    cleaner.poll(&mut state, &wake);
    woken.recv_timeout(Duration::from_secs(5))?;
    cleaner.poll(&mut state, &wake);
    assert!(state.entry(&existing).is_some());
    assert!(state.entry(&missing).is_none());
    assert!(state.entry(&removed).is_none());
    assert!(!state.marks.contains_key(&'A'));
    assert!(!state.marks.contains_key(&'B'));
    assert_eq!(std::fs::read(existing)?, b"preserved");
    Ok(())
  }

  #[test]
  fn save_does_not_restore_stale_entries() -> Result<()> {
    let root = tempfile::tempdir()?;
    let store = root.path().join("state.json");
    let path = root.path().join("missing.pdf");
    let old = entry(path.to_str().unwrap(), 0, 1);
    let mut state = State::default();
    state.recent.push(old.clone());
    state.marks.insert('A', Mark { path, cursor: old.cursor });
    state.save(&store, &["help".into()], &["query".into()])?;
    state.stale = Arc::new(scanned(&state));
    assert!(state.prune());
    state.save(&store, &[], &[])?;
    let saved = State::load(&store);
    assert_eq!(saved.recent, []);
    assert!(saved.marks.is_empty());
    assert_eq!(saved.commands, ["help"]);
    assert_eq!(saved.searches, ["query"]);
    state.save(&store, &[], &[])?;
    assert_eq!(State::load(&store).recent, []);
    Ok(())
  }

  #[test]
  fn cleanup_preserves_newer_entries() -> Result<()> {
    let root = tempfile::tempdir()?;
    let store = root.path().join("state.json");
    let path = root.path().join("missing.pdf");
    let mut state = State::default();
    state.recent.push(entry(path.to_str().unwrap(), 0, 1));
    state.stale = Arc::new(scanned(&state));
    state.recent[0].used = 2;
    assert!(!state.prune());
    let mut concurrent = State::default();
    concurrent.recent.push(entry(path.to_str().unwrap(), 1, 3));
    concurrent.save(&store, &[], &[])?;
    state.save(&store, &[], &[])?;
    assert_eq!(State::load(&store).recent[0].used, 3);
    Ok(())
  }
}
