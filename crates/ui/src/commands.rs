#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Complete {
  Nothing,
  Language,
  Tab,
  Help,
  Config,
  Theme,
  Recolor,
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

impl Spec {
  pub fn switch(&self, argument: &str, current: bool) -> crate::Result<bool> {
    match argument {
      "" => Ok(!current),
      "on" => Ok(true),
      "off" => Ok(false),
      _ => anyhow::bail!("Usage: :{} {}", self.name, self.usage),
    }
  }

  pub fn available(&self, view: View) -> bool {
    match view {
      View::Document => true,
      View::Recent => self.view != View::Document,
      View::Home => self.view == View::Home,
    }
  }

  pub fn topic(&self) -> String {
    format!("ex.{}", self.name.replace('!', "-beside"))
  }
}

pub fn theme(argument: &str) -> crate::Result<vpdf_model::theme::Theme> {
  vpdf_model::theme::Theme::parse(argument).ok_or_else(|| {
    anyhow::anyhow!("appearance.theme: {}", vpdf_model::theme::Theme::EXPECTED)
  })
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

/// Generates `Id` and one `COMMANDS` entry per command.
macro_rules! commands {
  ($(
    $(#[$attr:meta])*
    $id:ident $name:literal [$($alias:literal),*] $usage:literal
      $complete:ident $view:ident $help:literal $(requires $program:literal)?;
  )*) => {
    /// What a command does, one per `COMMANDS` entry.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum Id {
      $($id,)*
    }

    pub const COMMANDS: &[Spec] = &[$(
      $(#[$attr])*
      Spec {
        id: Id::$id,
        name: $name,
        aliases: &[$($alias),*],
        usage: $usage,
        help: $help,
        complete: Complete::$complete,
        requires: commands!(@some $($program)?),
        view: View::$view,
      },
    )*];
  };
  (@some) => { None };
  (@some $value:literal) => { Some($value) };
}

// id, name, aliases, usage, completion, view, help
commands! {
  Config "config" [] "reload" Config Home
    "reload configuration";
  Theme "theme" ["colorscheme"] "<name|path>" Theme Home "switch UI theme";
  Edit "edit" ["e"] "[file]" Path Home "pick or open a PDF in this pane";
  Open "open" [] "[file]" Path Home
    "pick or open a PDF in a new tab";
  TabEdit "tabedit" ["tabe"] "[file]" Path Home
    "pick or open a PDF in a new tab";
  TabNew "tabnew" [] "[file]" Path Home
    "open a PDF or recent list in a new tab";
  BDelete "bdelete" ["bd"] "" Nothing Home "unload this document";
  New "new" [] "[file]" Path Home "split above with Recent or a PDF";
  VNew "vnew" [] "[file]" Path Home "split left with Recent or a PDF";
  Split "split" ["sp"] "[file]" Path Home "split this pane above";
  VSplit "vsplit" ["vs"] "[file]" Path Home "split this pane to the left";
  Close "close" ["clo"] "" Nothing Home "close this pane";
  Only "only" ["on"] "" Nothing Home "close every other pane";
  TabNext "tabnext" ["tabn"] "[n]" Tab Home "go to the next or nth tab";
  TabPrevious "tabprevious" ["tabp", "tabN"] "" Nothing Home
    "go to the previous tab";
  TabClose "tabclose" ["tabc"] "" Nothing Home "close this tab";
  Recent "recent" ["oldfiles", "enew"] "" Nothing Home
    "list recently opened PDFs";
  Fzf "fzf" [] "[path]" Path Home "pick a PDF below this directory"
    requires "fzf";
  Marks "marks" [] "" Nothing Recent "list marks";
  Jumps "jumps" [] "" Nothing Recent "list jump destinations";
  Annotations "annotations" ["annots"] "" Nothing Recent
    "list highlights and notes";
  Toc "toc" ["outline"] "" Nothing Recent "open the table of contents";
  Overview "overview" ["pages"] "" Nothing Recent "show page thumbnails";
  CursorLine "cursorline" [] "[on|off]" Switch Document
    "toggle the Cursor/Visual line band (default on)";
  Links "links" [] "[on|off]" Switch Document
    "toggle link underlines";
  Recolor "recolor" [] "[dark|sepia [amount]|off]" Recolor Document
    "recolor document pixels";
  Crop "crop" [] "[on|off]" Switch Document "toggle cropped page margins";
  Mouse "mouse" [] "[on|off]" Switch Recent
    "toggle mouse capture; off: terminal selects";
  Perf "perf" [] "[on|off]" Switch Home "toggle persistent performance stats";
  Cancel "cancel" [] "" Nothing Recent
    "stop pane search, OCR, outline, region jobs";
  NoHlSearch "nohlsearch" ["noh"] "" Nothing Document "hide search highlights";
  Text "text" [] "" Nothing Recent "preview extracted page text";
  Info "info" [] "" Nothing Recent "document, reader, and terminal info";
  Reload "reload" ["e!"] "" Nothing Document
    "reread the file from disk";
  #[cfg(target_os = "macos")]
  Print "print" [] "" Nothing Document "open the system print dialog";
  Password "password" [] "" Nothing Document "unlock and reopen the document";
  Ocr "ocr" [] "[lang]" Language Document "recognize the page with Tesseract";
  Help "help" ["h"] "[topic]" Help Home "open the built-in manual";
  HelpBeside "help!" ["h!"] "[topic]" Help Home "open help beside this pane";
  Version "version" [] "" Nothing Home "show build information";
  Quit "quit" ["q", "q!", "x", "wq"] "" Nothing Home
    "close this pane, tab, then reader";
  QuitAll "qall" ["qa", "quitall", "qa!"] "" Nothing Home "close every tab";
}

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
  let mut start = before.len() - argument.len();
  let items = match lookup(name, installed)
    .filter(|spec| spec.available(view))
    .map(|spec| spec.complete)
  {
    Some(Complete::Language) => {
      let split = argument.rfind('+').map_or(0, |index| index + 1);
      let (selected, prefix) = argument.split_at(split);
      start += split;
      let items = supply(Complete::Language, prefix)
        .into_iter()
        .filter(|item| !selected.split('+').any(|used| used == item))
        .collect();
      prefixed(items, prefix)
    }
    Some(Complete::Tab) => prefixed(supply(Complete::Tab, argument), argument),
    Some(Complete::Path) => supply(Complete::Path, argument),
    Some(Complete::Help) => supply(Complete::Help, argument),
    Some(Complete::Theme) => {
      let mut items: Vec<_> = vpdf_model::theme::Theme::ALL
        .iter()
        .map(|theme| theme.name().to_owned())
        .collect();
      items.extend(supply(Complete::Theme, argument));
      prefixed(items, argument)
    }
    Some(Complete::Recolor) => {
      prefixed(vec!["dark".into(), "sepia".into(), "off".into()], argument)
    }
    Some(Complete::Config) => prefixed(vec!["reload".into()], argument),
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
  fn themes_validate_and_complete() -> crate::Result<()> {
    use vpdf_model::theme::Theme;
    for name in ["theme", "colorscheme"] {
      let spec = lookup(name, |_| true).unwrap();
      assert_eq!(spec.id, Id::Theme);
      for view in [View::Home, View::Recent, View::Document] {
        assert!(spec.available(view));
        let (_, items) =
          candidates(&format!("{name} "), view, |_| true, |_, _| Vec::new());
        assert_eq!(
          items,
          Theme::ALL
            .iter()
            .map(|theme| theme.name().to_owned())
            .collect::<Vec<_>>()
        );
        let (_, items) =
          candidates(&format!("{name} d"), view, |_| true, |_, _| Vec::new());
        assert_eq!(items, ["dark"]);
      }
    }
    for &selected in Theme::ALL {
      assert_eq!(theme(selected.name())?, selected);
    }
    for argument in ["", "unknown", "nord extra"] {
      assert!(theme(argument).is_err());
    }
    Ok(())
  }

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
  fn printing_is_macos_only() {
    let found = commands(|_| true).find(|spec| spec.id == Id::Print);
    assert_eq!(found.is_some(), cfg!(target_os = "macos"));
    for view in [View::Home, View::Recent, View::Document] {
      let items = candidates("print", view, |_| true, |_, _| Vec::new()).1;
      let available = cfg!(target_os = "macos") && view == View::Document;
      assert_eq!(items == ["print"], available);
      assert_eq!(
        resolve("print", view, |_| true, |_| false).is_ok(),
        available
      );
    }
  }

  #[test]
  fn opening_metadata_agrees() {
    for name in ["e", "edit", "ed", "open", "op", "tabe", "tabedit"] {
      let spec = lookup(name, |_| false).unwrap();
      assert_eq!(
        spec.id,
        match name.chars().next() {
          Some('e') => Id::Edit,
          Some('o') => Id::Open,
          _ => Id::TabEdit,
        }
      );
      assert_eq!(spec.usage, "[file]");
      assert_eq!(spec.complete, Complete::Path);
      for view in [View::Home, View::Recent, View::Document] {
        assert!(spec.available(view));
        let (_, items) = candidates(
          &format!("{name} "),
          view,
          |_| false,
          |kind, argument| {
            assert_eq!(kind, Complete::Path);
            assert_eq!(argument, "");
            vec!["notes.pdf".into()]
          },
        );
        assert_eq!(items, ["notes.pdf"]);
      }
    }
  }

  #[test]
  fn completes_compound_languages() {
    for (before, start, expected) in [
      ("ocr ", 4, vec!["eng", "deu", "vie"]),
      ("ocr e", 4, vec!["eng"]),
      ("ocr eng+", 8, vec!["deu", "vie"]),
      ("ocr eng+v", 8, vec!["vie"]),
      ("ocr eng+vie+E", 12, vec![]),
      ("ocr deu+V", 8, vec!["vie"]),
      ("ocr eng+vie+d", 12, vec!["deu"]),
    ] {
      let (offset, items) = candidates(
        before,
        View::Document,
        |_| true,
        |kind, _| {
          assert_eq!(kind, Complete::Language);
          ["eng", "deu", "vie"].map(str::to_owned).to_vec()
        },
      );
      assert_eq!(offset, start, "{before}");
      assert_eq!(items, expected, "{before}");
    }
  }

  #[test]
  fn completes_tab_numbers() {
    for name in ["tabnext", "tabn"] {
      for view in [View::Recent, View::Document] {
        assert_eq!(
          candidates(
            &format!("{name} 1"),
            view,
            |_| true,
            |kind, prefix| {
              assert_eq!(kind, Complete::Tab);
              assert_eq!(prefix, "1");
              vec!["1".into(), "2".into(), "10".into()]
            }
          )
          .1,
          ["1", "10"]
        );
      }
    }
    assert_eq!(
      candidates("tabnext ", View::Home, |_| true, |_, _| Vec::new()).1,
      [] as [String; 0]
    );
  }

  #[test]
  fn fzf_completes_paths() {
    for view in [View::Home, View::Recent, View::Document] {
      assert_eq!(
        candidates(
          "fzf bo",
          view,
          |_| true,
          |kind, argument| {
            assert_eq!(kind, Complete::Path);
            assert_eq!(argument, "bo");
            vec!["books/".into(), "book.pdf".into()]
          }
        ),
        (4, vec!["books/".into(), "book.pdf".into()])
      );
      assert_eq!(
        candidates("fzf bo", view, |_| false, |_, _| unreachable!()).1,
        [] as [String; 0]
      );
    }
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
      ("split", true, true),
      ("toc", false, true),
      ("help", true, true),
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
        vec!["recent", "recolor", "reload"]
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
      candidates("split p", View::Home, |_| true, |_, _| Vec::new()).1,
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

#[cfg(test)]
mod recolor_tests {
  use super::*;

  #[test]
  fn presets_complete_in_documents() {
    let spec = lookup("recolor", |_| true).unwrap();
    assert!(spec.available(View::Document));
    assert!(!spec.available(View::Home));
    assert!(!spec.available(View::Recent));
    for (input, expected) in [
      ("recolor ", vec!["dark", "sepia", "off"]),
      ("recolor s", vec!["sepia"]),
      ("recolor dark ", vec![]),
    ] {
      let (_, items) =
        candidates(input, View::Document, |_| true, |_, _| vec![]);
      assert_eq!(items, expected);
    }
  }
}
