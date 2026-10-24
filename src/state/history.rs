use super::{Entry, RECENT, State};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(
  Clone,
  Copy,
  Debug,
  Default,
  Deserialize,
  Eq,
  Ord,
  PartialEq,
  PartialOrd,
  Serialize,
)]
pub(crate) struct Revision(u64);

#[derive(
  Clone,
  Copy,
  Debug,
  Default,
  Deserialize,
  Eq,
  Ord,
  PartialEq,
  PartialOrd,
  Serialize,
)]
pub(crate) struct Visit {
  /// Last removal observed before opening.
  after: Revision,
  revision: Revision,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub(super) struct Ledger {
  pub clock: Revision,
  /// Old snapshots must refresh below this.
  pub floor: Revision,
  opens: BTreeMap<PathBuf, Visit>,
  forgotten: BTreeMap<PathBuf, Revision>,
}

impl Ledger {
  pub fn opened(&self, path: &Path) -> Visit {
    self.opens.get(path).copied().unwrap_or_default()
  }

  pub fn agrees(&self, other: &Self, path: &Path) -> bool {
    self.floor == other.floor
      && self.opened(path) == other.opened(path)
      && self.forgotten.get(path) == other.forgotten.get(path)
  }

  pub fn excludes(&self, path: &Path) -> bool {
    self
      .forgotten
      .get(path)
      .is_some_and(|removed| *removed > self.opened(path).after)
  }

  pub fn allows_record(&self, path: &Path) -> bool {
    !self.excludes(path)
      && (self.floor == Revision::default() || self.opens.contains_key(path))
  }

  fn next(&mut self) -> Revision {
    self.clock = Revision(
      self.clock.0.checked_add(1).expect("History revision exhausted"),
    );
    self.clock
  }

  pub fn compact(&mut self) {
    if self.forgotten.len() <= RECENT {
      return;
    }
    let mut revisions: Vec<_> = self.forgotten.values().copied().collect();
    revisions.sort_unstable();
    self.floor = self.floor.max(revisions[revisions.len() - RECENT - 1]);
    self.forgotten.retain(|_, revision| *revision > self.floor);
  }
}

impl State {
  pub(crate) fn refresh_history(&mut self, store: Option<&Path>) {
    if let Some(store) = store {
      self.merge_history(&Self::load(store));
    }
  }

  pub(crate) fn open_revision(&self, path: &Path) -> Visit {
    self.history.opened(path)
  }

  pub(crate) fn opened(&mut self, entry: Entry) -> Visit {
    let revision = self.history.next();
    let visit = Visit {
      after: self
        .history
        .forgotten
        .get(&entry.path)
        .copied()
        .unwrap_or_default()
        .max(self.history.floor),
      revision,
    };
    self.history.opens.insert(entry.path.clone(), visit);
    self.remember(entry);
    visit
  }

  pub fn forget(&mut self, paths: &[PathBuf]) -> bool {
    if paths.is_empty() {
      return false;
    }
    let revision = self.history.next();
    for path in paths {
      self.history.forgotten.insert(path.clone(), revision);
    }
    self.recent.retain(|entry| !self.history.excludes(&entry.path));
    true
  }

  pub(super) fn merge_history(&mut self, disk: &Self) {
    let mut recent = std::mem::take(&mut self.recent);
    // Compacted markers invalidate older history baselines.
    recent.retain(|entry| {
      self.baseline >= disk.history.floor
        || disk.entry(&entry.path).is_some()
          && disk.history.opened(&entry.path)
            == self.history.opened(&entry.path)
    });
    let mut opens = self.history.opens.clone();
    for entry in &disk.recent {
      let revision = disk.history.opened(&entry.path);
      match recent.iter_mut().find(|own| own.path == entry.path) {
        Some(own)
          if (self.history.opened(&own.path), own.used)
            >= (revision, entry.used) => {}
        Some(own) => {
          *own = entry.clone();
          opens.insert(entry.path.clone(), revision);
        }
        None => {
          recent.push(entry.clone());
          opens.insert(entry.path.clone(), revision);
        }
      }
    }
    self.history.opens = opens;
    self.history.floor = self.history.floor.max(disk.history.floor);
    self.history.forgotten.retain(|_, rev| *rev > self.history.floor);
    for (path, revision) in &disk.history.forgotten {
      let own = self.history.forgotten.entry(path.clone()).or_default();
      *own = (*own).max(*revision);
    }
    recent.retain(|entry| !self.history.excludes(&entry.path));
    recent.sort_by_key(|entry| std::cmp::Reverse(entry.used));
    recent.truncate(RECENT);
    self
      .history
      .opens
      .retain(|path, _| recent.iter().any(|entry| &entry.path == path));
    self.recent = recent;
    self.history.clock = self.history.clock.max(disk.history.clock);
    self.baseline = self.baseline.max(disk.history.clock);
  }
}
