#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Complete {
  Nothing,
  Language,
  Path,
  /// `on` or `off`.
  Switch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum View {
  Home,
  Recent,
  Document,
}

pub struct Spec {
  pub name: &'static str,
  pub aliases: &'static [&'static str],
  pub usage: &'static str,
  pub help: &'static str,
  pub complete: Complete,
  /// Executable the command needs on `PATH`.
  pub requires: Option<&'static str>,
  view: View,
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
    view: View::Document,
  }
}

impl Spec {
  const fn in_view(self, view: View) -> Self {
    Self { view, ..self }
  }

  pub fn available(&self, view: View) -> bool {
    match view {
      View::Document => true,
      View::Recent => self.view != View::Document,
      View::Home => self.view == View::Home,
    }
  }

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

/// Commands whose required programs are installed.
pub fn commands(
  installed: impl Fn(&str) -> bool,
) -> impl Iterator<Item = &'static Spec> {
  COMMANDS.iter().filter(move |spec| spec.requires.is_none_or(&installed))
}

pub const COMMANDS: &[Spec] = &[
  spec("edit", &["e"], "<file>", "open a PDF in this tab")
    .completes(Complete::Path)
    .in_view(View::Home),
  spec("tabedit", &["tabe", "open"], "<file>", "open a PDF in a new tab")
    .completes(Complete::Path)
    .in_view(View::Home),
  spec("tabnew", &[], "[file]", "open a PDF or recent list in a new tab")
    .completes(Complete::Path)
    .in_view(View::Home),
  spec("split", &["sp"], "[file]", "split this pane above")
    .completes(Complete::Path)
    .in_view(View::Recent),
  spec("vsplit", &["vs"], "[file]", "split this pane to the left")
    .completes(Complete::Path)
    .in_view(View::Recent),
  spec("close", &["clo"], "", "close this pane").in_view(View::Recent),
  spec("only", &["on"], "", "close every other pane").in_view(View::Recent),
  spec("tabnext", &["tabn"], "[n]", "go to the next or nth tab")
    .in_view(View::Recent),
  spec("tabprevious", &["tabp", "tabN"], "", "go to the previous tab")
    .in_view(View::Recent),
  spec("tabclose", &["tabc"], "", "close this tab").in_view(View::Home),
  spec("recent", &["oldfiles"], "", "list recently opened PDFs")
    .in_view(View::Home),
  spec("fzf", &[], "", "pick a PDF below this directory")
    .requires("fzf")
    .in_view(View::Home),
  spec("marks", &[], "", "list marks").in_view(View::Recent),
  spec("jumps", &[], "", "list jump destinations").in_view(View::Recent),
  spec("annotations", &["annots"], "", "list highlights and notes")
    .in_view(View::Recent),
  spec("toc", &["outline"], "", "open the table of contents")
    .in_view(View::Recent),
  spec("overview", &["pages"], "", "show page thumbnails")
    .in_view(View::Recent),
  spec("cursorline", &[], "", "toggle the Visual line band (default on)"),
  spec("crop", &[], "", "toggle cropped page margins"),
  spec("mouse", &[], "[on|off]", "toggle mouse capture; off: terminal selects")
    .completes(Complete::Switch)
    .in_view(View::Recent),
  spec("perf", &[], "[on|off]", "toggle persistent performance stats")
    .completes(Complete::Switch)
    .in_view(View::Home),
  spec("cancel", &[], "", "stop pane search, OCR, outline, region jobs")
    .in_view(View::Recent),
  spec("nohlsearch", &["noh"], "", "hide search highlights"),
  spec("text", &[], "", "preview extracted page text").in_view(View::Recent),
  spec("info", &[], "", "document, reader, and terminal info")
    .in_view(View::Recent),
  spec("reload", &[], "", "reread the file from disk"),
  spec("password", &[], "", "unlock and reopen the document"),
  spec("ocr", &[], "[lang]", "recognize the page with Tesseract")
    .completes(Complete::Language),
  spec("help", &["h"], "", "show the key reference").in_view(View::Recent),
  spec("quit", &["q"], "", "close this pane, tab, then reader")
    .in_view(View::Home),
  spec("qall", &["qa", "quitall"], "", "close every tab").in_view(View::Home),
];

/// Command named `name` or aliased to it.
pub fn exact(
  name: &str,
  installed: impl Fn(&str) -> bool,
) -> Option<&'static Spec> {
  commands(installed).find(|s| s.name == name || s.aliases.contains(&name))
}

/// Exact names and aliases win; otherwise a unique prefix.
pub fn lookup(
  name: &str,
  installed: impl Fn(&str) -> bool,
) -> Option<&'static Spec> {
  if let Some(spec) = exact(name, &installed) {
    return Some(spec);
  }
  let mut matches = commands(installed).filter(|s| s.name.starts_with(name));
  let first = matches.next()?;
  matches.next().is_none().then_some(first)
}

/// Byte offset and replacements for the word before the cursor.
pub fn candidates(
  before: &str,
  installed: impl Fn(&str) -> bool,
  supply: impl Fn(Complete, &str) -> Vec<String>,
) -> (usize, Vec<String>) {
  let start = before.len() - before.trim_start().len();
  let Some((name, rest)) = before[start..].split_once(char::is_whitespace)
  else {
    let word = &before[start..];
    let names = commands(&installed).map(|spec| spec.name.to_owned()).collect();
    return (start, prefixed(names, word));
  };
  let argument = rest.trim_start();
  let start = before.len() - argument.len();
  let items = match lookup(name, installed).map(|spec| spec.complete) {
    Some(Complete::Language) => {
      prefixed(supply(Complete::Language, argument), argument)
    }
    Some(Complete::Path) => supply(Complete::Path, argument),
    Some(Complete::Switch) => {
      prefixed(vec!["on".into(), "off".into()], argument)
    }
    _ => Vec::new(),
  };
  (start, items)
}

/// Items starting with `typed`; case-insensitive when nothing matches
/// exactly, like zsh's `matcher-list '' 'm:{a-zA-Z}={A-Za-z}'`.
pub fn prefixed(items: Vec<String>, typed: &str) -> Vec<String> {
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
      assert_eq!(
        lookup(input, |_| true).map(|spec| spec.name),
        expected,
        "{input}"
      );
    }
  }

  #[test]
  fn completes_command_names() {
    let candidates = |before| candidates(before, |_| true, |_, _| Vec::new());
    assert_eq!(candidates("  pa"), (2, vec!["password".into()]));
    assert_eq!(candidates("info 1").1, Vec::<String>::new());
    assert_eq!(candidates("cu"), (0, vec!["cursorline".into()]));
    assert_eq!(candidates("PAS").1, ["password"]);
    assert_eq!(candidates("mouse ").1, ["on", "off"]);
    assert_eq!(candidates("mouse of"), (6, vec!["off".into()]));
  }
  #[test]
  fn prerequisites_are_explicit() {
    assert!(lookup("fzf", |_| false).is_none());
    assert_eq!(
      lookup("fz", |program| program == "fzf").map(|s| s.name),
      Some("fzf")
    );
    let missing = candidates("fz", |_| false, |_, _| unreachable!());
    assert_eq!(missing.1, [] as [String; 0]);
  }

  #[test]
  fn views_preserve_command_scope() {
    for (name, home, recent) in [
      ("edit", true, true),
      ("perf", true, true),
      ("tabclose", true, true),
      ("split", false, true),
      ("toc", false, true),
      ("help", false, true),
      ("mouse", false, true),
      ("ocr", false, false),
      ("reload", false, false),
      ("crop", false, false),
      ("password", false, false),
    ] {
      let spec = lookup(name, |_| true).unwrap();
      assert_eq!(spec.available(View::Home), home, "{name}");
      assert_eq!(spec.available(View::Recent), recent, "{name}");
      assert!(spec.available(View::Document), "{name}");
      for alias in spec.aliases {
        let alias = lookup(alias, |_| true).unwrap();
        assert_eq!(alias.available(View::Home), home);
        assert_eq!(alias.available(View::Recent), recent);
      }
    }
  }
}
