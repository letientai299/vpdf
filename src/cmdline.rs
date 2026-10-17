use crate::Result;
use crate::prompt::{History, Prompt, PromptEvent, PromptKind};
use crossterm::event::KeyEvent;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Complete {
  Nothing,
  Language,
  Path,
}

pub struct Spec {
  pub name: &'static str,
  pub aliases: &'static [&'static str],
  pub usage: &'static str,
  pub help: &'static str,
  pub complete: Complete,
  /// Executable the command needs on `PATH`.
  pub requires: Option<&'static str>,
}

const fn spec(
  name: &'static str,
  aliases: &'static [&'static str],
  usage: &'static str,
  help: &'static str,
) -> Spec {
  Spec {
    name,
    aliases,
    usage,
    help,
    complete: Complete::Nothing,
    requires: None,
  }
}

impl Spec {
  const fn completes(self, complete: Complete) -> Self {
    Self { complete, ..self }
  }

  const fn requires(self, program: &'static str) -> Self {
    Self { requires: Some(program), ..self }
  }
}

/// Command name and trimmed argument of `line`.
pub fn parse(line: &str) -> (&str, &str) {
  let line = line.trim();
  line
    .split_once(char::is_whitespace)
    .map_or((line, ""), |(name, rest)| (name, rest.trim()))
}

/// Feeds `key` to `prompt`, completing commands and editing the line in
/// `$EDITOR`; returns the event left for the caller.
pub fn feed(
  prompt: &mut Prompt,
  key: KeyEvent,
  history: &History,
) -> Result<PromptEvent> {
  Ok(match prompt.feed(key, history) {
    PromptEvent::Editor => {
      let edited = crate::editor::edit(&prompt.text)?;
      prompt.replace(&edited);
      PromptEvent::Edited
    }
    PromptEvent::Complete if prompt.kind == PromptKind::Command => {
      let (start, items) = candidates(prompt.before_cursor());
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

/// Commands whose required programs are installed.
pub fn commands() -> impl Iterator<Item = &'static Spec> {
  COMMANDS.iter().filter(|spec| spec.requires.is_none_or(on_path))
}

pub const COMMANDS: &[Spec] = &[
  spec("edit", &["e"], "<file>", "open a PDF in this tab")
    .completes(Complete::Path),
  spec(
    "tabedit",
    &["tabe", "tabnew", "open"],
    "<file>",
    "open a PDF in a new tab",
  )
  .completes(Complete::Path),
  spec("split", &["sp"], "[file]", "split this pane above")
    .completes(Complete::Path),
  spec("vsplit", &["vs"], "[file]", "split this pane to the left")
    .completes(Complete::Path),
  spec("close", &["clo"], "", "close this pane"),
  spec("only", &["on"], "", "close every other pane"),
  spec("tabnext", &["tabn"], "[n]", "go to the next or nth tab"),
  spec("tabprevious", &["tabp", "tabN"], "", "go to the previous tab"),
  spec("tabclose", &["tabc"], "", "close this tab"),
  spec("recent", &["oldfiles"], "", "list recently opened PDFs"),
  spec("fzf", &[], "", "pick a PDF below this directory").requires("fzf"),
  spec("marks", &[], "", "list marks"),
  spec("jumps", &[], "", "list jump destinations"),
  spec("annotations", &["annots"], "", "list highlights and notes"),
  spec("toc", &["outline"], "", "open the table of contents"),
  spec("overview", &["pages"], "", "show page thumbnails"),
  spec("cursorline", &[], "", "toggle the Visual line band (default on)"),
  spec("crop", &[], "", "toggle cropped page margins"),
  spec("cancel", &[], "", "stop pane search, OCR, outline, region jobs"),
  spec("nohlsearch", &["noh"], "", "hide search highlights"),
  spec("text", &[], "", "preview extracted page text"),
  spec("info", &[], "", "document, reader, and terminal info"),
  spec("reload", &[], "", "reread the file from disk"),
  spec("password", &[], "", "unlock and reopen the document"),
  spec("ocr", &[], "[lang]", "recognize the page with Tesseract")
    .completes(Complete::Language),
  spec("help", &["h"], "", "show the key reference"),
  spec("quit", &["q"], "", "close this pane, tab, then reader"),
  spec("qall", &["qa", "quitall"], "", "close every tab"),
];

/// Command named `name` or aliased to it.
pub fn exact(name: &str) -> Option<&'static Spec> {
  commands().find(|s| s.name == name || s.aliases.contains(&name))
}

/// Exact names and aliases win; otherwise a unique prefix.
pub fn lookup(name: &str) -> Option<&'static Spec> {
  if let Some(spec) = exact(name) {
    return Some(spec);
  }
  let mut matches = commands().filter(|s| s.name.starts_with(name));
  let first = matches.next()?;
  matches.next().is_none().then_some(first)
}

/// Byte offset and replacements for the word before the cursor.
pub fn candidates(before: &str) -> (usize, Vec<String>) {
  let start = before.len() - before.trim_start().len();
  let Some((name, rest)) = before[start..].split_once(char::is_whitespace)
  else {
    let word = &before[start..];
    let names = commands().map(|spec| spec.name.to_owned()).collect();
    return (start, prefixed(names, word));
  };
  let argument = rest.trim_start();
  let start = before.len() - argument.len();
  let items = match lookup(name).map(|spec| spec.complete) {
    Some(Complete::Language) => prefixed(languages().to_vec(), argument),
    Some(Complete::Path) => paths(argument),
    _ => Vec::new(),
  };
  (start, items)
}

/// Items starting with `typed`; case-insensitive when nothing matches
/// exactly, like zsh's `matcher-list '' 'm:{a-zA-Z}={A-Za-z}'`.
fn prefixed(items: Vec<String>, typed: &str) -> Vec<String> {
  let exact: Vec<_> =
    items.iter().filter(|item| item.starts_with(typed)).cloned().collect();
  if !exact.is_empty() {
    return exact;
  }
  let folded = typed.to_lowercase();
  items
    .into_iter()
    .filter(|item| item.to_lowercase().starts_with(&folded))
    .collect()
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
  prefixed(names, prefix)
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
  fn resolves_names_aliases_and_prefixes() {
    let cases = [
      ("q", Some("quit")),
      ("noh", Some("nohlsearch")),
      ("cursorline", Some("cursorline")),
      ("cu", Some("cursorline")),
      ("rel", Some("reload")),
      ("c", None),
      ("PdfPage", None),
    ];
    for (input, expected) in cases {
      assert_eq!(lookup(input).map(|spec| spec.name), expected, "{input}");
    }
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
    let (start, items) = candidates(&format!("tabe {base}b"));
    assert_eq!((start, names(items)), (5, vec!["b.PDF".to_owned()]));
    Ok(())
  }

  #[test]
  fn completes_command_names() {
    assert_eq!(candidates("  pa"), (2, vec!["password".into()]));
    assert_eq!(candidates("info 1").1, Vec::<String>::new());
    assert_eq!(candidates("cu"), (0, vec!["cursorline".into()]));
    assert_eq!(candidates("PAS").1, ["password"]);
  }
}
