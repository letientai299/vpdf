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
}

const fn spec(
  name: &'static str,
  aliases: &'static [&'static str],
  usage: &'static str,
  help: &'static str,
) -> Spec {
  Spec { name, aliases, usage, help, complete: Complete::Nothing }
}

impl Spec {
  const fn completes(self, complete: Complete) -> Self {
    Self { complete, ..self }
  }
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
  spec("tabnext", &["tabn"], "[n]", "go to the next or nth tab"),
  spec("tabprevious", &["tabp", "tabN"], "", "go to the previous tab"),
  spec("tabclose", &["tabc"], "", "close this tab"),
  spec("recent", &["oldfiles"], "", "list recently opened PDFs"),
  spec("page", &[], "<n|label>", "jump to a page number or label"),
  spec("toc", &["outline"], "", "open the table of contents"),
  spec("nohlsearch", &["noh"], "", "hide search highlights"),
  spec("text", &[], "", "preview extracted page text"),
  spec("info", &[], "", "document, cache, and timing info"),
  spec("reload", &[], "", "reread the file from disk"),
  spec("password", &[], "", "unlock and reopen the document"),
  spec("ocr", &[], "[lang]", "recognize the page with Tesseract")
    .completes(Complete::Language),
  spec("help", &["h"], "", "show the key reference"),
  spec("quit", &["q"], "", "close this tab, then the reader"),
  spec("qall", &["qa", "quitall"], "", "close every tab"),
];

/// Exact names and aliases win; otherwise a unique prefix.
pub fn lookup(name: &str) -> Option<&'static Spec> {
  if let Some(spec) =
    COMMANDS.iter().find(|s| s.name == name || s.aliases.contains(&name))
  {
    return Some(spec);
  }
  let mut matches = COMMANDS.iter().filter(|s| s.name.starts_with(name));
  let first = matches.next()?;
  matches.next().is_none().then_some(first)
}

/// Byte offset and replacements for the word before the cursor.
pub fn candidates(before: &str) -> (usize, Vec<String>) {
  let start = before.len() - before.trim_start().len();
  let Some((name, rest)) = before[start..].split_once(char::is_whitespace)
  else {
    let word = &before[start..];
    let names = COMMANDS
      .iter()
      .filter(|spec| spec.name.starts_with(word))
      .map(|spec| spec.name.to_owned())
      .collect();
    return (start, names);
  };
  let argument = rest.trim_start();
  let start = before.len() - argument.len();
  let items = match lookup(name).map(|spec| spec.complete) {
    Some(Complete::Language) => languages()
      .iter()
      .filter(|language| language.starts_with(argument))
      .cloned()
      .collect(),
    Some(Complete::Path) => paths(argument),
    _ => Vec::new(),
  };
  (start, items)
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
  let mut items: Vec<_> = entries
    .filter_map(|entry| {
      let entry = entry.ok()?;
      let name = entry.file_name().into_string().ok()?;
      if !name.starts_with(prefix)
        || name.starts_with('.') && !prefix.starts_with('.')
      {
        return None;
      }
      if entry.path().is_dir() {
        return Some(format!("{directory}{name}/"));
      }
      Path::new(&name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        .then(|| format!("{directory}{name}"))
    })
    .collect();
  items.sort_unstable();
  items
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
      ("rel", Some("reload")),
      ("p", None),
      ("PdfPage", None),
    ];
    for (input, expected) in cases {
      assert_eq!(lookup(input).map(|spec| spec.name), expected, "{input}");
    }
  }

  #[test]
  fn completes_pdfs_and_directories() -> crate::Result<()> {
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
    assert_eq!(candidates("  pa"), (2, vec!["page".into(), "password".into()]));
    assert_eq!(candidates("page 1").1, Vec::<String>::new());
  }
}
