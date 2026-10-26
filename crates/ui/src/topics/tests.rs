use super::*;
use crate::bindings::{Bindings, Change, Edit};
use crate::keys::{Action, Dispatch, Keys};
use crate::reference::Source;
use vpdf_reader::motion::Motion;

fn sequence(text: &str) -> Sequence {
  Sequence::parse(text).unwrap()
}

#[test]
fn command_topics_identify_targets() {
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[
      ("<Space>", None),
      ("<Space>r", Some(Target::Command("config reload".into()))),
      ("Q", Some(Target::Command("help sample".into()))),
      ("<F2>", Some(Target::Command("help sample".into()))),
    ],
  );
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  let mut ids = Vec::new();
  for (key, command) in [("Q", "help sample"), ("<Space>r", "config reload")] {
    let Lookup::Found(Destination::Action(id)) =
      registry.lookup(key, Context::Normal)
    else {
      panic!("Missing command topic: {key}");
    };
    let reference = registry.action(&id).unwrap();
    assert_eq!(reference.title, format!("Command: {command}"));
    assert!(
      reference
        .effective
        .iter()
        .all(|row| { row.target == Target::Command(command.into()) })
    );
    assert_eq!(
      registry.lookup(id.as_str(), Context::Normal),
      Lookup::Found(Destination::Action(id.clone()))
    );
    ids.push(id);
  }
  assert_ne!(ids[0], ids[1]);
  assert_eq!(
    registry.lookup("Q", Context::Normal),
    registry.lookup("<F2>", Context::Normal)
  );
}

fn remap(
  bindings: &mut Bindings,
  context: Context,
  edits: &[(&str, Option<Target>)],
) {
  let previous = bindings.map(context);
  let edits: Vec<_> = edits
    .iter()
    .map(|(key, target)| Edit {
      sequence: sequence(key),
      change: target.clone().map_or(Change::Unbind, Change::Bind),
    })
    .collect();
  let (map, diagnostics) = previous
    .apply_sourced(&previous, &edits, |_| Source::File("bindings.toml".into()));
  assert!(diagnostics.is_empty(), "{diagnostics:?}");
  bindings.set(context, map);
}

fn action(action: Action) -> Lookup {
  Lookup::Found(Destination::Action(
    TopicId::parse(&Target::Reader(action).topic()).unwrap(),
  ))
}

fn topic(id: &str, aliases: &[&str]) -> Topic {
  let id = TopicId::parse(id).unwrap();
  Topic {
    destination: Destination::Authored(id.clone()),
    id,
    aliases: aliases.iter().map(|s| (*s).into()).collect(),
    title: "Test".into(),
  }
}

#[test]
fn defaults_resolve_modes_and_sequences() {
  let bindings = Bindings::default();
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  for (key, context, expected) in [
    ("h", Context::Normal, Action::Motion(Motion::Left)),
    ("<C-d>", Context::Normal, Action::Scroll(0.5)),
    ("gg", Context::Normal, Action::Motion(Motion::First)),
    ("<Left>", Context::Normal, Action::Motion(Motion::Left)),
    ("<Enter>", Context::Normal, Action::Follow),
    ("<C-i>", Context::Normal, Action::History(true)),
    ("<Tab>", Context::Normal, Action::LocalFocus),
    ("0", Context::Normal, Action::Motion(Motion::LineStart)),
    ("yy", Context::Panel, Action::YankLine),
    (
      "yiw",
      Context::Cursor,
      Action::Object(vpdf_reader::objects::Object::Word, false),
    ),
    ("ma", Context::Normal, Action::Mark('a')),
  ] {
    assert_eq!(registry.lookup(key, context), action(expected), "{key}");
  }
  assert_eq!(
    registry.lookup("half_down", Context::Normal),
    action(Action::Scroll(0.5))
  );
  for key in ["g", "iw", "x", "<A-z>"] {
    assert_eq!(registry.lookup(key, Context::Normal), Lookup::Missing, "{key}");
  }
  assert_ne!(
    registry.lookup("h", Context::Normal),
    registry.lookup("H", Context::Normal)
  );
}

#[test]
fn remaps_override_aliases_and_keep_unbound_actions() {
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[
      ("h", Some(Target::Reader(Action::Scroll(0.5)))),
      ("d", None),
      ("<C-d>", None),
    ],
  );
  remap(
    &mut bindings,
    Context::Cursor,
    &[("h", Some(Target::Reader(Action::Motion(Motion::Right))))],
  );
  let registry =
    Registry::new(vec![topic("history", &["h", "d"])], bindings.snapshot())
      .unwrap();
  assert_eq!(
    registry.lookup("h", Context::Normal),
    action(Action::Scroll(0.5))
  );
  assert_eq!(
    registry.lookup("h", Context::Cursor),
    action(Action::Motion(Motion::Right))
  );
  for key in ["d", "<C-d>"] {
    assert_eq!(registry.lookup(key, Context::Normal), Lookup::Missing);
    assert!(
      !registry
        .complete(key, Context::Normal)
        .iter()
        .any(|candidate| candidate.name == key)
    );
  }
  let reference =
    registry.action(&TopicId::parse("half_down").unwrap()).unwrap();
  assert!(reference.defaults.iter().any(|row| row.sequence == sequence("d")));
  let effective: Vec<_> = reference
    .effective
    .iter()
    .filter(|row| row.context == Context::Normal)
    .collect();
  let h = effective.iter().find(|row| row.sequence == sequence("h")).unwrap();
  assert!(
    effective
      .iter()
      .all(|row| ![sequence("d"), sequence("<C-d>")].contains(&row.sequence))
  );
  assert_eq!(h.sources, [Source::File("bindings.toml".into())]);
  assert!(registry.action(&TopicId::parse("panel.text").unwrap()).is_some());
}

#[test]
fn stable_actions_survive_without_bindings() {
  let mut bindings = Bindings::default();
  for &context in Context::ALL {
    let edits: Vec<_> = bindings
      .map(context)
      .entries()
      .filter(|(_, target)| **target == Target::Reader(Action::Scroll(0.5)))
      .map(|(sequence, _)| (sequence.to_string(), None))
      .collect();
    let edits: Vec<_> = edits
      .iter()
      .map(|(key, target)| (key.as_str(), target.clone()))
      .collect();
    remap(&mut bindings, context, &edits);
  }
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  let reference =
    registry.action(&TopicId::parse("half_down").unwrap()).unwrap();
  assert_eq!(reference.effective, Vec::<Binding>::new());
  assert_ne!(reference.defaults, Vec::<Binding>::new());
  assert_eq!(
    registry.lookup("half_down", Context::Normal),
    action(Action::Scroll(0.5))
  );
  assert!(
    registry
      .complete("half", Context::Normal)
      .iter()
      .any(|candidate| candidate.name == "half_down")
  );
}

#[test]
fn names_and_completion_use_literal_prefixes() {
  let registry = Registry::new(
    vec![
      topic("history", &["hi"]),
      topic("histogram", &[]),
      topic("helpful", &[]),
      topic("homecoming", &[]),
    ],
    Bindings::default().snapshot(),
  )
  .unwrap();
  assert!(matches!(
    registry.lookup("hist", Context::Normal),
    Lookup::Ambiguous(_)
  ));
  assert_eq!(
    registry.lookup("history", Context::Normal),
    Lookup::Found(Destination::Authored(TopicId::parse("history").unwrap()))
  );
  assert_eq!(
    registry.lookup("homeco", Context::Normal),
    Lookup::Found(Destination::Authored(TopicId::parse("homecoming").unwrap()))
  );
  assert_eq!(
    registry.lookup("hi", Context::Normal),
    registry.lookup("history", Context::Normal)
  );
  assert_eq!(
    registry.lookup("unknown_topic", Context::Normal),
    Lookup::Missing
  );
  assert_eq!(
    registry.lookup("unknown_topic", Context::Normal).message(),
    Some("No help topic")
  );
  let candidates = registry.complete("h", Context::Normal);
  for name in ["h", "half_down", "half_up", "helpful", "history"] {
    assert!(candidates.iter().any(|c| c.name == name), "{name}");
  }
  assert!(candidates.iter().find(|c| c.name == "h").unwrap().action.is_some());
  assert!(
    registry
      .complete("H", Context::Normal)
      .iter()
      .all(|candidate| candidate.name.starts_with('H'))
  );
}

#[test]
fn removed_sequences_do_not_use_stale_aliases() {
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[("xy", Some(Target::Reader(Action::Scroll(0.5)))), ("gg", None)],
  );
  let previous = bindings.clone();
  bindings = Bindings::default();
  bindings.remember(&previous);
  remap(&mut bindings, Context::Normal, &[("gg", None)]);
  let registry = Registry::new(
    vec![
      topic("old_first", &["gg"]),
      topic("old_custom", &["xy"]),
      topic("xy_details", &[]),
    ],
    bindings.snapshot(),
  )
  .unwrap();
  for key in ["gg", "xy"] {
    assert_eq!(registry.lookup(key, Context::Normal), Lookup::Missing);
    assert!(
      !registry
        .complete(key, Context::Normal)
        .iter()
        .any(|candidate| candidate.name == key)
    );
  }
  assert!(
    registry
      .complete("xy", Context::Normal)
      .iter()
      .any(|candidate| candidate.name == "xy_details")
  );
}

#[test]
fn registry_rejects_invalid_and_duplicate_ids() {
  for name in ["h", "", "<C-d>", "bad name", "Bad"] {
    assert!(TopicId::parse(name).is_err(), "{name}");
  }
  for topics in [
    vec![topic("same", &[]), topic("same", &[])],
    vec![topic("first", &["other"]), topic("other", &[])],
    vec![topic("config", &[])],
  ] {
    assert!(Registry::new(topics, Snapshot::default()).is_err());
  }
}

#[test]
fn snapshot_actions_match_dispatch() {
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[("xy", Some(Target::Reader(Action::Scroll(0.5))))],
  );
  remap(
    &mut bindings,
    Context::Operator,
    &[(
      "t",
      Some(Target::Control(crate::bindings::Control::Object { around: true })),
    )],
  );
  for row in bindings.snapshot().effective {
    if !matches!(row.target, Target::Reader(_) | Target::Command(_)) {
      continue;
    }
    let mut keys = Keys::default();
    let mut last = None;
    for &key in row.sequence.keys() {
      last = keys.feed_with(key, row.context, &bindings);
    }
    let target = match last {
      Some(Dispatch::Input(input)) => Target::Reader(input.action),
      Some(Dispatch::Command(command)) => Target::Command(command),
      None => panic!("{}: {}", row.context.name(), row.sequence),
    };
    assert_eq!(target, row.target, "{}: {}", row.context.name(), row.sequence);
  }
}

#[test]
fn renderer_inputs_are_thread_safe() {
  fn send_sync<T: Send + Sync>() {}
  send_sync::<Reference>();
  send_sync::<Registry>();
}

#[test]
fn inactive_aliases_do_not_affect_named_prefixes() {
  let mut bindings = Bindings::default();
  remap(&mut bindings, Context::Normal, &[("xyz", None)]);
  let registry = Registry::new(
    vec![
      topic("former", &["xyz"]),
      topic("xy_details", &[]),
      topic("unique", &["uniq_a", "uniq_z"]),
    ],
    bindings.snapshot().in_panel(crate::keys::PanelKind::Annotations),
  )
  .unwrap();
  assert_eq!(
    registry.lookup("xy", Context::Normal),
    Lookup::Found(Destination::Authored(TopicId::parse("xy_details").unwrap()))
  );
  assert_eq!(
    registry.lookup("uniq", Context::Normal),
    Lookup::Found(Destination::Authored(TopicId::parse("unique").unwrap()))
  );
  assert_eq!(registry.lookup("h", Context::Panel), Lookup::Missing);
  assert_eq!(
    registry.lookup("<C-S-d>", Context::Normal),
    action(Action::Scroll(0.5))
  );
}

#[test]
fn composed_binding_history_is_recognized() {
  let mut previous = Bindings::default();
  remap(
    &mut previous,
    Context::Operator,
    &[(
      "t",
      Some(Target::Control(crate::bindings::Control::Object { around: true })),
    )],
  );
  let mut current = Bindings::default();
  current.remember(&previous);
  let registry =
    Registry::new(vec![topic("former", &["ytw"])], current.snapshot()).unwrap();
  assert_eq!(registry.lookup("ytw", Context::Normal), Lookup::Missing);
}

#[test]
fn prefixes_keep_reference_data_without_key_topics() {
  let registry =
    Registry::new(authored(), Bindings::default().snapshot()).unwrap();
  let id = TopicId::parse("register.choose").unwrap();
  let reference = registry.action(&id).unwrap();
  assert!(reference.effective.iter().any(|row| row.context == Context::Normal
    && row.sequence.to_string() == "\""
    && row.is_prefix()));
  assert_eq!(
    registry.lookup("register.choose", Context::Normal),
    Lookup::Found(Destination::Action(id))
  );
  for key in ["\"", "1"] {
    assert_eq!(
      registry.lookup(key, Context::Normal),
      Lookup::Found(Destination::Authored(TopicId::parse("keys").unwrap()))
    );
  }
  assert!(
    !registry
      .complete("\"", Context::Normal)
      .iter()
      .any(|candidate| candidate.name == "\"")
  );
}

#[test]
fn canonical_topics_survive_key_collisions() {
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[("h", None), ("half_down", Some(Target::Reader(Action::Quit)))],
  );
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  let expected = action(Action::Scroll(0.5));
  assert_eq!(registry.lookup("half_down", Context::Normal), expected);
  assert_eq!(
    registry.lookup("<h>alf_down", Context::Normal),
    action(Action::Quit)
  );
  let candidates = registry.complete("half", Context::Normal);
  let candidate = candidates.iter().find(|c| c.name == "half_down").unwrap();
  assert_eq!(Lookup::Found(candidate.destination.clone()), expected);
}

#[test]
fn panel_references_describe_effects_and_filter_ignored_keys() {
  use crate::keys::Input;
  use crate::panel::{Event, Panel, Row};
  let bindings = Bindings::default();
  let registry = Registry::new(
    authored(),
    bindings.snapshot().in_panel(crate::keys::PanelKind::Outline),
  )
  .unwrap();
  for (key, action, purpose, event) in [
    ("q", Action::Quit, "close panel", Event::Close),
    (
      "o",
      Action::Swap,
      "follow without focusing content",
      Event::Follow { focus: false },
    ),
  ] {
    assert_eq!(registry.lookup(key, Context::Panel), self::action(action));
    let id = TopicId::parse(&Target::Reader(action).topic()).unwrap();
    let reference = registry.action(&id).unwrap();
    assert!(
      reference.effective.iter().any(|row| row.context == Context::Panel
        && row.sequence == sequence(key)
        && row.purpose == purpose)
    );
    let mut panel = Panel::new(
      crate::keys::PanelKind::Outline,
      vec![Row::new(
        "section".into(),
        String::new(),
        crate::panel::Destination::Position(Position::default()),
      )],
    );
    assert_eq!(panel.input(Input::plain(action), 24), event);
  }
  let reference =
    registry.action(&TopicId::parse("panel.outline").unwrap()).unwrap();
  assert!(reference.effective.iter().any(|row| row.context == Context::Panel
    && row.sequence == sequence("go")
    && row.purpose == "close panel"));
  let down = registry
    .action(
      &TopicId::parse(&Target::Reader(Action::Motion(Motion::Down)).topic())
        .unwrap(),
    )
    .unwrap();
  assert!(down.effective.iter().any(|row| row.context == Context::Panel
    && row.sequence == sequence("j")
    && row.purpose == "select next row; keep panel focus"));
  let recent = Registry::new(
    authored(),
    bindings.snapshot().in_panel(crate::keys::PanelKind::Recent),
  )
  .unwrap();
  assert_eq!(recent.lookup("o", Context::Panel), Lookup::Missing);
  assert!(
    !recent
      .complete("o", Context::Panel)
      .iter()
      .any(|candidate| candidate.name == "o")
  );
  for registry in [&registry, &recent] {
    assert_eq!(
      registry.lookup("<Esc>", Context::Panel),
      action(Action::Cancel)
    );
    for key in ["<Tab>", "<S-Tab>"] {
      assert_eq!(
        registry.lookup(key, Context::Panel),
        action(Action::LocalFocus)
      );
    }
  }
}

#[test]
fn help_modes_and_notation() {
  let registry =
    Registry::new(authored(), Bindings::default().snapshot()).unwrap();
  assert_eq!(
    registry.lookup("CTRL-D", Context::Normal),
    registry.lookup("<C-d>", Context::Normal)
  );
  assert_eq!(
    registry.lookup("search:<Tab>", Context::Normal),
    registry.lookup("<Tab>", Context::Search)
  );
  assert_ne!(
    registry.lookup("search:<Tab>", Context::Normal),
    registry.lookup("<Tab>", Context::Normal)
  );
  assert_eq!(
    registry.lookup("m", Context::Normal),
    Lookup::Found(Destination::Authored(TopicId::parse("keys").unwrap()))
  );
  assert!(
    registry
      .complete("search:<", Context::Normal)
      .iter()
      .all(
        |row| row.name.starts_with("search:<") && !row.description.is_empty()
      )
  );
  let candidates = registry.complete("", Context::Normal);
  assert!(matches!(candidates[0].destination, Destination::Authored(_)));
  assert_eq!(registry.complete("h", Context::Normal)[0].name, "h");
  assert!(candidates.iter().all(|row| !row.description.starts_with('{')));
  for query in ["search", "zoom", "count"] {
    assert!(matches!(
      registry.lookup(query, Context::Normal),
      Lookup::Found(Destination::Authored(_))
    ));
  }
}

#[test]
fn control_shorthand_resolves() {
  let registry =
    Registry::new(authored(), Bindings::default().snapshot()).unwrap();
  let expected = registry.lookup("<C-d>", Context::Normal);
  for query in ["c-d", "C-d", "CTRL-D", "ctrl-d"] {
    assert_eq!(registry.lookup(query, Context::Normal), expected, "{query}");
    assert!(
      registry.complete(query, Context::Normal).iter().any(|row| {
        row.name == query && Lookup::Found(row.destination.clone()) == expected
      }),
      "{query}"
    );
  }
  let candidates = registry.complete("c-", Context::Normal);
  assert!(candidates.iter().any(|row| row.name == "c-d"));
  assert!(candidates.iter().all(|row| row.name.starts_with("c-")));
  assert_eq!(
    registry.lookup("search:c-u", Context::Normal),
    registry.lookup("search:<C-u>", Context::Normal)
  );
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[("c-d", Some(Target::Reader(Action::Motion(Motion::Left))))],
  );
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  assert_eq!(
    registry.lookup("c-d", Context::Normal),
    action(Action::Motion(Motion::Left))
  );
  assert_eq!(registry.complete("c-d", Context::Normal)[0].name, "c-d");
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[("c-dx", Some(Target::Reader(Action::Motion(Motion::Left))))],
  );
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  let candidates = registry.complete("c-d", Context::Normal);
  assert_eq!(candidates[0].name, "c-d");
  assert!(candidates.iter().any(|row| row.name == "c-dx"));
}

#[test]
fn command_topics_include_aliases() -> crate::Result<()> {
  let registry = Registry::new(authored(), Bindings::default().snapshot())?;
  for spec in crate::commands::COMMANDS {
    let destination = Destination::Command(TopicId::parse(&spec.topic())?);
    for name in std::iter::once(&spec.name).chain(spec.aliases.iter()) {
      let query = format!(":{name}");
      for context in [Context::Home, Context::Normal, Context::Panel] {
        assert_eq!(
          registry.lookup(&query, context),
          Lookup::Found(destination.clone()),
          "{query}"
        );
        assert!(
          registry
            .complete(&query, context)
            .iter()
            .any(|row| row.name == query && row.destination == destination)
        );
      }
    }
  }
  for name in ["split", "vsplit", "perf", "theme", "reload", "password"] {
    assert_eq!(
      registry.lookup(name, Context::Normal),
      registry.lookup(&format!(":{name}"), Context::Normal)
    );
  }
  assert!(matches!(
    registry.lookup("ocr", Context::Normal),
    Lookup::Found(Destination::Authored(_))
  ));
  Ok(())
}

#[test]
fn browsing_collapses_dispatch_aliases() {
  let registry =
    Registry::new(authored(), Bindings::default().snapshot()).unwrap();
  let candidates = registry.complete("C-", Context::Normal);
  assert!(candidates.iter().any(|row| row.name == "C-d"));
  assert!(!candidates.iter().any(|row| row.name == "C-D"));
  assert!(!candidates.iter().any(|row| row.name.contains(['<', '>'])));
  assert!(!candidates.iter().any(|row| row.name.contains("Esc")));
  let alias = "<C-A-S-Esc>";
  assert_eq!(registry.lookup(alias, Context::Normal), action(Action::Cancel));
  assert!(
    registry
      .complete(alias, Context::Normal)
      .iter()
      .any(|row| row.name == alias)
  );
  let guides = registry.complete("se", Context::Normal);
  assert!(guides.iter().any(|row| row.name == "search"));
  assert!(!guides.iter().any(|row| row.name == "searching"));
  assert_eq!(
    registry.lookup("searching", Context::Normal),
    registry.lookup("search", Context::Normal)
  );
  let mut bindings = Bindings::default();
  remap(
    &mut bindings,
    Context::Normal,
    &[(alias, Some(Target::Reader(Action::Cancel)))],
  );
  let registry = Registry::new(authored(), bindings.snapshot()).unwrap();
  assert!(
    registry
      .complete("<C-", Context::Normal)
      .iter()
      .any(|row| row.name == alias)
  );
}

#[test]
fn lookup_recovers_spelling_and_quotes() {
  let registry =
    Registry::new(authored(), Bindings::default().snapshot()).unwrap();
  assert_eq!(
    registry.lookup("'reading.zoom'", Context::Normal),
    registry.lookup("reading.zoom", Context::Normal)
  );
  assert_eq!(
    registry.lookup("helphelp", Context::Normal),
    registry.lookup("introduction", Context::Normal)
  );
  assert_eq!(
    registry.lookup("quickref", Context::Normal),
    registry.lookup("keys", Context::Normal)
  );
  for query in ["serach", "searh", "searrch"] {
    assert_eq!(registry.suggest(query, Context::Normal)[0], "search");
  }
  assert_eq!(
    registry.suggest("unrecognized-topic", Context::Normal),
    [] as [String; 0]
  );
}

#[test]
fn contents_folds_use_custom_bindings() {
  use crate::keys::{Fold, Input, Keys, PanelKind};
  use crate::panel::{Event, Panel};
  let mut bindings = Bindings::default();
  let fold = Action::Fold(Fold::CollapseAll);
  remap(&mut bindings, Context::Panel, &[("x", Some(Target::Reader(fold)))]);
  let registry =
    Registry::new(authored(), bindings.snapshot().in_panel(PanelKind::Outline))
      .unwrap();
  assert_eq!(registry.lookup("x", Context::Panel), action(fold));
  let topic =
    registry.action(&TopicId::parse("outline.collapse-all").unwrap()).unwrap();
  assert!(topic.effective.iter().any(|row| row.context == Context::Panel
    && row.sequence.to_string() == "x"
    && row.purpose == "collapse all branches"));
  let mut keys = Keys::default();
  let inputs: Vec<_> = sequence("3x")
    .keys()
    .iter()
    .filter_map(|&key| match keys.feed_with(key, Context::Panel, &bindings) {
      Some(Dispatch::Input(input)) => Some(input),
      _ => None,
    })
    .collect();
  assert_eq!(inputs.len(), 1);
  let mut panel = Panel::new(PanelKind::Outline, vec![]);
  assert_eq!(
    panel.input(inputs[0], 20),
    Event::Fold { operation: Fold::CollapseAll, count: 3 }
  );
  assert_eq!(
    panel.input(Input::plain(Action::Fold(Fold::Open)), 20),
    Event::Fold { operation: Fold::Open, count: 1 }
  );
  for kind in [PanelKind::Annotations, PanelKind::Recent] {
    let snapshot = bindings.snapshot().in_panel(kind);
    assert!(
      !snapshot.effective.iter().any(|row| row.context == Context::Panel
        && matches!(row.target, Target::Reader(Action::Fold(_))))
    );
  }
}
