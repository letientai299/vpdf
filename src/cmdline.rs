use crate::Result;
use crossterm::event::KeyEvent;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use vpdf_ui::commands::{self, Complete, View};
use vpdf_ui::prompt::{History, Prompt, PromptEvent, PromptKind};

/// Feeds `key` to `prompt`, completing commands and editing the line in
/// `$EDITOR`; returns the event left for the caller.
pub fn feed(
  prompt: &mut Prompt,
  key: KeyEvent,
  history: &History,
  view: View,
) -> Result<PromptEvent> {
  Ok(match prompt.feed(key, history) {
    PromptEvent::Editor => {
      let edited = crate::editor::edit(prompt.text())?;
      prompt.replace(&edited);
      PromptEvent::Edited
    }
    PromptEvent::Complete if prompt.kind() == PromptKind::Command => {
      let (start, items) = candidates(prompt.before_cursor(), view);
      prompt.complete(start, items);
      PromptEvent::Edited
    }
    event => event,
  })
}

/// Whether `program` is an executable file on `PATH`.
pub fn on_path(program: &str) -> bool {
  std::env::var_os("PATH").is_some_and(|paths| {
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
  })
}

pub fn candidates(before: &str, view: View) -> (usize, Vec<String>) {
  commands::candidates(before, view, on_path, |kind, argument| match kind {
    Complete::Path => paths(argument),
    Complete::Language => languages().to_vec(),
    _ => Vec::new(),
  })
}

/// Expands a leading `~` to the home directory.
pub fn expand(path: &str) -> PathBuf {
  let home = std::env::var_os("HOME").map(PathBuf::from);
  match (path.strip_prefix('~'), home) {
    (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
      home.join(rest.trim_start_matches('/'))
    }
    _ => PathBuf::from(path),
  }
}

/// Directories and PDFs whose names extend the typed path.
fn paths(argument: &str) -> Vec<String> {
  let split = argument.rfind('/').map_or(0, |slash| slash + 1);
  let (directory, prefix) = argument.split_at(split);
  let root = if directory.is_empty() { ".".into() } else { expand(directory) };
  let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
  let folded = prefix.to_lowercase();
  let mut names: Vec<_> = entries
    .filter_map(|entry| {
      let entry = entry.ok()?;
      let name = entry.file_name().into_string().ok()?;
      let hidden = name.starts_with('.') && !prefix.starts_with('.');
      if hidden || !name.to_lowercase().starts_with(&folded) {
        return None;
      }
      // Only symlinks need a stat to tell folders apart.
      let kind = entry.file_type().ok()?;
      if kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()) {
        return Some(format!("{name}/"));
      }
      Path::new(&name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        .then_some(name)
    })
    .collect();
  names.sort_unstable();
  commands::prefixed(names, prefix)
    .into_iter()
    .map(|name| format!("{directory}{name}"))
    .collect()
}

fn languages() -> &'static [String] {
  static LANGUAGES: OnceLock<Vec<String>> = OnceLock::new();
  LANGUAGES.get_or_init(|| {
    let Ok(output) = Command::new("tesseract")
      .arg("--list-langs")
      .stdin(Stdio::null())
      .stderr(Stdio::null())
      .output()
    else {
      return Vec::new();
    };
    String::from_utf8_lossy(&output.stdout)
      .lines()
      .skip(1)
      .map(str::trim)
      .filter(|line| !line.is_empty())
      .map(str::to_owned)
      .collect()
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  #[test]
  fn completes_pdfs_and_directories() -> Result<()> {
    let root = tempfile::tempdir()?;
    for name in ["a.pdf", "b.PDF", "notes.txt", ".hidden.pdf"] {
      std::fs::write(root.path().join(name), "")?;
    }
    std::fs::create_dir(root.path().join("archive"))?;
    let base = format!("{}/", root.path().display());
    let names = |items: Vec<String>| -> Vec<String> {
      items.iter().map(|item| item.replacen(&base, "", 1)).collect()
    };
    assert_eq!(names(paths(&base)), ["a.pdf", "archive/", "b.PDF"]);
    assert_eq!(names(paths(&format!("{base}a"))), ["a.pdf", "archive/"]);
    assert_eq!(names(paths(&format!("{base}."))), [".hidden.pdf"]);
    for (command, start) in [("tabe", 5), ("tabnew", 7)] {
      let items = candidates(&format!("{command} {base}b"), View::Document);
      assert_eq!((items.0, names(items.1)), (start, vec!["b.PDF".to_owned()]));
    }
    Ok(())
  }
}
