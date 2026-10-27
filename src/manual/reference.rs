use crate::Result;
use std::collections::BTreeMap;
use vpdf_model::input::{KeyCode, KeyModifiers};
use vpdf_pdf::manual::{Chapter, Line, Section};
use vpdf_ui::bindings::Binding;
use vpdf_ui::bindings::{Bindings, Control, HintAction, Target};
use vpdf_ui::keys::Action;
use vpdf_ui::topics::{Destination, Registry, authored};

fn bindings(rows: &[&Binding], defaults: &Bindings) -> Vec<Line> {
  let mut contexts = BTreeMap::<_, Vec<&Binding>>::new();
  for row in rows {
    contexts.entry((row.context, row.purpose.clone())).or_default().push(row);
  }
  let mut groups = BTreeMap::<_, Vec<&str>>::new();
  for ((context, purpose), rows) in contexts {
    let preferred = defaults.shortcut(context, &rows[0].target);
    let row = rows
      .iter()
      .min_by_key(|row| {
        let sequence = row.sequence.to_string();
        let uppercase = row.sequence.keys().iter().filter(|key| {
        key.modifiers.contains(KeyModifiers::CONTROL)
          && matches!(key.code, KeyCode::Char(ch) if ch.is_ascii_uppercase())
      }).count();
        (
          sequence != preferred,
          row.sequence.keys().len(),
          uppercase,
          sequence.len(),
          sequence,
        )
      })
      .unwrap();
    groups
      .entry((row.sequence.to_string(), purpose))
      .or_default()
      .push(context.name());
  }
  groups
    .into_iter()
    .map(|((keys, purpose), modes)| Line::Binding {
      keys,
      description: format!(
        "{purpose} in {} {}.",
        modes.join(", "),
        if modes.len() == 1 { "mode" } else { "modes" }
      ),
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
  let rows = grouped(&snapshot.defaults);
  let mut entries = BTreeMap::<String, Chapter>::new();
  for topic in registry.topics() {
    let Destination::Action(original) = &topic.destination else { continue };
    let rows = rows.get(original.as_str()).map_or(&[][..], Vec::as_slice);
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
      let mut lines = bindings(rows, &defaults);
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
        !original.parameterized() && !rows.is_empty(),
      )
    };
    let chapter = entries.entry(id.clone()).or_insert_with(|| {
      let guide = teaching(&id);
      Chapter {
        id: id.clone(),
        title,
        lines,
        related: guide.into(),
        related_title: guides[guide].clone(),
        anchors: BTreeMap::new(),
        section: Some(Section::Keys),
        listed,
        aliases: Vec::new(),
        links: Vec::new(),
      }
    });
    if original.as_str() != id {
      chapter.aliases.push(original.as_str().into());
    }
  }
  let mut chapters: Vec<_> = entries.into_values().collect();
  chapters.push(configuration());
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
    Id::Split
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
    Id::Annotations => "links",
    Id::Toc | Id::Overview => "navigation",
    Id::CursorLine | Id::Crop | Id::Text => "zoom",
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
  let example = match spec.id {
    Id::Config => ":config reload",
    Id::Theme => ":theme dark",
    Id::Edit => ":edit book.pdf",
    Id::Open => ":open",
    Id::TabEdit => ":tabedit notes.pdf",
    Id::TabNew => ":tabnew",
    Id::Split => ":split",
    Id::VSplit => ":vsplit",
    Id::TabNext => ":tabnext 2",
    Id::Ocr => ":ocr eng+vie",
    Id::Help => ":help :split",
    Id::HelpBeside => ":help! search",
    _ => "",
  };
  if !example.is_empty() {
    lines.push(format!("For example: {example}").into());
  }
  match spec.id {
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
    Id::Split | Id::VSplit => lines.push(
      concat!(
        "No file: duplicate this view. ",
        "An already open file focuses its existing pane."
      )
      .into(),
    ),
    Id::CursorLine | Id::Crop | Id::Mouse | Id::Perf => lines.push(
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

fn configuration() -> Chapter {
  let config = crate::config::Settings::default().reference();
  let mut lines = Vec::new();
  let mut anchors = BTreeMap::new();
  for setting in config.settings {
    anchors.insert(lines.len(), setting.name.clone());
    lines.push(Line::Binding {
      keys: setting.name,
      description: format!("Default: {}", setting.default),
    });
  }
  Chapter {
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
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
      ("quit", ":q[uit]"),
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
    assert!(
      chapters.iter().all(|chapter| !chapter.id.ends_with(".dispatch")
        && chapter.links.is_empty())
    );
    let action = chapters.iter().find(|row| row.id == "half_down").unwrap();
    assert_eq!(action.section, Some(Section::Keys));
    assert!(action.lines.iter().any(|line| matches!(line,
      Line::Binding { keys, .. } if keys.contains("<C-d>"))));
    assert!(chapters.iter().filter(|row| row.listed).count() < 150);
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
