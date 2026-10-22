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

/// What a command does, one per `COMMANDS` entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Id {
  Edit,
  TabEdit,
  TabNew,
  Split,
  VSplit,
  Close,
  Only,
  TabNext,
  TabPrevious,
  TabClose,
  Recent,
  Fzf,
  Marks,
  Jumps,
  Annotations,
  Toc,
  Overview,
  CursorLine,
  Crop,
  Mouse,
  Perf,
  Cancel,
  NoHlSearch,
  Text,
  Info,
  Reload,
  Password,
  Ocr,
  Help,
  Version,
  Quit,
  QuitAll,
}

pub struct Spec {
  pub id: Id,
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
  id: Id,
  name: &'static str,
  aliases: &'static [&'static str],
  usage: &'static str,
  help: &'static str,
) -> Spec {
  Spec {
    id,
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
  pub fn switch(&self, argument: &str, current: bool) -> crate::Result<bool> {
    match argument {
      "" => Ok(!current),
      "on" => Ok(true),
      "off" => Ok(false),
      _ => anyhow::bail!("Usage: :{} {}", self.name, self.usage),
    }
  }

  const fn in_view(self, view: View) -> Self {
    Self { view, ..self }
  }

  fn available(&self, view: View) -> bool {
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
fn parse(line: &str) -> (&str, &str) {
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
  spec(Id::Edit, "edit", &["e"], "<file>", "open a PDF in this tab")
    .completes(Complete::Path)
    .in_view(View::Home),
  spec(
    Id::TabEdit,
    "tabedit",
    &["tabe", "open"],
    "<file>",
    "open a PDF in a new tab",
  )
  .completes(Complete::Path)
  .in_view(View::Home),
  spec(
    Id::TabNew,
    "tabnew",
    &[],
    "[file]",
    "open a PDF or recent list in a new tab",
  )
  .completes(Complete::Path)
  .in_view(View::Home),
  spec(Id::Split, "split", &["sp"], "[file]", "split this pane above")
    .completes(Complete::Path)
    .in_view(View::Recent),
  spec(Id::VSplit, "vsplit", &["vs"], "[file]", "split this pane to the left")
    .completes(Complete::Path)
    .in_view(View::Recent),
  spec(Id::Close, "close", &["clo"], "", "close this pane")
    .in_view(View::Recent),
  spec(Id::Only, "only", &["on"], "", "close every other pane")
    .in_view(View::Recent),
  spec(Id::TabNext, "tabnext", &["tabn"], "[n]", "go to the next or nth tab")
    .in_view(View::Recent),
  spec(
    Id::TabPrevious,
    "tabprevious",
    &["tabp", "tabN"],
    "",
    "go to the previous tab",
  )
  .in_view(View::Recent),
  spec(Id::TabClose, "tabclose", &["tabc"], "", "close this tab")
    .in_view(View::Home),
  spec(Id::Recent, "recent", &["oldfiles"], "", "list recently opened PDFs")
    .in_view(View::Home),
  spec(Id::Fzf, "fzf", &[], "", "pick a PDF below this directory")
    .requires("fzf")
    .in_view(View::Home),
  spec(Id::Marks, "marks", &[], "", "list marks").in_view(View::Recent),
  spec(Id::Jumps, "jumps", &[], "", "list jump destinations")
    .in_view(View::Recent),
  spec(
    Id::Annotations,
    "annotations",
    &["annots"],
    "",
    "list highlights and notes",
  )
  .in_view(View::Recent),
  spec(Id::Toc, "toc", &["outline"], "", "open the table of contents")
    .in_view(View::Recent),
  spec(Id::Overview, "overview", &["pages"], "", "show page thumbnails")
    .in_view(View::Recent),
  spec(
    Id::CursorLine,
    "cursorline",
    &[],
    "[on|off]",
    "toggle the Cursor/Visual line band (default on)",
  )
  .completes(Complete::Switch),
  spec(Id::Crop, "crop", &[], "[on|off]", "toggle cropped page margins")
    .completes(Complete::Switch),
  spec(
    Id::Mouse,
    "mouse",
    &[],
    "[on|off]",
    "toggle mouse capture; off: terminal selects",
  )
  .completes(Complete::Switch)
  .in_view(View::Recent),
  spec(
    Id::Perf,
    "perf",
    &[],
    "[on|off]",
    "toggle persistent performance stats",
  )
  .completes(Complete::Switch)
  .in_view(View::Home),
  spec(
    Id::Cancel,
    "cancel",
    &[],
    "",
    "stop pane search, OCR, outline, region jobs",
  )
  .in_view(View::Recent),
  spec(Id::NoHlSearch, "nohlsearch", &["noh"], "", "hide search highlights"),
  spec(Id::Text, "text", &[], "", "preview extracted page text")
    .in_view(View::Recent),
  spec(Id::Info, "info", &[], "", "document, reader, and terminal info")
    .in_view(View::Recent),
  spec(Id::Reload, "reload", &[], "", "reread the file from disk"),
  spec(Id::Password, "password", &[], "", "unlock and reopen the document"),
  spec(Id::Ocr, "ocr", &[], "[lang]", "recognize the page with Tesseract")
    .completes(Complete::Language),
  spec(Id::Help, "help", &["h"], "", "show the key reference")
    .in_view(View::Recent),
  spec(Id::Version, "version", &[], "", "show build information")
    .in_view(View::Home),
  spec(Id::Quit, "quit", &["q"], "", "close this pane, tab, then reader")
    .in_view(View::Home),
  spec(Id::QuitAll, "qall", &["qa", "quitall"], "", "close every tab")
    .in_view(View::Home),
];

/// Command named `name` or aliased to it.
fn exact(
  name: &str,
  installed: impl Fn(&str) -> bool,
) -> Option<&'static Spec> {
  commands(installed).find(|s| s.name == name || s.aliases.contains(&name))
}

/// Exact names and aliases win; otherwise a unique prefix.
fn lookup(
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

pub enum Resolution<'a> {
  Empty,
  Page(&'a str),
  Command { spec: &'static Spec, argument: &'a str },
}

pub fn resolve(
  line: &str,
  view: View,
  installed: impl Fn(&str) -> bool,
  label: impl Fn(&str) -> bool,
) -> crate::Result<Resolution<'_>> {
  let (name, argument) = parse(line);
  if name.is_empty() {
    return Ok(Resolution::Empty);
  }
  let spec = lookup(name, &installed);
  if view == View::Home {
    let spec =
      spec.ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
    anyhow::ensure!(
      spec.available(view),
      "Open a document before :{}",
      spec.name
    );
    return Ok(Resolution::Command { spec, argument });
  }
  // Labels must not bypass command availability in a recent-document view.
  if view == View::Recent {
    anyhow::ensure!(
      spec.is_some_and(|spec| spec.available(view)),
      "Open a document before :{name}"
    );
  }
  // Labels like `iv` beat prefixes like `:info`, not exact names or aliases.
  let page = exact(name, installed).is_none() && label(name);
  if page || name.chars().all(|ch| ch.is_ascii_digit()) {
    return Ok(Resolution::Page(name));
  }
  let spec = spec.ok_or_else(|| anyhow::anyhow!("Unknown command: {name}"))?;
  Ok(Resolution::Command { spec, argument })
}

/// Byte offset and replacements for the word before the cursor.
pub fn candidates(
  before: &str,
  view: View,
  installed: impl Fn(&str) -> bool,
  supply: impl Fn(Complete, &str) -> Vec<String>,
) -> (usize, Vec<String>) {
  let start = before.len() - before.trim_start().len();
  let Some((name, rest)) = before[start..].split_once(char::is_whitespace)
  else {
    let word = &before[start..];
    let names = commands(&installed)
      .filter(|spec| spec.available(view))
      .map(|spec| spec.name.to_owned())
      .collect();
    return (start, prefixed(names, word));
  };
  let argument = rest.trim_start();
  let start = before.len() - argument.len();
  let items = match lookup(name, installed)
    .filter(|spec| spec.available(view))
    .map(|spec| spec.complete)
  {
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
  fn switches_toggle_and_set() -> crate::Result<()> {
    for name in ["cursorline", "mouse", "crop"] {
      let spec = lookup(name, |_| true).unwrap();
      for current in [false, true] {
        for (argument, expected) in
          [("", !current), ("on", true), ("off", false)]
        {
          assert_eq!(spec.switch(argument, current)?, expected);
        }
        for argument in ["invalid", "on off"] {
          assert_eq!(
            spec.switch(argument, current).unwrap_err().to_string(),
            format!("Usage: :{name} [on|off]")
          );
        }
      }
    }
    Ok(())
  }

  #[test]
  fn each_command_has_one_spec() {
    for spec in COMMANDS {
      let same = COMMANDS.iter().filter(|other| other.id == spec.id).count();
      assert_eq!(same, 1, "{}", spec.name);
    }
  }

  #[test]
  fn completes_command_names() {
    let candidates =
      |before| candidates(before, View::Document, |_| true, |_, _| Vec::new());
    assert_eq!(candidates("  pa"), (2, vec!["password".into()]));
    assert_eq!(candidates("info 1").1, Vec::<String>::new());
    assert_eq!(candidates("cu"), (0, vec!["cursorline".into()]));
    assert_eq!(
      candidates("cursorline "),
      (11, vec!["on".into(), "off".into()])
    );
    assert_eq!(candidates("cu of"), (3, vec!["off".into()]));
    assert_eq!(candidates("crop ").1, ["on", "off"]);
    assert_eq!(candidates("crop of"), (5, vec!["off".into()]));
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
    let missing =
      candidates("fz", View::Document, |_| false, |_, _| unreachable!());
    assert_eq!(missing.1, [] as [String; 0]);
  }

  #[test]
  fn views_preserve_command_scope() {
    for (name, home, recent) in [
      ("edit", true, true),
      ("perf", true, true),
      ("version", true, true),
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

  #[test]
  fn resolution_preserves_labels_and_view_error_order() {
    for (view, line, found, expected, reads) in [
      (View::Document, "  ", true, "empty", 0),
      (View::Document, " i ", true, "page:i", 1),
      (View::Document, "info", true, "info:", 0),
      (View::Document, "e  paper.pdf ", true, "edit:paper.pdf", 0),
      (View::Document, "3", false, "page:3", 1),
      (View::Document, "c", true, "page:c", 1),
      (View::Document, "c", false, "Unknown command: c", 1),
      (View::Recent, "rel", true, "Open a document before :rel", 0),
      (View::Recent, "3", true, "Open a document before :3", 0),
      (View::Recent, "per on", true, "page:per", 1),
      (View::Home, "rel", true, "Open a document before :reload", 0),
      (View::Home, "c", true, "Unknown command: c", 0),
      (View::Home, "e paper.pdf", true, "edit:paper.pdf", 0),
    ] {
      let consulted = std::cell::Cell::new(0);
      let resolved = resolve(
        line,
        view,
        |_| true,
        |_| {
          consulted.set(consulted.get() + 1);
          found
        },
      );
      let actual = match resolved {
        Ok(Resolution::Empty) => "empty".into(),
        Ok(Resolution::Page(name)) => format!("page:{name}"),
        Ok(Resolution::Command { spec, argument }) => {
          format!("{}:{argument}", spec.name)
        }
        Err(error) => error.to_string(),
      };
      assert_eq!(actual, expected, "{view:?} {line}");
      assert_eq!(consulted.get(), reads, "{view:?} {line}");
    }
  }

  #[test]
  fn completion_uses_execution_scope() {
    for view in [View::Home, View::Recent, View::Document] {
      let complete =
        |before| candidates(before, view, |_| true, |_, _| unreachable!());
      let expected = if view == View::Document {
        vec!["recent", "reload"]
      } else {
        vec!["recent"]
      };
      assert_eq!(
        complete("  re"),
        (2, expected.into_iter().map(str::to_owned).collect())
      );
      assert_eq!(complete("RE").1, complete("re").1);
      assert_eq!(
        complete("mouse ").1,
        if view == View::Home { vec![] } else { vec!["on", "off"] }
      );
      assert_eq!(complete("tab ").1, Vec::<String>::new());
    }
    assert_eq!(
      candidates("ocr e", View::Home, |_| true, |_, _| unreachable!()).1,
      Vec::<String>::new()
    );
    assert_eq!(
      candidates("split p", View::Home, |_| true, |_, _| unreachable!()).1,
      Vec::<String>::new()
    );
    assert_eq!(
      candidates(
        "e p",
        View::Home,
        |_| true,
        |kind, prefix| {
          assert_eq!(kind, Complete::Path);
          assert_eq!(prefix, "p");
          vec!["paper.pdf".into()]
        }
      ),
      (2, vec!["paper.pdf".into()])
    );
  }
}
