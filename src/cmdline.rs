use crate::Result;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use vpdf_model::input::KeyEvent;
use vpdf_ui::commands::{self, Complete, View};
use vpdf_ui::prompt::{History, Prompt, PromptEvent, PromptKind};

/// Feeds `key` to `prompt`, completing commands and editing the line in
/// `$EDITOR`; returns the event left for the caller.
pub fn feed(
  prompt: &mut Prompt,
  key: KeyEvent,
  history: &History,
  view: View,
  terminal: bool,
  topics: impl FnOnce(&str) -> Result<Vec<vpdf_ui::topics::Candidate>>,
) -> Result<PromptEvent> {
  Ok(match prompt.feed(key, history) {
    PromptEvent::Editor => {
      let edited = crate::programs::edit(prompt.text(), terminal)?;
      prompt.replace(&edited);
      PromptEvent::Edited
    }
    PromptEvent::Complete if prompt.kind() == PromptKind::Command => {
      let candidates =
        topics(help_query(prompt.before_cursor()).unwrap_or(""))?;
      let names: Vec<_> =
        candidates.iter().map(|item| item.name.clone()).collect();
      let (start, items) =
        complete(prompt.before_cursor(), view, terminal, &names);
      let help = help_query(prompt.before_cursor()).is_some();
      prompt.complete(start, items);
      if help {
        prompt.describe(|name| describe(name, &candidates));
      }
      PromptEvent::Edited
    }
    event => event,
  })
}

pub(crate) fn help_query(before: &str) -> Option<&str> {
  let commands::Resolution::Command { spec, argument } =
    commands::resolve(before, View::Document, |_| true, |_| false).ok()?
  else {
    return None;
  };
  matches!(spec.id, commands::Id::Help | commands::Id::HelpBeside)
    .then_some(argument)
}

pub(crate) fn describe(
  name: &str,
  candidates: &[vpdf_ui::topics::Candidate],
) -> String {
  candidates.iter().find(|candidate| candidate.name == name).map_or_else(
    || name.into(),
    |candidate| {
      let group = match (&candidate.destination, &candidate.action) {
        (vpdf_ui::topics::Destination::Authored(_), _) => "Guide",
        (vpdf_ui::topics::Destination::Configuration, _) => "Settings",
        (vpdf_ui::topics::Destination::Command(_), _) => "Command",
        (_, Some(_)) => "Key",
        _ => "Action",
      };
      format!("{name}  [{group}] {}", candidate.description)
    },
  )
}

/// Whether `program` is an executable file on `PATH`.
pub fn on_path(program: &str) -> bool {
  std::env::var_os("PATH").is_some_and(|paths| {
    std::env::split_paths(&paths).any(|dir| dir.join(program).is_file())
  })
}

pub(crate) fn available(program: &str, terminal: bool) -> bool {
  (!terminal && program == "fzf") || on_path(program)
}

pub fn candidates(
  before: &str,
  view: View,
  terminal: bool,
) -> (usize, Vec<String>) {
  complete(before, view, terminal, &[])
}

pub(crate) fn complete(
  before: &str,
  view: View,
  terminal: bool,
  topics: &[String],
) -> (usize, Vec<String>) {
  commands::candidates(
    before,
    view,
    |program| available(program, terminal),
    |kind, argument| match kind {
      Complete::Path => paths(argument),
      Complete::Language => languages().to_vec(),
      Complete::Help => topics
        .iter()
        .filter(|name| name.starts_with(argument))
        .cloned()
        .collect(),
      _ => Vec::new(),
    },
  )
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
      is_pdf(Path::new(&name)).then_some(name)
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

/// Whether `path` names a PDF by its extension.
pub(crate) fn is_pdf(path: &Path) -> bool {
  path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
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
      let items =
        candidates(&format!("{command} {base}b"), View::Document, true);
      assert_eq!((items.0, names(items.1)), (start, vec!["b.PDF".to_owned()]));
    }
    Ok(())
  }
}
