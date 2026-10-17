use crate::Result;
use crate::app::App;
use crate::layout::Crop;
use crate::prompt::History;
use crate::text::{PageId, Position};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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

fn now() -> u64 {
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

  pub fn entry(&self, path: &Path) -> Option<&Entry> {
    self.recent.iter().find(|entry| entry.path == path)
  }

  /// Records where `app` is; returns whether anything moved.
  pub fn remember(&mut self, app: &App) -> bool {
    let entry = Entry {
      path: app.path.clone(),
      cursor: app.reader.cursor,
      zoom: app.layout.zoom,
      rotation: app.layout.rotation,
      used: now(),
      marks: app.marks.clone(),
      crop: app.layout.crop,
    };
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

  pub fn history(&self) -> History {
    History {
      commands: self.commands.clone(),
      searches: self.searches.clone(),
      revision: 0,
    }
  }

  /// Merges with the file on disk, so concurrent readers keep entries.
  pub fn save(&mut self, path: &Path, history: &History) -> Result<()> {
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
    self.commands = merge(disk.commands, &history.commands);
    self.searches = merge(disk.searches, &history.searches);
    let directory = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)?;
    let staged = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&staged, serde_json::to_vec(self)?)?;
    std::fs::rename(&staged, path)?;
    Ok(())
  }
}

impl App {
  /// Copies another pane's view of the same document, as a new split.
  pub fn mirror(&mut self, other: &Self) -> Result<()> {
    self.layout.zoom = other.layout.zoom;
    self.layout.rotation = other.layout.rotation;
    self.layout.crop = other.layout.crop;
    // A text preview (`gx`) is the other pane's, not the document's mode.
    self.text_mode = other.preview.unwrap_or(other.text_mode);
    self.layout.reflow(&self.doc.pages);
    self.reader.cursor =
      self.doc.text(other.reader.cursor.page)?.clamp(other.reader.cursor);
    self.reader.scroll = other.reader.scroll;
    Ok(())
  }

  /// Returns to a saved position and view.
  pub fn restore(&mut self, entry: &Entry) -> Result<()> {
    self.layout.zoom = entry.zoom.clamp(0.1, 8.0);
    self.marks.clone_from(&entry.marks);
    self.layout.crop = entry.crop;
    self.layout.rotation = entry.rotation % 360 / 90 * 90;
    self.layout.reflow(&self.doc.pages);
    let page = PageId(entry.cursor.page.0.min(self.doc.pages.len() - 1));
    let cursor = Position { page, ..entry.cursor };
    self.reader.cursor = self.doc.text(page)?.clamp(cursor);
    self.reader.scroll = self.layout.pages[page.0].top;
    self.ensure_cursor()
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

#[cfg(test)]
mod tests {
  use super::*;

  fn entry(path: &str, page: usize, used: u64) -> Entry {
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
  fn save_merges_concurrent_readers() -> Result<()> {
    let root = tempfile::tempdir()?;
    let file = root.path().join("vpdf/state.json");
    let mut other = State {
      recent: vec![entry("/b.pdf", 2, 20), entry("/a.pdf", 9, 5)],
      ..State::default()
    };
    let history = History {
      commands: vec!["page 3".into(), "ocr".into()],
      ..History::default()
    };
    other.save(&file, &history)?;
    let mut own = State {
      recent: vec![entry("/a.pdf", 4, 30), entry("/c.pdf", 1, 10)],
      ..State::default()
    };
    let history = History {
      commands: vec!["ocr".into(), "help".into()],
      searches: vec!["attention".into()],
      ..History::default()
    };
    own.save(&file, &history)?;
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
