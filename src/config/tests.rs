use super::*;
use clap::Parser;

fn decoded(text: &str) -> Loaded {
  parse(Path::new("settings.toml"), text)
}

#[test]
fn example_matches_defaults() {
  let loaded = decoded(&Settings::example().unwrap());
  assert_eq!(loaded.settings, Settings::default());
  assert!(loaded.warning.is_none());
  assert_eq!(
    Settings::example().unwrap(),
    concat!(
      "[appearance]\ntheme = \"default\"\nascii_glyphs = false\n\n",
      "[reading]\ncursorline = true\nzoom = 1.0\n",
      "inactive_brightness = 0.8\n\n",
      "[ocr]\nlanguage = \"eng\"\n\n[keybindings]\n"
    )
  );
}

#[test]
fn ascii_glyphs_read_a_boolean() {
  for (value, ascii, warned) in
    [("true", true, false), ("false", false, false), ("'yes'", false, true)]
  {
    let loaded = decoded(&format!("[appearance]\nascii_glyphs = {value}"));
    assert_eq!(loaded.settings.ascii_glyphs(), ascii, "{value}");
    assert_eq!(loaded.warning.is_some(), warned, "{value}");
  }
}

#[test]
fn inactive_brightness_validates_range() {
  assert_eq!(Settings::default().inactive_brightness(), 0.8);
  for (value, expected, valid) in [
    ("0", 0.0, true),
    ("0.9", 0.9, true),
    ("1", 1.0, true),
    ("-0.1", 0.8, false),
    ("1.1", 0.8, false),
    ("nan", 0.8, false),
    ("inf", 0.8, false),
    ("-inf", 0.8, false),
    ("true", 0.8, false),
    ("'0.9'", 0.8, false),
  ] {
    let loaded = decoded(&format!(
      "[reading]\ncursorline = false\ninactive_brightness = {value}"
    ));
    assert_eq!(loaded.settings.inactive_brightness(), expected, "{value}");
    assert!(!loaded.settings.cursorline());
    assert_eq!(loaded.warning.is_none(), valid, "{value}");
    if let Some(warning) = loaded.warning {
      assert!(warning.contains("reading.inactive_brightness"));
    }
  }
}

#[test]
fn fields_decode_independently() {
  for (source, cursorline, zoom, language, warning) in [
    ("", true, 1.0, "eng", None),
    (
      "[reading]\ncursorline = false\nzoom = 2\n[ocr]\nlanguage = 'eng+vie'",
      false,
      2.0,
      "eng+vie",
      None,
    ),
    (
      "[reading]\ncursorline = 'false'\nzoom = 0.1",
      true,
      0.1,
      "eng",
      Some("reading.cursorline"),
    ),
    (
      "[reading]\ncursorline = false\nzoom = '2'",
      false,
      1.0,
      "eng",
      Some("reading.zoom"),
    ),
    (
      "[reading]\nzoom = 8.0\nextra = { nested = 1 }",
      true,
      8.0,
      "eng",
      Some("reading.extra"),
    ),
    (
      "reading = false\n[ocr]\nlanguage = 'vie'",
      true,
      1.0,
      "vie",
      Some("reading"),
    ),
    ("ocr = []\n[reading]\ncursorline = false", false, 1.0, "eng", Some("ocr")),
    (
      "[unknown]\na = 1\n[reading]\nzoom = 3.0",
      true,
      3.0,
      "eng",
      Some("unknown"),
    ),
    (
      "[reading]\nzoom = 2\n[ocr]\nlanguage = 1",
      true,
      2.0,
      "eng",
      Some("ocr.language"),
    ),
  ] {
    let loaded = decoded(source);
    assert_eq!(loaded.settings.cursorline(), cursorline, "{source}");
    assert_eq!(loaded.settings.zoom(), zoom, "{source}");
    assert_eq!(loaded.settings.language(), language, "{source}");
    match warning {
      Some(key) => {
        let message = loaded.warning.expect(source);
        assert!(message.contains("settings.toml"), "{message}");
        assert!(message.contains(key), "{message}");
      }
      None => assert!(loaded.warning.is_none(), "{source}"),
    }
  }
}

#[test]
fn invalid_values_keep_defaults() {
  for zoom in ["nan", "inf", "-inf", "0.099", "8.01", "-1", "true", "[]"] {
    let loaded =
      decoded(&format!("[reading]\ncursorline = false\nzoom = {zoom}"));
    assert!(!loaded.settings.cursorline());
    assert_eq!(loaded.settings.zoom(), 1.0, "{zoom}");
    assert!(loaded.warning.unwrap().contains("reading.zoom"));
  }
  for language in
    ["", "+eng", "eng+", "eng++vie", " ", "eng+ ", "eng\n", "\u{1b}"]
  {
    let text = format!(
      "[reading]\nzoom = 2\n[ocr]\nlanguage = {}",
      toml::Value::String(language.into())
    );
    let loaded = decoded(&text);
    assert_eq!(loaded.settings.zoom(), 2.0);
    assert_eq!(loaded.settings.language(), "eng");
    assert!(loaded.warning.unwrap().contains("ocr.language"));
  }
}

#[test]
fn syntax_errors_discard_document() {
  for text in
    ["[reading]\nzoom = 2\nzoom = 3", "[reading]\ncursorline = false\n[broken"]
  {
    let loaded = decoded(text);
    assert_eq!(loaded.settings, Settings::default());
    let warning = loaded.warning.unwrap();
    assert!(warning.contains("line"), "{warning}");
    assert!(warning.contains("column"), "{warning}");
  }
}

#[test]
fn diagnostics_are_printable() {
  let loaded =
    parse(Path::new("bad\x1b\x07.toml"), "[reading]\n'evil\u{1b}' = true");
  let warning = loaded.warning.unwrap();
  assert!(!warning.chars().any(char::is_control), "{warning:?}");
  assert!(warning.contains("bad"));
}

#[test]
fn discovery_uses_absolute_bases() {
  for (xdg, home, expected) in [
    (None, None, None),
    (Some(""), None, None),
    (Some("relative"), Some("relative"), None),
    (None, Some("/home/user"), Some("/home/user/.config/vpdf/config.toml")),
    (Some(""), Some("/home/user"), Some("/home/user/.config/vpdf/config.toml")),
    (
      Some("relative"),
      Some("/home/user"),
      Some("/home/user/.config/vpdf/config.toml"),
    ),
    (Some("/config"), None, Some("/config/vpdf/config.toml")),
    (Some("/config"), Some("/home/user"), Some("/config/vpdf/config.toml")),
  ] {
    assert_eq!(
      discover(xdg.map(OsStr::new), home.map(OsStr::new)),
      expected.map(PathBuf::from)
    );
  }
}

#[test]
fn file_errors_and_selection() -> crate::Result<()> {
  let dir = tempfile::tempdir()?;
  let path = dir.path().join("config.toml");
  assert!(load(&path, false).warning.is_none());
  assert!(load(&path, true).warning.unwrap().contains("config.toml"));
  std::fs::write(&path, [0xff])?;
  assert_eq!(load(&path, true).settings, Settings::default());
  assert!(load(&path, true).warning.is_some());
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  let options = Options { config: Some(path), ..Options::default() };
  assert_eq!(options.load_from(None, None).settings.zoom(), 2.0);
  let options = Options { no_config: true, ..options };
  assert_eq!(options.load_from(None, None).settings, Settings::default());
  assert!(load(dir.path(), true).warning.is_some());
  Ok(())
}

#[test]
fn selection_flags_conflict() {
  #[derive(Parser)]
  struct Cli {
    #[command(flatten)]
    options: Options,
  }
  assert!(
    Cli::try_parse_from(["vpdf", "--config", "a", "--no-config"]).is_err()
  );
  for flag in ["--config-help", "--config-licenses"] {
    let error = Cli::try_parse_from(["vpdf", flag]).err().unwrap();
    assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
  }
  let options =
    Cli::try_parse_from(["vpdf", "--config", "relative.toml"]).unwrap().options;
  assert_eq!(options.config, Some(PathBuf::from("relative.toml")));
}

#[test]
fn discovery_loads_one_file() -> crate::Result<()> {
  let root = tempfile::tempdir()?;
  let home = root.path().join("home");
  let xdg = root.path().join("xdg");
  std::fs::create_dir_all(home.join(".config/vpdf"))?;
  std::fs::create_dir_all(xdg.join("vpdf"))?;
  std::fs::write(home.join(".config/vpdf/config.toml"), "[reading]\nzoom = 2")?;
  std::fs::write(xdg.join("vpdf/config.toml"), "[reading]\nzoom = 3")?;
  let options = Options::default();
  assert_eq!(
    options
      .load_from(Some(xdg.as_os_str()), Some(home.as_os_str()))
      .settings
      .zoom(),
    3.0
  );
  assert_eq!(
    options
      .load_from(Some(OsStr::new("relative")), Some(home.as_os_str()))
      .settings
      .zoom(),
    2.0
  );
  std::fs::remove_file(xdg.join("vpdf/config.toml"))?;
  assert_eq!(
    options
      .load_from(Some(xdg.as_os_str()), Some(home.as_os_str()))
      .settings
      .zoom(),
    1.0
  );
  let options = Options { no_config: true, ..options };
  assert_eq!(
    options.load_from(None, Some(home.as_os_str())).settings,
    Settings::default()
  );
  Ok(())
}

#[test]
#[cfg(unix)]
fn unreadable_file_warns() -> crate::Result<()> {
  use std::os::unix::fs::PermissionsExt;
  let root = tempfile::tempdir()?;
  let path = root.path().join("config.toml");
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0))?;
  let loaded = load(&path, true);
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
  assert_eq!(loaded.settings, Settings::default());
  assert!(loaded.warning.unwrap().contains("config.toml"));
  Ok(())
}

#[test]
fn explicit_relative_file() -> crate::Result<()> {
  let root = tempfile::tempdir_in(".")?;
  let path = Path::new(root.path().file_name().unwrap()).join("config.toml");
  assert!(path.is_relative());
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  let options = Options { config: Some(path), ..Default::default() };
  assert_eq!(
    options.load_from(Some(OsStr::new("/ignored")), None).settings.zoom(),
    2.0
  );
  Ok(())
}

#[test]
fn reload_retains_invalid_and_resets_omitted() -> crate::Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("config.toml");
  std::fs::write(
    &path,
    concat!(
      "[reading]\nzoom = 2\ncursorline = false\n",
      "inactive_brightness = 0.3\n[ocr]\nlanguage = 'vie'",
    ),
  )?;
  let loaded = load(&path, true);
  std::fs::write(
    &path,
    concat!(
      "[reading]\nzoom = 'bad'\ninactive_brightness = 0.5\n",
      "[ocr]\nlanguage = 7\nunknown = true",
    ),
  )?;
  let reloaded = loaded.source.reload(&loaded.settings);
  assert_eq!(reloaded.settings.zoom(), 2.0);
  assert!(reloaded.settings.cursorline());
  assert_eq!(reloaded.settings.inactive_brightness(), 0.5);
  assert_eq!(reloaded.settings.language(), "vie");
  let warning = reloaded.warning.unwrap();
  for key in ["reading.zoom", "ocr.language", "ocr.unknown"] {
    assert!(warning.contains(key), "{warning}");
  }
  std::fs::write(&path, "reading = false")?;
  let reloaded = loaded.source.reload(&loaded.settings);
  assert_eq!(reloaded.settings.reading, loaded.settings.reading);
  assert_eq!(reloaded.settings.ocr, Ocr::default());
  std::fs::write(&path, "")?;
  assert_eq!(
    loaded.source.reload(&loaded.settings).settings,
    Settings::default()
  );
  Ok(())
}

#[test]
fn reload_source_failures_preserve_settings() -> crate::Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("config.toml");
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  let loaded = load(&path, true);
  for text in ["[broken", "reading = { zoom = "] {
    std::fs::write(&path, text)?;
    let reloaded = loaded.source.reload(&loaded.settings);
    assert_eq!(reloaded.settings, loaded.settings);
    assert!(reloaded.warning.unwrap().contains("config.toml"));
  }
  std::fs::write(&path, [0xff])?;
  assert_eq!(loaded.source.reload(&loaded.settings).settings, loaded.settings);
  std::fs::remove_file(&path)?;
  let missing = loaded.source.reload(&loaded.settings);
  assert_eq!(missing.settings, loaded.settings);
  assert!(missing.warning.is_some());
  let optional = Source::file(&path, false).reload(&loaded.settings);
  assert_eq!(optional.settings, Settings::default());
  assert!(optional.warning.is_none());
  assert_eq!(optional.source, Source::file(&path, false));
  std::fs::write(&path, "[reading]\nzoom = 3")?;
  assert_eq!(optional.source.reload(&optional.settings).settings.zoom(), 3.0);
  let disabled = Options { no_config: true, ..Default::default() }.load();
  assert_eq!(disabled.source, Source::Disabled);
  assert_eq!(
    disabled.source.reload(&loaded.settings).settings,
    Settings::default()
  );
  Ok(())
}

#[test]
fn binding_targets_validate_by_context() {
  use vpdf_ui::bindings::{Context, PromptAction, Sequence, Target};
  let loaded = decoded(concat!(
    "[keybindings.normal]\n",
    "'<C-n>' = { action = 'tab.next' }\n",
    "'<A-n>' = { action = 'motion', motion = 'next-page' }\n",
    "'q' = { unbind = true }\n",
    "'x' = { action = 'prompt.start' }\n",
    "[keybindings.prompt]\n",
    "'<D-a>' = { action = 'prompt.start' }\n",
    "'<C-a>' = { unbind = true }\n",
    "[keybindings.password]\n",
    "'<D-e>' = { action = 'prompt.editor' }\n",
  ));
  let bindings = loaded.settings.bindings();
  assert_eq!(
    bindings.map(Context::Normal).get(&Sequence::parse("<C-n>").unwrap()),
    Some(&Target::Reader(vpdf_ui::keys::Action::Tab(true))),
  );
  assert!(
    bindings.map(Context::Normal).get(&Sequence::parse("q").unwrap()).is_none()
  );
  for context in
    [Context::Prompt, Context::Command, Context::Search, Context::Password]
  {
    assert_eq!(
      bindings.map(context).get(&Sequence::parse("<D-a>").unwrap()),
      Some(&Target::Prompt(PromptAction::Start))
    );
    assert!(
      bindings.map(context).get(&Sequence::parse("<C-a>").unwrap()).is_none()
    );
  }
  let warning = loaded.warning.unwrap();
  assert!(warning.contains("keybindings.normal.x"), "{warning}");
  assert!(warning.contains("keybindings.password.<D-e>"), "{warning}");
}

#[test]
fn binding_prefixes_require_explicit_unbinding() {
  use std::fmt::Write as _;
  use vpdf_ui::bindings::{Context, Sequence, Target};
  use vpdf_ui::keys::Action;
  let loaded = decoded(concat!(
    "[keybindings.normal]\n",
    "'g' = { action = 'quit' }\n",
    "'q' = { action = 'tab.next' }\n",
  ));
  let map = loaded.settings.bindings().map(Context::Normal);
  assert!(map.get(&Sequence::parse("g").unwrap()).is_none());
  assert_eq!(
    map.get(&Sequence::parse("q").unwrap()),
    Some(&Target::Reader(Action::Tab(true)))
  );
  assert!(loaded.warning.unwrap().contains("Prefix conflict with g"));
  let prefix = Sequence::parse("<C-x>").unwrap();
  let mut text =
    "[keybindings.prompt]\n'<C-x>' = { action = 'prompt.cancel' }\n".to_owned();
  for (sequence, _) in
    vpdf_ui::bindings::Bindings::default().map(Context::Prompt).entries()
  {
    if sequence.keys().starts_with(prefix.keys()) {
      writeln!(text, "'{sequence}' = {{ unbind = true }}").unwrap();
    }
  }
  let loaded = decoded(&text);
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  for context in [Context::Command, Context::Search, Context::Password] {
    assert!(
      loaded
        .settings
        .bindings()
        .map(context)
        .get(&Sequence::parse("<C-x>").unwrap())
        .unwrap()
        .cancels()
    );
  }
}

#[test]
fn prompt_bindings_reload_individually() {
  use vpdf_ui::bindings::{Context, PromptAction, Sequence, Target};
  let first = decoded(concat!(
    "[keybindings.prompt]\n",
    "'<C-a>' = { action = 'prompt.end' }\n",
    "'<D-a>' = { action = 'prompt.start' }\n",
  ));
  let next = parse_with(
    Path::new("config.toml"),
    concat!(
      "[keybindings.prompt]\n",
      "'<C-a>' = { action = 'missing' }\n",
      "'<D-b>' = { action = 'prompt.start' }\n",
    ),
    &first.settings,
  );
  for context in
    [Context::Prompt, Context::Command, Context::Search, Context::Password]
  {
    let map = next.settings.bindings().map(context);
    assert_eq!(
      map.get(&Sequence::parse("<C-a>").unwrap()),
      Some(&Target::Prompt(PromptAction::End))
    );
    assert!(map.get(&Sequence::parse("<D-a>").unwrap()).is_none());
    assert_eq!(
      map.get(&Sequence::parse("<D-b>").unwrap()),
      Some(&Target::Prompt(PromptAction::Start))
    );
  }
  assert!(next.warning.unwrap().contains("Unknown action missing"));
}

#[test]
fn remapped_grammar_preserves_counts_and_registers() {
  use vpdf_reader::motion::Motion;
  use vpdf_reader::objects::Object;
  use vpdf_ui::bindings::{Context, Sequence};
  use vpdf_ui::keys::{Action, Dispatch, Keys};
  let loaded = decoded(concat!(
    "[keybindings.cursor]\n",
    "'r' = { action = 'register.choose' }\n",
    "'c' = { action = 'count.digit', digit = 3 }\n",
    "'x' = { action = 'motion', motion = 'right' }\n",
    "[keybindings.operator]\n",
    "'o' = { action = 'object.begin', around = true }\n",
    "[keybindings.object]\n",
    "'x' = { action = 'object.select', object = 'word' }\n",
    "[keybindings.normal]\n",
    "'M' = { action = 'mark.choose' }\n",
    "'Z' = { action = 'mark.jump', exact = true }\n",
    "[keybindings.mark-jump]\n",
    "'<D-b>' = { action = 'history.previous' }\n",
  ));
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let bindings = loaded.settings.bindings();
  let mut keys = Keys::default();
  let mut feed = |text, context| {
    let mut result = None;
    for &key in Sequence::parse(text).unwrap().keys() {
      result = keys.feed_with(key, context, bindings);
    }
    result.map(|result| match result {
      Dispatch::Input(input) => input,
      Dispatch::Command(_) => panic!("Unexpected command"),
    })
  };
  let input = feed("racx", Context::Cursor).unwrap();
  assert_eq!(input.action, Action::Motion(Motion::Right));
  assert_eq!(input.count, 3);
  assert_eq!(input.register, 'a');
  let input = feed("rby", Context::Cursor).unwrap();
  assert_eq!(input.action, Action::Yank);
  assert_eq!(input.register, 'b');
  let input = feed("2ox", Context::Operator).unwrap();
  assert_eq!(input.action, Action::Object(Object::Word, true));
  assert_eq!(input.count, 2);
  assert_eq!(input.register, 'b');
  assert_eq!(feed("Ma", Context::Normal).unwrap().action, Action::Mark('a'));
  assert_eq!(
    feed("Zb", Context::Normal).unwrap().action,
    Action::GoMark('b', true)
  );
  assert_eq!(
    feed("Z<D-b>", Context::Normal).unwrap().action,
    Action::History(false)
  );
}

#[test]
fn themes_reload_independently() {
  use vpdf_model::theme::Theme;
  let mut previous = Settings::default();
  for &theme in Theme::ALL {
    let loaded = parse_with(
      Path::new("settings.toml"),
      &format!("[appearance]\ntheme = '{}'", theme.name()),
      &previous,
    );
    assert!(loaded.warning.is_none());
    assert_eq!(loaded.settings.theme(), theme);
    assert_eq!(
      decoded(&toml::to_string_pretty(&loaded.settings).unwrap()).settings,
      loaded.settings
    );
    previous = loaded.settings;
  }
  for value in ["'unknown'", "42", "[]"] {
    let loaded = parse_with(
      Path::new("settings.toml"),
      &format!("[appearance]\ntheme = {value}\n[reading]\nzoom = 3"),
      &previous,
    );
    assert_eq!(loaded.settings.theme(), Theme::Nord);
    assert_eq!(loaded.settings.zoom(), 3.0);
    assert!(loaded.warning.unwrap().contains("appearance.theme"));
  }
  let invalid =
    parse_with(Path::new("settings.toml"), "appearance = false", &previous);
  assert_eq!(invalid.settings.theme(), Theme::Nord);
  let omitted =
    parse_with(Path::new("settings.toml"), "[appearance]", &previous);
  assert_eq!(omitted.settings.theme(), Theme::Default);
}

#[test]
fn references_keep_values_sources_and_warnings() {
  use vpdf_ui::reference::Source as Origin;
  let first_path = Path::new("first.toml");
  let second_path = Path::new("second.toml");
  let first = parse(first_path, "[reading]\nzoom = 2\ncursorline = true");
  let next = parse_with(
    second_path,
    "[reading]\nzoom = 'bad'\ncursorline = false\nextra = 7",
    &first.settings,
  );
  let reference = next.settings.reference();
  let setting = |name| {
    reference.settings.iter().find(|setting| setting.name == name).unwrap()
  };
  let zoom = setting("reading.zoom");
  assert_eq!(zoom.default, serde_json::json!(1.0));
  assert_eq!(zoom.effective, serde_json::json!(2.0));
  assert_eq!(zoom.source, Origin::File(first_path.into()));
  assert_eq!(
    setting("reading.cursorline").source,
    Origin::File(second_path.into())
  );
  assert_eq!(setting("ocr.language").source, Origin::Default);
  assert!(reference.generation > first.settings.reference().generation);
  assert_eq!(reference.warnings.len(), 2);
  let invalid = parse_with(second_path, "reading = false", &first.settings);
  assert_eq!(
    invalid.settings.reference().settings,
    first.settings.reference().settings
  );
  let reset = parse_with(second_path, "", &first.settings);
  assert!(
    reset
      .settings
      .reference()
      .settings
      .iter()
      .all(|setting| setting.source == Origin::Default)
  );
  let mut settings = next.settings;
  settings.set_theme(vpdf_model::theme::Theme::Nord);
  assert_eq!(
    settings
      .reference()
      .settings
      .iter()
      .find(|setting| setting.name == "appearance.theme")
      .unwrap()
      .source,
    Origin::Command
  );
}

#[test]
fn binding_sources_survive_inheritance_and_rejection() {
  use vpdf_ui::bindings::{Context, Sequence};
  use vpdf_ui::reference::Source as Origin;
  let first_path = Path::new("first.toml");
  let next_path = Path::new("next.toml");
  let first = parse(
    first_path,
    concat!(
      "[keybindings.prompt]\n'<C-a>' = { action = 'prompt.start' }\n",
      "[keybindings.normal]\n'h' = { action = 'tab.next' }\n",
      "'xy' = { action = 'tab.next' }\n'q' = { unbind = true }",
    ),
  );
  let next = parse_with(
    next_path,
    concat!(
      "[keybindings.normal]\n'h' = { action = 'unknown' }\n",
      "'q' = { action = 'unknown' }\n'g' = { action = 'quit' }\n",
      "[keybindings.prompt]\n'<C-a>' = { action = 'unknown' }\n",
      "[keybindings.command]\n'<C-a>' = { action = 'prompt.end' }\n",
    ),
    &first.settings,
  );
  for context in [Context::Prompt, Context::Search, Context::Password] {
    assert_eq!(
      next
        .settings
        .bindings()
        .map(context)
        .source(&Sequence::parse("<C-a>").unwrap()),
      Origin::File(first_path.into())
    );
  }
  assert_eq!(
    next
      .settings
      .bindings()
      .map(Context::Command)
      .source(&Sequence::parse("<C-a>").unwrap()),
    Origin::File(next_path.into())
  );
  let map = next.settings.bindings().map(Context::Normal);
  for key in ["h", "q"] {
    assert_eq!(
      map.source(&Sequence::parse(key).unwrap()),
      Origin::File(first_path.into())
    );
  }
  assert_eq!(map.source(&Sequence::parse("g").unwrap()), Origin::Default);
  let reference =
    next.settings.help_reference(vpdf_ui::topics::Invocation::home());
  assert_eq!(
    reference.bindings,
    vpdf_ui::bindings::Bindings::default().snapshot()
  );
  assert_ne!(reference.configuration.warnings, [] as [String; 0]);
}

#[test]
fn reference_uses_applied_configuration() -> crate::Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("config.toml");
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  let loaded = load(&path, true);
  std::fs::write(&path, "[reading]\nzoom = 3")?;
  let snapshot =
    loaded.settings.help_reference(vpdf_ui::topics::Invocation::home());
  assert_eq!(
    snapshot
      .configuration
      .settings
      .iter()
      .find(|setting| setting.name == "reading.zoom")
      .unwrap()
      .effective,
    serde_json::json!(2.0)
  );
  std::fs::write(&path, "[broken")?;
  let invalid = loaded.source.reload(&loaded.settings);
  assert_eq!(invalid.settings.zoom(), 2.0);
  assert!(
    invalid.settings.reference().generation > snapshot.configuration.generation
  );
  assert_ne!(invalid.settings.reference().warnings, [] as [String; 0]);
  Ok(())
}

#[test]
fn recent_bindings_inherit_remaps() {
  use vpdf_ui::bindings::{Context, Sequence, Target};
  use vpdf_ui::keys::Action;
  let source = "\
[keybindings.panel]\n\
'<Space>' = { action = 'screen', amount = -1 }\n\
x = { unbind = true }\n\
[keybindings.home]\n\
x = { command = 'help' }\n\
[keybindings.recent]\n\
s = { action = 'recent.toggle' }\n\
r = { action = 'recent.remove' }\n";
  let loaded = decoded(source);
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let bindings = loaded.settings.bindings();
  let space = Sequence::parse("<Space>").unwrap();
  let x = Sequence::parse("x").unwrap();
  assert_eq!(
    bindings.map(Context::Recent).get(&space),
    Some(&Target::Reader(Action::Screen(-1)))
  );
  assert!(bindings.map(Context::Recent).is_unbound(&x));
  assert_eq!(
    bindings.map(Context::HomeRecent).get(&x),
    Some(&Target::Command("help".into()))
  );
  assert_eq!(
    bindings.map(Context::Recent).get(&Sequence::parse("s").unwrap()),
    Some(&Target::Reader(Action::RecentToggle))
  );
  assert_eq!(
    bindings.map(Context::Recent).get(&Sequence::parse("r").unwrap()),
    Some(&Target::Reader(Action::RecentRemove))
  );
}
