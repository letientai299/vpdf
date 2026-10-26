//! Saved reading state, and saving it when the terminal goes away.

use crate::Result;
use crate::app::App;
use serde::{Deserialize, Serialize};
use signal_hook::consts::{SIGHUP, SIGTERM};
use signal_hook::iterator::Signals;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use vpdf_model::geometry::Crop;
use vpdf_model::text::Position;
use vpdf_ui::keys::PanelKind;
use vpdf_ui::prompt::History;

const RECENT: usize = 100;
const HISTORY: usize = 200;

mod cleanup;
mod history;
#[cfg(test)]
mod history_tests;
#[cfg(test)]
mod outline_tests;
pub(crate) use cleanup::Cleaner;
pub(crate) use history::{Revision, Visit};

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
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub outline: Option<vpdf_ui::reading::Preferences>,
  /// Unknown panels from newer versions read as unsaved.
  #[serde(
    default,
    deserialize_with = "lenient",
    skip_serializing_if = "Option::is_none"
  )]
  pub sidebar: Option<Sidebar>,
}

/// The sidebar a document was left with.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sidebar {
  Closed,
  Open(PanelKind),
}

impl Sidebar {
  /// Panels built from saved or document data reopen; session-only
  /// ones (Results, Jumps, Recent) and Version save as closed.
  pub fn of(kind: Option<PanelKind>) -> Self {
    match kind {
      Some(
        kind @ (PanelKind::Outline
        | PanelKind::Figures
        | PanelKind::Annotations
        | PanelKind::Marks
        | PanelKind::Info),
      ) => Self::Open(kind),
      _ => Self::Closed,
    }
  }
}

fn lenient<'de, D: serde::Deserializer<'de>>(
  deserializer: D,
) -> std::result::Result<Option<Sidebar>, D::Error> {
  let value = serde_json::Value::deserialize(deserializer)?;
  Ok(serde_json::from_value(value).ok())
}

impl Entry {
  fn merge_outline(&mut self, other: &Self) -> bool {
    if let Some(preferences) = &other.outline
      && self.outline.as_ref().is_none_or(|own| preferences.newer(own))
    {
      self.outline = Some(preferences.clone());
      return true;
    }
    false
  }
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
  #[serde(skip)]
  stale: Arc<cleanup::Stale>,
  #[serde(default)]
  history: history::Ledger,
  #[serde(skip)]
  /// Persisted history revision last observed.
  baseline: Revision,
  #[cfg(test)]
  #[serde(skip)]
  pub(crate) writes: usize,
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
    let mut state: Self = std::fs::read(path)
      .ok()
      .and_then(|bytes| serde_json::from_slice(&bytes).ok())
      .unwrap_or_default();
    state.baseline = state.history.clock;
    state
  }

  pub fn set_perf(&mut self, on: bool) {
    self.perf = on;
    self.perf_updated = now().max(self.perf_updated.saturating_add(1));
  }

  pub fn entry(&self, path: &Path) -> Option<&Entry> {
    self.recent.iter().find(|entry| entry.path == path)
  }

  /// Records `entry` as the latest visit; returns whether anything moved.
  pub fn remember(&mut self, mut entry: Entry) -> bool {
    if !self.history.allows_record(&entry.path) {
      return false;
    }
    let index = self.recent.iter().position(|old| old.path == entry.path);
    let previous = index.map(|index| self.recent.remove(index));
    if let Some(old) = &previous {
      entry.merge_outline(old);
    }
    let moved = previous.as_ref().is_none_or(|old| {
      if old.outline != entry.outline || old.sidebar != entry.sidebar {
        return true;
      }
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
    let directory = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)?;
    let lock = std::fs::File::options()
      .create(true)
      .truncate(false)
      .read(true)
      .write(true)
      .open(path.with_extension("lock"))?;
    lock.lock()?;
    let mut disk = Self::load(path);
    disk.stale.clone_from(&self.stale);
    disk.prune();
    let mut saved = self.clone();
    saved.prune();
    saved.merge_history(&disk);
    saved.history.compact();
    let mut marks = disk.marks;
    marks.append(&mut saved.marks);
    saved.marks = marks;
    saved.commands = merge(disk.commands, commands);
    saved.searches = merge(disk.searches, searches);
    if disk.perf_updated > self.perf_updated {
      saved.perf = disk.perf;
      saved.perf_updated = disk.perf_updated;
    }
    let mut staged =
      tempfile::Builder::new().suffix(".tmp").tempfile_in(directory)?;
    staged.write_all(&serde_json::to_vec(&saved)?)?;
    staged.persist(path).map_err(|error| error.error)?;
    saved.baseline = saved.history.clock;
    // Keep the live preference until explicitly toggled.
    saved.perf = self.perf;
    saved.perf_updated = self.perf_updated;
    #[cfg(test)]
    {
      saved.writes += 1;
    }
    *self = saved;
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

impl State {
  /// Records where `app` is; a covered pane records nothing. Returns
  /// whether anything moved.
  pub fn remember_pane(&mut self, app: &App) -> bool {
    let Some(entry) = app.saved_entry() else { return false };
    let opened = self.history.opened(&app.path);
    if opened == app.opened {
      return self.remember(entry);
    }
    if !opened.same_document(app.opened)
      || !self.history.allows_record(&app.path)
    {
      return false;
    }
    self
      .recent
      .iter_mut()
      .find(|old| old.path == entry.path)
      .is_some_and(|old| old.merge_outline(&entry))
  }
}

impl App {
  /// Where this pane is, to save; a covered document counts as closed,
  /// since its pane may be a stale copy.
  pub fn saved_entry(&self) -> Option<Entry> {
    if self.covered() || self.manual.is_some() {
      return None;
    }
    Some(self.reading_entry())
  }

  pub(crate) fn reading_entry(&self) -> Entry {
    Entry {
      path: self.path.clone(),
      cursor: self.navigation_position(),
      zoom: self.layout.zoom(),
      rotation: self.layout.rotation(),
      used: now(),
      marks: self.marks.clone(),
      crop: self.layout.crop(),
      outline: self.outline.preferences().cloned(),
      sidebar: Some(Sidebar::of(
        self.panel.as_ref().map(|panel| panel.view.kind()),
      )),
    }
  }

  /// Returns to a saved position and view.
  pub fn restore(&mut self, entry: &Entry) -> Result<()> {
    self.outline.restore_preferences(entry.outline.clone());
    self.crop_scan.clear();
    self.marks.clone_from(&entry.marks);
    self.layout.restore(entry.zoom, entry.rotation, entry.crop);
    self.layout.reflow(self.doc.pages.iter().map(|page| page.bounds));
    self.reading_view().restore_position(entry.cursor)?;
    if !entry.crop.is_none() && self.layout.crops().is_empty() {
      self.start_crop();
    }
    Ok(())
  }
}

// Saves reading positions when the terminal goes away.
//
// A closed terminal sends SIGHUP while a child such as fzf may hold
// the tty, so the event loop cannot always save. A thread waits for
// SIGHUP or SIGTERM, gives the loop a second to exit and save on its
// own, then writes the last recorded state, restores the terminal, and
// exits.

/// State as last recorded by the event loop, for the hangup thread.
pub type Latest = Arc<Mutex<Option<(State, History)>>>;

/// Sets `flag` on SIGHUP or SIGTERM, then saves `latest` to `store` and
/// exits unless the process ended first.
pub fn watch_hangup(
  store: Option<PathBuf>,
  flag: Arc<AtomicBool>,
  latest: Latest,
) -> Result<()> {
  let mut signals = Signals::new([SIGHUP, SIGTERM])?;
  thread::spawn(move || {
    let Some(signal) = signals.forever().next() else { return };
    flag.store(true, Ordering::Relaxed);
    vpdf_terminal::input::wake();
    thread::sleep(Duration::from_secs(1));
    if let Some(store) = store
      && let Ok(mut latest) = latest.lock()
      && let Some((state, history)) = latest.as_mut()
    {
      drop(state.save(&store, history.commands(), history.searches()));
    }
    // The loop may block in a prompt that never sees the flag.
    vpdf_terminal::terminal::restore();
    std::process::exit(128 + signal);
  });
  Ok(())
}

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
      outline: None,
      sidebar: None,
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
  fn saving_cleanup() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir()?;
    let path = root.path().join("state.json");
    let mut state = State::default();
    state.save(&path, &["first".into()], &[])?;
    let old = std::fs::read(&path)?;
    let mut old_file = std::fs::File::open(&path)?;
    state.save(&path, &["second".into()], &[])?;
    let mut retained = Vec::new();
    std::io::Read::read_to_end(&mut old_file, &mut retained)?;
    assert_eq!(retained, old);
    assert_eq!(State::load(&path).commands, ["first", "second"]);
    assert_eq!(path.metadata()?.permissions().mode() & 0o777, 0o600);
    std::fs::remove_file(&path)?;
    std::fs::create_dir(&path)?;
    assert!(state.save(&path, &[], &[]).is_err());
    assert!(path.is_dir());
    assert_eq!(std::fs::read_dir(root.path())?.count(), 2);
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
