use crate::Result;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use vpdf_model::input::KeyEvent;
use vpdf_ui::commands::{self, Complete, View};
use vpdf_ui::prompt::{History, Prompt, PromptEvent, PromptKind};

const URI_PATH: &percent_encoding::AsciiSet =
  &percent_encoding::NON_ALPHANUMERIC
    .remove(b'/')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

pub(crate) struct Completion<'a> {
  pub view: View,
  pub terminal: bool,
  pub tabs: &'a [String],
}

impl Completion<'_> {
  /// Feeds `key` to `prompt`, completing commands and editing the line in
  /// `$EDITOR`; returns the event left for the caller.
  pub fn feed(
    &self,
    prompt: &mut Prompt,
    key: KeyEvent,
    history: &History,
    topics: impl FnOnce(&str) -> Result<Vec<vpdf_ui::topics::Candidate>>,
  ) -> Result<PromptEvent> {
    Ok(match prompt.feed(key, history) {
      PromptEvent::Editor => {
        let edited = crate::programs::edit(prompt.text(), self.terminal)?;
        prompt.replace(&edited);
        PromptEvent::Edited
      }
      PromptEvent::Complete if prompt.kind() == PromptKind::Command => {
        let candidates =
          topics(help_query(prompt.before_cursor()).unwrap_or(""))?;
        let names: Vec<_> =
          candidates.iter().map(|item| item.name.clone()).collect();
        let (start, items) = complete(
          prompt.before_cursor(),
          self.view,
          self.terminal,
          &names,
          self.tabs,
        );
        let help = help_query(prompt.before_cursor()).is_some();
        prompt.complete(start, items);
        if help {
          prompt.describe(|name| describe(name, &candidates));
        } else if query(prompt.before_cursor(), Complete::Tab).is_some() {
          prompt.describe(|number| {
            number
              .parse::<usize>()
              .ok()
              .and_then(|index| index.checked_sub(1))
              .and_then(|index| self.tabs.get(index))
              .map_or_else(|| number.into(), |name| format!("{number}  {name}"))
          });
        }
        PromptEvent::Edited
      }
      event => event,
    })
  }
}

pub(crate) fn help_query(before: &str) -> Option<&str> {
  query(before, Complete::Help)
}

fn query(before: &str, kind: Complete) -> Option<&str> {
  let commands::Resolution::Command { spec, argument } =
    commands::resolve(before, View::Document, |_| true, |_| false).ok()?
  else {
    return None;
  };
  (spec.complete == kind).then_some(argument)
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
  complete(before, view, terminal, &[], &[])
}

pub(crate) fn complete(
  before: &str,
  view: View,
  terminal: bool,
  topics: &[String],
  tabs: &[String],
) -> (usize, Vec<String>) {
  commands::candidates(
    before,
    view,
    |program| available(program, terminal),
    |kind, argument| match kind {
      Complete::Path => paths(argument),
      Complete::Theme => filesystem_paths(argument, |path| {
        path.to_str().is_some_and(crate::config::ThemeChoice::is_path)
      }),
      Complete::Language => languages().to_vec(),
      Complete::Tab => {
        (1..=tabs.len()).map(|index| index.to_string()).collect()
      }
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

pub fn reference(argument: &str) -> Result<vpdf_model::reference::Reference> {
  let mut reference =
    vpdf_model::reference::Reference::parse(argument.as_ref())?;
  if let Ok(rest) = reference.path.strip_prefix("~")
    && let Some(home) = std::env::var_os("HOME")
  {
    reference.path = PathBuf::from(home).join(rest);
  }
  Ok(reference)
}

pub(crate) fn picker_root(argument: &str) -> Option<PathBuf> {
  let root = expand(if argument.is_empty() { "." } else { argument });
  (argument.is_empty() || root.is_dir()).then_some(root)
}

/// Directories and PDFs whose names extend the typed path.
fn paths(argument: &str) -> Vec<String> {
  let (path, fragment) = argument
    .split_once('#')
    .map_or((argument, ""), |(path, _)| (path, &argument[path.len()..]));
  let uri = path.get(..5).is_some_and(|s| s.eq_ignore_ascii_case("file:"));
  let decoded = if uri {
    let Ok(reference) = reference(path) else { return Vec::new() };
    let Ok(path) = reference.path.into_os_string().into_string() else {
      return Vec::new();
    };
    path
  } else {
    path.to_owned()
  };
  filesystem_paths(&decoded, is_pdf)
    .into_iter()
    .filter_map(|candidate| {
      let candidate = if uri || candidate.contains('#') {
        let absolute = std::env::current_dir().ok()?.join(expand(&candidate));
        let path = absolute.to_str()?;
        let encoded = percent_encoding::utf8_percent_encode(path, URI_PATH);
        format!("file://{encoded}")
      } else {
        candidate
      };
      Some(format!("{candidate}{fragment}"))
    })
    .collect()
}

fn filesystem_paths(
  argument: &str,
  accepts: impl Fn(&Path) -> bool,
) -> Vec<String> {
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
      accepts(Path::new(&name)).then_some(name)
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
  fn tab_menu_inserts_numbers() -> Result<()> {
    use vpdf_model::input::{KeyCode, KeyModifiers};
    let history = History::default();
    let names = vec!["first.pdf".into(), "second.pdf".into()];
    let completion =
      Completion { view: View::Document, terminal: true, tabs: &names };
    let mut prompt = Prompt::new(PromptKind::Command);
    prompt.paste("tabnext ");
    let key = KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE);
    completion.feed(&mut prompt, key, &history, |_| Ok(Vec::new()))?;
    let menu = prompt.menu().unwrap();
    assert_eq!(menu.items(), ["1", "2"]);
    assert_eq!(menu.labels(), ["1  first.pdf", "2  second.pdf"]);
    completion.feed(&mut prompt, key, &history, |_| Ok(Vec::new()))?;
    completion.feed(&mut prompt, key, &history, |_| Ok(Vec::new()))?;
    assert_eq!(prompt.text(), "tabnext 2");
    assert_eq!(
      complete("tabnext 3", View::Document, true, &[], &names).1,
      [] as [String; 0]
    );
    Ok(())
  }

  #[test]
  fn completes_theme_files() -> Result<()> {
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join("custom.toml"), "")?;
    std::fs::write(root.path().join("document.pdf"), "")?;
    std::fs::create_dir(root.path().join("palettes"))?;
    let prefix = format!("{}/", root.path().display());
    for command in ["theme", "colorscheme"] {
      let (_, items) =
        candidates(&format!("{command} {prefix}"), View::Home, true);
      assert_eq!(
        items,
        [format!("{prefix}custom.toml"), format!("{prefix}palettes/")]
      );
    }
    Ok(())
  }

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
    for (command, start) in [("tabe", 5), ("tabnew", 7), ("fzf", 4)] {
      let items =
        candidates(&format!("{command} {base}b"), View::Document, false);
      assert_eq!((items.0, names(items.1)), (start, vec!["b.PDF".to_owned()]));
    }
    Ok(())
  }
  #[test]
  fn completion_preserves_targets() -> Result<()> {
    let root = tempfile::tempdir()?;
    for name in ["a b%.pdf", "hash#name.pdf"] {
      std::fs::write(root.path().join(name), "")?;
    }
    let base = root.path().display().to_string();
    assert_eq!(
      paths(&format!("{base}/a#page=2")),
      [format!("{base}/a b%.pdf#page=2")]
    );
    let input = format!("file://{base}/a#page=2&comment=note");
    let candidates = paths(&input);
    assert_eq!(candidates.len(), 1);
    let parsed = reference(&candidates[0])?;
    assert_eq!(parsed.path, root.path().join("a b%.pdf"));
    assert_eq!(parsed.operations.len(), 2);
    let candidates = paths(&format!("{base}/hash"));
    assert_eq!(candidates.len(), 1);
    let parsed = reference(&candidates[0])?;
    assert_eq!(parsed.path, root.path().join("hash#name.pdf"));
    assert_eq!(parsed.operations, [] as [vpdf_model::reference::Operation; 0]);
    Ok(())
  }
}
