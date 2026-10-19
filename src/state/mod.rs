use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use vpdf_model::geometry::Crop;
use vpdf_model::text::Position;

const RECENT: usize = 100;
const HISTORY: usize = 200;

/// Where a document was left, for reopening.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entry {
  pub path: PathBuf,
  pub cursor: Position,
  pub zoom: f32,
  pub rotation: u16,
  /// Unix milliseconds of the last visit.
  pub used: u64,
  #[serde(default)]
  pub marks: BTreeMap<char, Position>,
  #[serde(default)]
  pub crop: Crop,
}

/// An uppercase mark: a position in any document.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Mark {
  pub path: PathBuf,
  pub cursor: Position,
}

/// Recent documents and prompt history kept across runs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct State {
  #[serde(default)]
  pub perf: bool,
  #[serde(default)]
  perf_updated: u64,
  #[serde(default)]
  pub recent: Vec<Entry>,
  #[serde(default)]
  pub commands: Vec<String>,
  #[serde(default)]
  pub searches: Vec<String>,
  #[serde(default)]
  pub marks: BTreeMap<char, Mark>,
}

/// `$XDG_STATE_HOME/vpdf/state.json`, else under `~/.local/state`.
pub fn location() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_STATE_HOME")
    .filter(|dir| !dir.is_empty())
    .map(PathBuf::from)
    .or_else(|| {
      std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state"))
    })?;
  Some(base.join("vpdf/state.json"))
}

pub fn now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl State {
  /// Missing or unreadable state starts empty.
  pub fn load(path: &Path) -> Self {
    std::fs::read(path)
      .ok()
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default()
  }

  pub fn set_perf(&mut self, on: bool) {
    self.perf = on;
    self.perf_updated = now().max(self.perf_updated.saturating_add(1));
  }

  pub fn entry(&self, path: &Path) -> Option<&Entry> {
    self.recent.iter().find(|entry| entry.path == path)
  }

  /// Records `entry` as the latest visit; returns whether anything moved.
  pub fn remember(&mut self, entry: Entry) -> bool {
    let index = self.recent.iter().position(|old| old.path == entry.path);
    let previous = index.map(|index| self.recent.remove(index));
    let moved = previous.as_ref().is_none_or(|old| {
      (old.cursor, old.zoom, old.rotation, &old.marks, old.crop)
        != (entry.cursor, entry.zoom, entry.rotation, &entry.marks, entry.crop)
    });
    self.recent.insert(0, entry);
    self.recent.truncate(RECENT);
    moved
  }

  /// Merges with the file on disk, so concurrent readers keep entries;
  /// `commands` and `searches` are this session's history.
  pub fn save(
    &mut self,
    path: &Path,
    commands: &[String],
    searches: &[String],
  ) -> Result<()> {
    let disk = Self::load(path);
    let mut recent = std::mem::take(&mut self.recent);
    for entry in disk.recent {
      match recent.iter_mut().find(|own| own.path == entry.path) {
        Some(own) if own.used >= entry.used => {}
        Some(own) => *own = entry,
        None => recent.push(entry),
      }
    }
    recent.sort_by_key(|entry| std::cmp::Reverse(entry.used));
    recent.truncate(RECENT);
    self.recent = recent;
    let mut marks = disk.marks;
    marks.append(&mut self.marks);
    self.marks = marks;
    self.commands = merge(disk.commands, commands);
    self.searches = merge(disk.searches, searches);
    let directory = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)?;
    let staged = path.with_extension(format!("{}.tmp", std::process::id()));
    // Saving an older reader must not undo a newer preference. Keep its
    // live layout unchanged; only the persisted snapshot adopts the toggle.
    let mut saved = self.clone();
    if disk.perf_updated > self.perf_updated {
      saved.perf = disk.perf;
      saved.perf_updated = disk.perf_updated;
    }
    std::fs::write(&staged, serde_json::to_vec(&saved)?)?;
    std::fs::rename(&staged, path)?;
    Ok(())
  }
}

/// Disk-only entries first, then this session's in order.
fn merge(disk: Vec<String>, own: &[String]) -> Vec<String> {
  let mut merged: Vec<_> =
    disk.into_iter().filter(|entry| !own.contains(entry)).collect();
  merged.extend_from_slice(own);
  let excess = merged.len().saturating_sub(HISTORY);
  merged.drain(..excess);
  merged
}

mod pane;

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use vpdf_model::text::PageId;

  pub(crate) fn entry(path: &str, page: usize, used: u64) -> Entry {
    Entry {
      path: path.into(),
      cursor: Position { page: PageId(page), ..Position::default() },
      zoom: 1.0,
      rotation: 0,
      used,
      marks: BTreeMap::new(),
      crop: Crop::default(),
    }
  }

  #[test]
  fn perf_defaults_off_for_old_state() -> Result<()> {
    let state: State = serde_json::from_str("{}")?;
    assert!(!state.perf);
    let root = tempfile::tempdir()?;
    let file = root.path().join("state.json");
    let mut state = State::default();
    state.set_perf(true);
    state.save(&file, &[], &[])?;
    assert!(State::load(&file).perf);
    state.set_perf(false);
    state.save(&file, &[], &[])?;
    assert!(!State::load(&file).perf);
    Ok(())
  }

  #[test]
  fn stale_reader_preserves_newer_perf_preference() -> Result<()> {
    let root = tempfile::tempdir()?;
    let file = root.path().join("state.json");
    let mut old = State::default();
    let mut newer = State::default();
    newer.set_perf(true);
    newer.save(&file, &[], &[])?;
    old.save(&file, &[], &[])?;
    assert!(State::load(&file).perf);
    assert!(!old.perf);
    let mut stale = State::load(&file);
    newer.set_perf(false);
    newer.save(&file, &[], &[])?;
    stale.save(&file, &[], &[])?;
    assert!(!State::load(&file).perf);
    assert!(stale.perf);
    Ok(())
  }

  #[test]
  fn save_merges_concurrent_readers() -> Result<()> {
    let root = tempfile::tempdir()?;
    let file = root.path().join("vpdf/state.json");
    let mut other = State {
      recent: vec![entry("/b.pdf", 2, 20), entry("/a.pdf", 9, 5)],
      ..State::default()
    };
    let commands = ["page 3".into(), "ocr".into()];
    other.save(&file, &commands, &[])?;
    let mut own = State {
      recent: vec![entry("/a.pdf", 4, 30), entry("/c.pdf", 1, 10)],
      ..State::default()
    };
    let commands = ["ocr".into(), "help".into()];
    own.save(&file, &commands, &["attention".into()])?;
    let saved = State::load(&file);
    let pages: Vec<_> = saved
      .recent
      .iter()
      .map(|e| (e.path.to_string_lossy().into_owned(), e.cursor.page.0))
      .collect();
    assert_eq!(
      pages,
      [("/a.pdf".into(), 4), ("/b.pdf".into(), 2), ("/c.pdf".into(), 1)]
    );
    assert_eq!(saved.commands, ["page 3", "ocr", "help"]);
    assert_eq!(saved.searches, ["attention"]);
    Ok(())
  }

  #[test]
  fn corrupt_state_starts_empty() -> Result<()> {
    let root = tempfile::tempdir()?;
    let file = root.path().join("state.json");
    std::fs::write(&file, "{not json")?;
    assert_eq!(State::load(&file).recent, []);
    Ok(())
  }
}
