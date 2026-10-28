use crate::Result;
use std::collections::BTreeMap;
use std::fmt::Write;
use vpdf_model::input::{KeyCode, KeyModifiers};
use vpdf_pdf::manual::{Chapter, Line, Section};
use vpdf_ui::bindings::Binding;
use vpdf_ui::bindings::catalog::Family;
use vpdf_ui::bindings::{Bindings, Context, Control, HintAction, Target};
use vpdf_ui::keys::Action;
use vpdf_ui::topics::{Destination, Registry, authored, key_anchor};

struct Shortcut<'a> {
  row: &'a Binding,
  modes: Vec<Context>,
  purpose: String,
  anchors: Vec<String>,
}

fn preferred<'a>(rows: &[&'a Binding], defaults: &Bindings) -> &'a Binding {
  let preferred = defaults.shortcut(rows[0].context, &rows[0].target);
  rows
    .iter()
    .min_by_key(|row| {
      let sequence = row.sequence.to_string();
      let uppercase = row
        .sequence
        .keys()
        .iter()
        .filter(|key| {
          key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char(ch) if ch.is_ascii_uppercase())
        })
        .count();
      (
        sequence != preferred,
        row.sequence.keys().len(),
        uppercase,
        sequence.len(),
        sequence,
      )
    })
    .unwrap()
}

/// A key sequence that enters a continuation mode.
struct Prefix {
  from: Context,
  into: Context,
  keys: Vec<vpdf_model::input::KeyEvent>,
}

/// Default sequences that start an operator or a text object.
fn prefixes(rows: &[Binding]) -> Vec<Prefix> {
  rows
    .iter()
    .filter_map(|row| {
      let into = match row.target {
        Target::Reader(Action::Yank) | Target::Control(Control::Yank) => {
          Context::Operator
        }
        Target::Control(Control::Object { .. }) => Context::Object,
        _ => return None,
      };
      Some(Prefix {
        from: row.context,
        into,
        keys: row.sequence.keys().to_vec(),
      })
    })
    .collect()
}

/// Reader modes whose prefixes lead into a continuation mode.
const fn origins(context: Context) -> &'static [Context] {
  use Context as C;
  match context {
    C::Operator => &[C::Normal, C::Cursor, C::Visual],
    C::Object => &[C::Normal, C::Cursor, C::Visual, C::Operator],
    _ => &[],
  }
}

/// Lists one entry per key; contexts label differing effects.
fn bindings(
  rows: &[&Binding],
  defaults: &Bindings,
  prefixes: &[Prefix],
) -> Vec<(Line, Vec<String>)> {
  let mut contexts = BTreeMap::<_, Vec<&Binding>>::new();
  for row in rows {
    contexts.entry((row.context, row.purpose.clone())).or_default().push(row);
  }
  let shortcuts: Vec<_> = contexts
    .into_iter()
    .map(|((context, purpose), rows)| Shortcut {
      row: preferred(&rows, defaults),
      modes: vec![context],
      purpose,
      anchors: rows
        .iter()
        .map(|row| key_anchor(row.context, &row.sequence))
        .collect(),
    })
    .collect();
  let mut entries = BTreeMap::<String, Vec<Shortcut>>::new();
  let mut notes = Vec::new();
  for shortcut in &shortcuts {
    let context = shortcut.modes[0];
    let full = shortcuts
      .iter()
      .filter(|other| {
        let full = other.row.sequence.keys();
        let tail = shortcut.row.sequence.keys();
        origins(context).contains(&other.modes[0])
          && origins(other.modes[0]).is_empty()
          && full.len() > tail.len()
          && full.ends_with(tail)
          && prefixes.iter().any(|prefix| {
            prefix.from == other.modes[0]
              && prefix.into == context
              && prefix.keys == full[..full.len() - tail.len()]
          })
      })
      .min_by_key(|other| other.row.sequence.keys().len());
    if let Some(full) = full {
      notes.push((full.row.sequence.to_string(), shortcut));
      continue;
    }
    let keys = shortcut.row.sequence.to_string();
    let effects = entries.entry(keys).or_default();
    if let Some(effect) =
      effects.iter_mut().find(|effect| effect.purpose == shortcut.purpose)
    {
      effect.modes.push(context);
      effect.anchors.extend(shortcut.anchors.iter().cloned());
    } else {
      effects.push(Shortcut {
        row: shortcut.row,
        modes: vec![context],
        purpose: shortcut.purpose.clone(),
        anchors: shortcut.anchors.clone(),
      });
    }
  }
  entries
    .into_iter()
    .map(|(keys, effects)| {
      let mut description = effects
        .iter()
        .map(|effect| {
          format!(
            "{} in {} {}",
            effect.purpose,
            effect
              .modes
              .iter()
              .map(|mode| mode.name())
              .collect::<Vec<_>>()
              .join(", "),
            if effect.modes.len() == 1 { "mode" } else { "modes" }
          )
        })
        .collect::<Vec<_>>()
        .join("; ");
      description.push('.');
      let mut anchors: Vec<_> =
        effects.iter().flat_map(|effect| effect.anchors.clone()).collect();
      for (_, continuation) in notes.iter().filter(|(full, _)| *full == keys) {
        let _ = write!(
          description,
          " In {} mode, `{}` completes it.",
          continuation.modes[0].name(),
          continuation.row.sequence
        );
        anchors.extend(continuation.anchors.iter().cloned());
      }
      anchors.sort_unstable();
      anchors.dedup();
      (Line::Binding { keys, description }, anchors)
    })
    .collect()
}

fn teaching(id: &str) -> &'static str {
  if let Some(panel) = id.strip_prefix("panel.") {
    return match panel {
      "annotations" => "links",
      "figures" => "regions",
      "marks" | "jumps" => "marks",
      "results" => "search",
      "text" => "zoom",
      "recent" => "state",
      "help" => "keys",
      "info" | "version" => "troubleshooting",
      "close" => "panes",
      _ => "navigation",
    };
  }
  let family = id.split('.').next().unwrap_or(id);
  match family {
    "search" | "result" | "word_search" => "search",
    "definition" | "hint" | "hints" | "peek" | "follow" | "url" => "links",
    "zoom" | "fit" | "text" => "zoom",
    "window" | "tab" | "local_focus" => "panes",
    "region" | "figure" => "regions",
    "yank" | "object" | "visual" | "selection" | "register" => "selection",
    "mark" | "history" => "marks",
    "prompt" | "count" | "command" => "keys",
    _ => "navigation",
  }
}

fn grouped(rows: &[Binding]) -> BTreeMap<String, Vec<&Binding>> {
  let mut grouped = BTreeMap::<_, Vec<_>>::new();
  for row in rows {
    grouped.entry(row.target.topic()).or_default().push(row);
  }
  grouped
}

fn family(target: &Target) -> Option<(String, &'static str, &'static str)> {
  if matches!(target, Target::Hint(HintAction::Type { .. })) {
    return Some((
      Target::Hint(HintAction::Type { character: 'a' }).topic(),
      "{label}",
      "Type the displayed link hint label in hints mode.",
    ));
  }
  let (control, syntax, description) = match target {
    Target::Reader(Action::Mark(_)) | Target::Control(Control::SetMark) => (
      Control::SetMark,
      "m{mark}",
      "Set a local mark (a-z) or global mark (A-Z).",
    ),
    Target::Reader(Action::GoMark(_, exact))
    | Target::Control(Control::GoMark { exact }) => (
      Control::GoMark { exact: *exact },
      if *exact { "`{mark}" } else { "'{mark}" },
      if *exact {
        "Jump to the mark's saved glyph."
      } else {
        "Jump to the mark's saved line."
      },
    ),
    Target::Control(Control::Digit { .. }) => (
      Control::Digit { digit: 0 },
      "{count}",
      "Type a count before a motion to repeat it.",
    ),
    _ => return None,
  };
  Some((Target::Control(control).topic(), syntax, description))
}

pub(super) fn chapters() -> Result<Vec<Chapter>> {
  let defaults = Bindings::default();
  let snapshot = defaults.snapshot();
  let registry = Registry::new(authored(), snapshot.clone())?;
  let guides: BTreeMap<_, _> = authored()
    .into_iter()
    .map(|topic| (topic.id.as_str().to_owned(), topic.title))
    .collect();
  let prefixes = prefixes(&snapshot.defaults);
  let rows = grouped(&snapshot.defaults);
  let mut entries = BTreeMap::<String, Chapter>::new();
  for topic in registry.topics() {
    let Destination::Action(original) = &topic.destination else { continue };
    let rows = rows.get(original.as_str()).map_or(&[][..], Vec::as_slice);
    let mut anchors = BTreeMap::new();
    let (id, title, lines, listed) = if let Some((id, syntax, description)) =
      rows.first().and_then(|row| family(&row.target))
    {
      (
        id,
        syntax.to_owned(),
        vec![Line::Binding {
          keys: syntax.into(),
          description: description.into(),
        }],
        true,
      )
    } else {
      let mut lines = Vec::new();
      for (line, names) in bindings(rows, &defaults, &prefixes) {
        anchors.insert(lines.len(), names);
        lines.push(line);
      }
      if lines.is_empty() {
        lines.push(Line::Binding {
          keys: topic.title.clone(),
          description: "No built-in shortcut.".into(),
        });
      }
      (
        original.as_str().to_owned(),
        topic.title.clone(),
        lines,
        !rows.is_empty(),
      )
    };
    let catalog = rows.first().map_or_else(
      || format!("action.{}", original.as_str()),
      |row| Family::topic_of(&row.target),
    );
    let catalog = registry
      .topics()
      .any(|topic| topic.id.as_str() == catalog)
      .then_some(catalog);
    let chapter = entries.entry(id.clone()).or_insert_with(|| {
      let guide = teaching(&id);
      Chapter {
        id: id.clone(),
        title,
        lines,
        related: guide.into(),
        related_title: guides[guide].clone(),
        anchors,
        section: Some(Section::Keys),
        listed,
        aliases: Vec::new(),
        links: catalog
          .into_iter()
          .map(|topic| ("Configure:".into(), topic))
          .collect(),
      }
    });
    if original.as_str() != id {
      chapter.aliases.push(original.as_str().into());
    }
  }
  let mut chapters: Vec<_> = entries.into_values().collect();
  chapters.extend(super::catalog::chapters(
    &defaults,
    &guides,
    teaching,
    |row| {
      family(&row.target).map_or_else(
        || {
          let keys = defaults.shortcut(row.context, &row.target);
          if keys.is_empty() { row.sequence.to_string() } else { keys }
        },
        |(_, syntax, _)| syntax.into(),
      )
    },
  )?);
  chapters.push(configuration()?);
  chapters.extend(
    vpdf_ui::commands::COMMANDS
      .iter()
      .map(|spec| command_chapter(spec, &guides)),
  );
  chapters.sort_by_key(|chapter| chapter.section);
  Ok(chapters)
}

fn command_guide(id: vpdf_ui::commands::Id) -> &'static str {
  use vpdf_ui::commands::Id;
  match id {
    Id::Config | Id::Theme => "configuration",
    Id::BDelete
    | Id::New
    | Id::VNew
    | Id::Split
    | Id::VSplit
    | Id::Close
    | Id::Only
    | Id::TabNext
    | Id::TabPrevious
    | Id::TabClose
    | Id::Help
    | Id::HelpBeside
    | Id::Quit
    | Id::QuitAll => "panes",
    Id::Marks | Id::Jumps => "marks",
    Id::Annotations | Id::Links => "links",
    Id::Toc | Id::Overview => "navigation",
    Id::CursorLine | Id::Crop | Id::Recolor | Id::Text => "zoom",
    Id::Ocr => "ocr",
    Id::Perf | Id::Cancel | Id::NoHlSearch => "status",
    Id::Mouse | Id::Info | Id::Version => "troubleshooting",
    _ => "state",
  }
}

fn command_syntax(spec: &vpdf_ui::commands::Spec) -> String {
  use vpdf_ui::commands::{Resolution, View, resolve};
  let mut start = spec.name.len();
  for (end, _) in spec.name.char_indices().rev() {
    if !matches!(
      resolve(&spec.name[..end], View::Document, |_| true, |_| false),
      Ok(Resolution::Command { spec: found, .. }) if found.id == spec.id
    ) {
      break;
    }
    start = end;
  }
  let name = if start < spec.name.len() {
    format!("{}[{}]", &spec.name[..start], &spec.name[start..])
  } else {
    spec.name.into()
  };
  let mut syntax = format!(":{name}");
  for alias in spec.aliases {
    if spec.name.starts_with(alias) && alias.len() >= start {
      continue;
    }
    syntax.push_str("  :");
    syntax.push_str(alias);
  }
  if !spec.usage.is_empty() {
    syntax.push(' ');
    syntax.push_str(spec.usage);
  }
  syntax
}

fn command_example(id: vpdf_ui::commands::Id) -> &'static str {
  use vpdf_ui::commands::Id;
  match id {
    Id::Config => ":config edit",
    Id::Theme => ":theme dark",
    Id::Recolor => ":recolor sepia 30",
    Id::Edit => ":edit book.pdf",
    Id::Open => ":open",
    Id::TabEdit => ":tabedit notes.pdf",
    Id::TabNew => ":tabnew",
    Id::BDelete => ":bd",
    Id::New => ":new",
    Id::VNew => ":vnew",
    Id::Split => ":split",
    Id::VSplit => ":vsplit",
    Id::TabNext => ":tabnext 2",
    Id::Ocr => ":ocr eng+vie",
    Id::Help => ":help :split",
    Id::HelpBeside => ":help! search",
    _ => "",
  }
}

fn recolor_help() -> Line {
  format!(
    "No argument selects dark at {0}. Dark and sepia default to {0}. \
       Amounts are unsigned decimal integers, clamped to {1}. \
       Off or amount zero restores original pixels. \
       Saved per document; split views inherit the setting. \
       Open views remain independent. Source PDFs stay unchanged. \
       Try :recolor dark 80 or :recolor sepia 30 on this help PDF. \
       Use :recolor off to restore its original colors. \
       Help recoloring lasts for this view's session. \
       Interface themes do not change help page colors.",
    vpdf_model::recolor::DEFAULT_AMOUNT,
    vpdf_model::recolor::MAX_AMOUNT,
  )
  .into()
}

fn command_chapter(
  spec: &vpdf_ui::commands::Spec,
  guides: &BTreeMap<String, String>,
) -> Chapter {
  use vpdf_ui::commands::Id;
  let guide = command_guide(spec.id);
  let mut lines = vec![Line::Binding {
    keys: command_syntax(spec),
    description: format!(
      "{}{}.",
      spec.help[..1].to_uppercase(),
      spec.help[1..].replace(" on)", " `on`)").replace(" off:", " `off`:")
    ),
  }];
  if let Some(program) = spec.requires {
    let requirement = if spec.id == Id::Fzf {
      "Terminal: requires fzf on PATH; hidden when missing. \
       Native: uses the system file picker without fzf."
        .into()
    } else {
      format!("Requires {program} on PATH; hidden when missing.")
    };
    lines.push(requirement.into());
  }
  let example = command_example(spec.id);
  if !example.is_empty() {
    lines.push(format!("For example: {example}").into());
  }
  lines.extend(command_notes(spec));
  Chapter {
    id: spec.topic(),
    title: format!(":{}", spec.name),
    lines,
    related: guide.into(),
    related_title: guides[guide].clone(),
    anchors: BTreeMap::new(),
    section: Some(Section::Commands),
    listed: true,
    aliases: Vec::new(),
    links: Vec::new(),
  }
}

fn configuration() -> Result<Chapter> {
  let config = crate::config::Settings::default().reference();
  let mut lines = Vec::new();
  let mut anchors = BTreeMap::new();
  for setting in config.settings {
    let field = crate::config::describe(&setting.name)?;
    anchors.insert(lines.len(), vec![setting.name.clone()]);
    lines.push(Line::Binding {
      keys: setting.name,
      description: format!(
        "{}. Default: {}. {}",
        field.summary,
        setting.default,
        field.details()
      ),
    });
  }
  Ok(Chapter {
    id: "config".into(),
    title: "Configuration defaults".into(),
    lines,
    related: "configuration".into(),
    related_title: "Configuring the reader".into(),
    section: Some(Section::Configuration),
    listed: true,
    aliases: Vec::new(),
    links: Vec::new(),
    anchors,
  })
}

fn command_notes(spec: &vpdf_ui::commands::Spec) -> Vec<Line> {
  use vpdf_ui::commands::Id;
  let mut lines = Vec::new();
  match spec.id {
    Id::Recolor => lines.push(recolor_help()),
    Id::Config => lines.push(
      concat!(
        "edit opens the file, creating it when missing: the native app uses ",
        "the system editor; the terminal runs $EDITOR, else nvim, else vim, ",
        "in this pane. A file present at launch, or created by edit, reloads ",
        "when saved; use reload for a file created another way."
      )
      .into(),
    ),
    Id::Fzf => lines.push(
      concat!(
        "Tab completes PDFs and directories. ",
        "A PDF opens directly; a directory sets the picker root. ",
        "No path uses the current directory."
      )
      .into(),
    ),
    Id::Ocr => lines.push(
      concat!(
        "Requires Tesseract and installed language data. ",
        "Uses configured language when omitted. Tab completes after +."
      )
      .into(),
    ),
    Id::Edit | Id::Open | Id::TabEdit => lines.push(
      concat!(
        "No file: choose a PDF with the frontend's picker. ",
        "Without fzf, :open starts path entry with Tab completion. ",
        "The native app uses the system file picker. ",
        "Cancel keeps the current view."
      )
      .into(),
    ),
    Id::Reload | Id::Password => lines.push(
      "Requires a file-backed PDF; unavailable for built-in help.".into(),
    ),
    Id::BDelete => lines.push(
      concat!(
        "Unloads this document and reopens its alternate, else shows Recent. ",
        "On help, reopens the replaced document, else closes the pane. ",
        "On Recent, closes the pane. No filename argument."
      )
      .into(),
    ),
    Id::New | Id::VNew => lines.push(
      concat!(
        "No file: split with Recent. With a file: split and open it. ",
        "An already open file focuses its pane."
      )
      .into(),
    ),
    Id::Recent => lines.push(
      concat!(
        "Saves and unloads the document. ",
        "q or Escape reopens its alternate from saved state."
      )
      .into(),
    ),
    Id::Split | Id::VSplit => lines.push(
      concat!(
        "No file: duplicate this view. ",
        "An already open file focuses its existing pane."
      )
      .into(),
    ),
    Id::CursorLine | Id::Links | Id::Crop | Id::Mouse | Id::Perf => lines.push(
      "No argument toggles; `on` and `off` set an explicit value.".into(),
    ),
    _ => {}
  }
  #[cfg(target_os = "macos")]
  if spec.id == Id::Print {
    lines.push(
      concat!(
        "macOS only. Requires a graphical session; ",
        "encrypted PDFs and built-in help cannot print."
      )
      .into(),
    );
  }
  lines
}

#[cfg(test)]
mod tests {
  use super::*;
  use vpdf_pdf::manual::Chapter;

  #[test]
  fn command_signatures_match_parser() {
    use vpdf_ui::commands::{COMMANDS, Resolution, View, resolve};
    for spec in COMMANDS {
      let syntax = command_syntax(spec);
      assert!(!syntax.ends_with(' '));
      let name = syntax.split_whitespace().next().unwrap();
      let name = name.trim_start_matches(':');
      if let Some((prefix, suffix)) = name.split_once('[') {
        let full = format!("{prefix}{}", suffix.trim_end_matches(']'));
        assert_eq!(full, spec.name);
        for end in prefix.len()..=full.len() {
          assert!(
            matches!(
              resolve(&full[..end], View::Document, |_| true, |_| false),
              Ok(Resolution::Command { spec: found, .. })
                if found.id == spec.id
            ),
            "{syntax}: {}",
            &full[..end]
          );
        }
      } else {
        assert_eq!(name, spec.name);
      }
    }
    for (name, expected) in [
      ("edit", ":e[dit] [file]"),
      ("quit", ":q[uit]  :q!  :x  :wq"),
      ("help!", ":help!  :h! [topic]"),
      ("split", ":s[plit] [file]"),
      ("tabprevious", ":tabp[revious]  :tabN"),
    ] {
      let spec = COMMANDS.iter().find(|spec| spec.name == name).unwrap();
      assert_eq!(command_syntax(spec), expected);
    }
  }

  #[test]
  fn related_guides_match_actions() -> Result<()> {
    let chapters = chapters()?;
    for (id, guide) in [
      ("url.open", "links"),
      ("panel.annotations", "links"),
      ("panel.figures", "regions"),
      ("panel.marks", "marks"),
      ("panel.jumps", "marks"),
      ("panel.results", "search"),
      ("panel.text", "zoom"),
      ("panel.recent", "state"),
    ] {
      let chapter = chapters.iter().find(|row| row.id == id).unwrap();
      assert_eq!(chapter.related, guide, "{id}");
    }
    Ok(())
  }

  #[test]
  fn builtins_have_no_dispatch() -> Result<()> {
    let chapters = chapters()?;
    assert!(chapters.iter().all(|chapter| !chapter.id.ends_with(".dispatch")));
    let keys: Vec<_> = chapters
      .iter()
      .filter(|row| row.section == Some(Section::Keys))
      .collect();
    for chapter in &keys {
      assert!(
        chapter.links.iter().all(|(_, target)| target.starts_with("action.")
          || target.starts_with("target.")),
        "{}",
        chapter.id
      );
    }
    let action = chapters.iter().find(|row| row.id == "half_down").unwrap();
    assert_eq!(action.section, Some(Section::Keys));
    assert!(action.lines.iter().any(|line| matches!(line,
      Line::Binding { keys, .. } if keys.contains("<C-d>"))));
    assert!(keys.iter().filter(|row| row.listed).count() < 250);
    Ok(())
  }
  #[test]
  fn shortcuts_merge_modes() -> Result<()> {
    use vpdf_ui::bindings::Target;
    use vpdf_ui::keys::Action;
    let chapters = chapters()?;
    let cancel = chapters
      .iter()
      .find(|row| row.id == Target::Reader(Action::Cancel).topic())
      .unwrap();
    assert_eq!(cancel.lines.len(), 1);
    let Line::Binding { keys, description } = &cancel.lines[0] else {
      panic!("Missing cancellation shortcut");
    };
    assert_eq!(keys, "<Esc>");
    for mode in ["normal", "cursor", "visual", "operator", "object"] {
      assert!(description.contains(mode), "{description}");
    }
    assert!(description.ends_with("modes."));
    for chapter in chapters
      .iter()
      .filter(|row| row.section == Some(Section::Keys) && row.listed)
    {
      for line in &chapter.lines {
        let Line::Binding { keys, .. } = line else { continue };
        assert_eq!(
          keys.split_whitespace().count(),
          1,
          "{}: {keys}",
          chapter.id
        );
      }
    }
    Ok(())
  }
  #[test]
  fn key_entries_precede_commands() -> Result<()> {
    let chapters = chapters()?;
    let commands = chapters
      .iter()
      .position(|row| row.section == Some(Section::Commands))
      .unwrap();
    assert!(
      chapters[commands..].iter().all(|row| row.section != Some(Section::Keys))
    );
    for key in ["zt", "zz", "zb"] {
      let at = chapters
        .iter()
        .position(|row| {
          row.lines.iter().any(
            |line| matches!(line, Line::Binding { keys, .. } if keys == key),
          )
        })
        .unwrap();
      assert!(at < commands, "{key}");
    }
    Ok(())
  }

  /// Labels repeat only across actions in disjoint modes.
  #[test]
  fn key_entries_are_unique_and_anchored() -> Result<()> {
    let chapters = chapters()?;
    let mut labels = BTreeMap::<&str, Vec<&Chapter>>::new();
    for chapter in
      chapters.iter().filter(|row| row.section == Some(Section::Keys))
    {
      let mut seen = Vec::new();
      for line in &chapter.lines {
        let Line::Binding { keys, .. } = line else { continue };
        assert!(!seen.contains(&keys), "{}: {keys}", chapter.id);
        seen.push(keys);
        labels.entry(keys).or_default().push(chapter);
      }
    }
    let ids = |label: &str| -> Vec<&str> {
      labels[label].iter().map(|chapter| chapter.id.as_str()).collect()
    };
    assert_eq!(ids("f"), ["hints.open"]);
    assert_eq!(ids("yf"), ["hints.copy"]);
    assert_eq!(
      ids("h"),
      ["motion.motion_left", "window.direction_left.operation_extend"]
    );
    for chapter in [labels["yf"][0], labels["h"][0]] {
      assert_eq!(chapter.lines.len(), 1, "{}", chapter.id);
    }
    for (label, chapters) in &labels {
      let suffix = format!(".{}", vpdf_ui::bindings::topic_text(label));
      let contexts: Vec<_> = chapters
        .iter()
        .flat_map(|chapter| chapter.anchors.values().flatten())
        .filter_map(|anchor| anchor.strip_suffix(&suffix))
        .collect();
      let mut unique = contexts.clone();
      unique.sort_unstable();
      unique.dedup();
      assert_eq!(unique.len(), contexts.len(), "{label}: {contexts:?}");
    }
    let snapshot = Bindings::default().snapshot();
    for row in snapshot.defaults.iter().filter(|row| !row.is_prefix()) {
      if family(&row.target).is_some() {
        continue;
      }
      let chapter = chapters
        .iter()
        .find(|chapter| chapter.id == row.target.topic())
        .unwrap();
      let anchor = key_anchor(row.context, &row.sequence);
      assert!(
        chapter.anchors.values().flatten().any(|name| *name == anchor),
        "{}: {anchor}",
        chapter.id
      );
    }
    Ok(())
  }

  #[test]
  fn continuations_follow_full_sequences() -> Result<()> {
    let chapters = chapters()?;
    let line = |id: &str| -> Vec<(String, String)> {
      chapters
        .iter()
        .find(|chapter| chapter.id == id)
        .unwrap()
        .lines
        .iter()
        .filter_map(|line| match line {
          Line::Binding { keys, description } => {
            Some((keys.clone(), description.clone()))
          }
          Line::Text(_) | Line::Code(_) | Line::Item(_) => None,
        })
        .collect()
    };
    let copy = line("hints.copy");
    assert_eq!(copy.len(), 1);
    assert_eq!(copy[0].0, "yf");
    assert!(copy[0].1.contains("normal"), "{}", copy[0].1);
    assert!(copy[0].1.ends_with("In operator mode, `f` completes it."));
    let word = line("object.around_false.object_word");
    assert!(word.iter().all(|(keys, _)| keys != "w"), "{word:?}");
    assert!(word.iter().any(|(keys, description)| keys == "iw"
      && description.contains("In object mode, `w` completes it.")));
    let left = line("motion.motion_left");
    assert_eq!(left.len(), 1);
    assert!(left[0].1.contains("left in normal"), "{}", left[0].1);
    assert!(
      left[0].1.contains("; close branch or select parent in panel mode")
    );
    let notes = chapters
      .iter()
      .flat_map(|chapter| &chapter.lines)
      .filter(|line| {
        matches!(line, Line::Binding { description, .. }
          if description.contains("completes it."))
      })
      .count();
    assert_eq!(notes, 35);
    let page = line("screen.amount_1");
    assert!(page.iter().any(|(keys, description)| keys == "<Space>"
      && description.contains("operator")));
    Ok(())
  }

  #[test]
  fn parameter_families_use_templates() -> Result<()> {
    let chapters = chapters()?;
    for (target, signature) in [
      (Target::Reader(Action::Mark('a')), "m{mark}"),
      (Target::Reader(Action::GoMark('a', false)), "'{mark}"),
      (Target::Reader(Action::GoMark('a', true)), "`{mark}"),
      (Target::Control(Control::Digit { digit: 1 }), "{count}"),
      (Target::Hint(HintAction::Type { character: 'z' }), "{label}"),
    ] {
      let id = target.topic();
      let chapter = chapters
        .iter()
        .find(|row| row.id == id || row.aliases.contains(&id))
        .unwrap();
      assert_eq!(chapter.lines.len(), 1);
      let line = &chapter.lines[0];
      assert!(matches!(line, Line::Binding { keys, .. } if keys == signature));
      if let Target::Reader(Action::Mark(_)) = target {
        for mark in ['a', 'z', 'A', 'Z'] {
          assert!(
            chapter
              .aliases
              .contains(&Target::Reader(Action::Mark(mark)).topic())
          );
        }
      }
    }
    Ok(())
  }
}
