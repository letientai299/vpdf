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
      "[appearance]\ntheme = \"system\"\nascii_glyphs = false\n",
      "ui_font_size = 16\n\n",
      "[appearance.opacity]\n\n",
      "[reading]\ncursorline = true\nzoom = 1.0\n",
      "restore_session = \"system\"\n\n",
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
fn restore_session_reads_a_choice() {
  for (value, expected, warned) in [
    ("'never'", Restore::Never, false),
    ("'always'", Restore::Always, false),
    ("'system'", Restore::System, false),
    ("false", Restore::System, true),
    ("'later'", Restore::System, true),
  ] {
    let loaded = decoded(&format!("[reading]\nrestore_session = {value}"));
    assert_eq!(loaded.settings.restore_session(), expected, "{value}");
    assert_eq!(loaded.warning.is_some(), warned, "{value}");
  }
}

#[test]
fn opacity_validates_keys() {
  for (key, default) in Opacity::default().values() {
    for (value, expected) in [
      ("0", Some(0.0)),
      ("0.9", Some(0.9)),
      ("1", Some(1.0)),
      ("-0.1", None),
      ("1.1", None),
      ("nan", None),
      ("inf", None),
      ("-inf", None),
      ("true", None),
      ("'0.9'", None),
    ] {
      let loaded = decoded(&format!(
        "[reading]\ncursorline = false\n[appearance.opacity]\n{key} = {value}"
      ));
      assert_eq!(
        loaded
          .settings
          .opacity()
          .values()
          .find(|(name, _)| *name == key)
          .unwrap()
          .1,
        expected.unwrap_or(default),
        "{key}: {value}"
      );
      assert!(!loaded.settings.cursorline());
      assert_eq!(
        loaded.warning.is_none(),
        expected.is_some(),
        "{key}: {value}"
      );
      if let Some(warning) = loaded.warning {
        assert!(warning.contains(&format!("appearance.opacity.{key}")));
      }
    }
  }
  for text in [
    "[appearance.opacity]\nunknown = 0.5",
    "[reading]\ninactive_brightness = 0.8",
  ] {
    let loaded = decoded(text);
    assert_eq!(loaded.settings.opacity(), Opacity::default());
    assert!(loaded.warning.unwrap().contains("unknown setting"));
  }
}

#[test]
fn opacity_reload_retains_sources() {
  use vpdf_ui::reference::Source as Origin;
  let first = parse(
    Path::new("first.toml"),
    "[appearance.opacity]\nselection = 1\nveil = 0.5",
  );
  let next = parse_with(
    Path::new("next.toml"),
    "[appearance.opacity]\nselection = -1\ninactive = 0.4",
    &first.settings,
  );
  assert_eq!(next.settings.opacity().selection.unwrap().get(), 1.0);
  assert_eq!(next.settings.opacity().veil, None);
  let reference = next.settings.reference();
  let selection = reference
    .settings
    .iter()
    .find(|s| s.name == "appearance.opacity.selection")
    .unwrap();
  assert_eq!(selection.source, Origin::File("first.toml".into()));
  assert_eq!(selection.default, serde_json::json!(85.0 / 255.0));
  assert_eq!(selection.effective, serde_json::json!(1.0));
  let invalid = parse_with(
    Path::new("bad.toml"),
    "[appearance]\nopacity = false",
    &first.settings,
  );
  assert_eq!(invalid.settings.opacity(), first.settings.opacity());
  assert!(invalid.warning.is_some());
  assert_eq!(
    decoded(&toml::to_string(&first.settings).unwrap()).settings.opacity(),
    first.settings.opacity()
  );
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
  let options = Options { config: Some(path) };
  assert_eq!(options.load_from(None, None).settings.zoom(), 2.0);
  let options = Options { config: Some(dir.path().join("NONE")) };
  assert_eq!(options.load_from(None, None).settings, Settings::default());
  assert!(load(dir.path(), true).warning.is_some());
  Ok(())
}

#[test]
fn selection_flags() {
  #[derive(Parser)]
  struct Cli {
    #[command(flatten)]
    options: Options,
  }
  for flag in
    ["--config-help", "--config-licenses", "--no-config", "--config-example"]
  {
    let error = Cli::try_parse_from(["vpdf", flag]).err().unwrap();
    assert_eq!(error.kind(), clap::error::ErrorKind::UnknownArgument);
  }
  for flag in ["-u", "--config"] {
    let options =
      Cli::try_parse_from(["vpdf", flag, "relative.toml"]).unwrap().options;
    assert_eq!(options.config, Some(PathBuf::from("relative.toml")));
  }
}

#[test]
fn explicit_files_disable_or_warn() -> crate::Result<()> {
  let root = tempfile::tempdir()?;
  for name in ["NONE", "missing.toml"] {
    let options = Options { config: Some(root.path().join(name)) };
    let loaded = options.load_from(None, None);
    assert_eq!(loaded.settings, Settings::default());
    assert_eq!(loaded.source, Source::Disabled);
    assert!(loaded.warning.is_none());
  }
  let path = root.path().join("bad.toml");
  std::fs::write(&path, [0xff])?;
  let loaded = Options { config: Some(path.clone()) }.load_from(None, None);
  assert_eq!(loaded.source, Source::Disabled);
  assert!(loaded.warning.is_none());
  for text in ["[broken", "[reading]\nzoom = 'bad'"] {
    std::fs::write(&path, text)?;
    let loaded = Options { config: Some(path.clone()) }.load_from(None, None);
    assert!(loaded.warning.unwrap().contains("bad.toml"));
    assert_eq!(loaded.source, Source::file(&path, true));
  }
  let loaded =
    Options { config: Some(root.path().to_owned()) }.load_from(None, None);
  assert_eq!(loaded.source, Source::Disabled);
  assert!(loaded.warning.is_none());
  Ok(())
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
  let options = Options { config: Some(root.path().join("NONE")) };
  assert_eq!(
    options.load_from(None, Some(home.as_os_str())).settings,
    Settings::default()
  );
  Ok(())
}

#[test]
#[cfg(unix)]
fn unreadable_file_disables() -> crate::Result<()> {
  use std::os::unix::fs::PermissionsExt;
  let root = tempfile::tempdir()?;
  let path = root.path().join("config.toml");
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o0))?;
  let loaded = Options { config: Some(path.clone()) }.load_from(None, None);
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
  assert_eq!(loaded.settings, Settings::default());
  assert!(loaded.warning.is_none());
  assert_eq!(loaded.source, Source::Disabled);
  Ok(())
}

#[test]
fn explicit_relative_file() -> crate::Result<()> {
  let root = tempfile::tempdir_in(".")?;
  let path = Path::new(root.path().file_name().unwrap()).join("config.toml");
  assert!(path.is_relative());
  std::fs::write(&path, "[reading]\nzoom = 2")?;
  let options = Options { config: Some(path) };
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
      "[appearance.opacity]\ninactive = 0.3\n[ocr]\nlanguage = 'vie'",
    ),
  )?;
  let loaded = load(&path, true);
  std::fs::write(
    &path,
    concat!(
      "[reading]\nzoom = 'bad'\n[appearance.opacity]\ninactive = 0.5\n",
      "[ocr]\nlanguage = 7\nunknown = true",
    ),
  )?;
  let reloaded = loaded.source.reload(&loaded.settings);
  assert_eq!(reloaded.settings.zoom(), 2.0);
  assert!(reloaded.settings.cursorline());
  assert_eq!(reloaded.settings.opacity().inactive.unwrap().get(), 0.5);
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
  let disabled = Options { config: Some(root.path().join("NONE")) }.load();
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
    "'ZZ' = { unbind = true }\n'ZQ' = { unbind = true }\n",
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
    assert_eq!(loaded.settings.theme(), &ThemeChoice::Builtin(theme));
    assert_eq!(
      decoded(&toml::to_string_pretty(&loaded.settings).unwrap()).settings,
      loaded.settings
    );
    previous = loaded.settings;
  }
  for value in ["'default'", "'nord'", "'unknown'", "42", "[]"] {
    let loaded = parse_with(
      Path::new("settings.toml"),
      &format!("[appearance]\ntheme = {value}\n[reading]\nzoom = 3"),
      &previous,
    );
    assert_eq!(loaded.settings.theme(), &ThemeChoice::Builtin(Theme::System));
    assert_eq!(loaded.settings.zoom(), 3.0);
    assert!(loaded.warning.unwrap().contains("appearance.theme"));
  }
  let invalid =
    parse_with(Path::new("settings.toml"), "appearance = false", &previous);
  assert_eq!(invalid.settings.theme(), &ThemeChoice::Builtin(Theme::Dark));
  let omitted =
    parse_with(Path::new("settings.toml"), "[appearance]", &previous);
  assert_eq!(omitted.settings.theme(), &ThemeChoice::Builtin(Theme::System));
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
  settings.set_theme(vpdf_model::theme::Theme::Dark);
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

fn write_palette(path: &Path, background: &str) -> crate::Result<()> {
  let mut value: toml::Value =
    toml::from_str(include_str!("../../etc/themes/dark.toml"))?;
  value["background"] = background.into();
  std::fs::write(path, toml::to_string(&value)?)?;
  Ok(())
}

#[test]
fn theme_paths_reload() -> crate::Result<()> {
  use Colors;
  let root = tempfile::tempdir()?;
  let directory = root.path().join("settings");
  std::fs::create_dir_all(directory.join("palettes"))?;
  let palette = directory.join("palettes/custom.toml");
  let config = directory.join("config.toml");
  write_palette(&palette, "#123456")?;
  std::fs::write(
    &config,
    "[appearance]\ntheme = 'palettes/custom.toml'\n[reading]\nzoom = 2",
  )?;
  let loaded = Options { config: Some(config) }.load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let ThemeChoice::File { path, palette: colors } = loaded.settings.theme()
  else {
    panic!("Expected theme file");
  };
  assert_eq!(path, &palette);
  assert_eq!(colors.background, [0x12, 0x34, 0x56]);
  assert_eq!(
    loaded.settings.colors(Colors::Terminal),
    Colors::Palette(*colors)
  );
  let serialized = toml::to_string(&loaded.settings)?;
  assert_eq!(decoded(&serialized).settings, loaded.settings);
  write_palette(&palette, "#abcdef")?;
  let reloaded = loaded.source.reload(&loaded.settings);
  assert!(reloaded.warning.is_none());
  assert_eq!(
    reloaded.settings.colors(Colors::Terminal).palette().background,
    [0xab, 0xcd, 0xef]
  );
  std::fs::remove_file(&palette)?;
  let missing = loaded.source.reload(&reloaded.settings);
  assert!(missing.warning.unwrap().contains("custom.toml"));
  assert_eq!(missing.settings.colors(Colors::Terminal), Colors::Terminal);
  assert_eq!(missing.settings.zoom(), 2.0);
  Ok(())
}

#[test]
fn theme_files_fall_back() -> crate::Result<()> {
  use vpdf_model::theme::{Colors, Theme};
  let root = tempfile::tempdir()?;
  let config = root.path().join("config.toml");
  let palette = root.path().join("custom.toml");
  let text = include_str!("../../etc/themes/dark.toml");
  let missing_token = text
    .lines()
    .filter(|line| !line.starts_with("foreground ="))
    .collect::<Vec<_>>()
    .join("\n");
  let mut invalid_hex: toml::Value = toml::from_str(text)?;
  invalid_hex["foreground"] = "#bad".into();
  let mut previous = Settings::default();
  previous.set_theme(Theme::Dark);
  for contents in [
    b"[broken".to_vec(),
    missing_token.into_bytes(),
    toml::to_string(&invalid_hex)?.into_bytes(),
    vec![0xff],
  ] {
    std::fs::write(&palette, contents)?;
    let loaded = parse_with(
      &config,
      "[appearance]\ntheme = 'custom.toml'\n[reading]\nzoom = 3",
      &previous,
    );
    let warning = loaded.warning.unwrap();
    assert!(warning.contains("appearance.theme"), "{warning}");
    assert!(warning.contains("custom.toml"), "{warning}");
    assert_eq!(loaded.settings.colors(Colors::Terminal), Colors::Terminal);
    assert_eq!(loaded.settings.zoom(), 3.0);
  }
  std::fs::remove_file(&palette)?;
  std::fs::create_dir(&palette)?;
  let loaded =
    parse_with(&config, "[appearance]\ntheme = 'custom.toml'", &previous);
  assert!(loaded.warning.is_some());
  assert_eq!(loaded.settings.colors(Colors::Terminal), Colors::Terminal);
  Ok(())
}

#[test]
fn theme_tilde_paths() -> crate::Result<()> {
  let Some(home) = std::env::var_os("HOME") else { return Ok(()) };
  let home = Path::new(&home);
  let root =
    tempfile::Builder::new().prefix(".vpdf-theme-").tempdir_in(home)?;
  let path = root.path().join("custom.toml");
  write_palette(&path, "#123456")?;
  let relative = path.strip_prefix(home)?;
  let text = format!("[appearance]\ntheme = '~/{}'", relative.display());
  let loaded = parse(Path::new("/unrelated/config.toml"), &text);
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let ThemeChoice::File { path: chosen, .. } = loaded.settings.theme() else {
    panic!("Expected theme file");
  };
  assert_eq!(chosen, &path);
  Ok(())
}

#[test]
fn theme_file_commands() -> crate::Result<()> {
  use Colors;
  let root = tempfile::tempdir_in(std::env::current_dir()?)?;
  let path = root.path().join("custom.toml");
  write_palette(&path, "#123456")?;
  let pdf = root.path().join("document.pdf");
  crate::fixtures::pdf(&pdf, &["Theme file"], "")?;
  let mut session = crate::session::Session::new(None);
  session.open(&pdf, true, crate::screen::cells(100, 30))?;
  let relative = path.strip_prefix(std::env::current_dir()?)?;
  session.app().unwrap().command(&format!("theme {}", relative.display()))?;
  session.settle()?;
  let reference = session.app().unwrap().settings.reference();
  let theme =
    reference.settings.iter().find(|s| s.name == "appearance.theme").unwrap();
  assert_eq!(theme.source, vpdf_ui::reference::Source::Command);
  assert_eq!(reference.palette.background, [0x12, 0x34, 0x56]);
  assert!(std::fs::read_to_string(&path)?.contains("#123456"));
  std::fs::write(&path, "[broken")?;
  session.app().unwrap().command(&format!("theme {}", relative.display()))?;
  session.settle()?;
  assert_eq!(
    session.app().unwrap().settings.colors(Colors::Terminal),
    Colors::Terminal
  );
  assert!(session.app().unwrap().reader.message.contains("custom.toml"));
  write_palette(&path, "#abcdef")?;
  session.app().unwrap().command(&format!("theme {}", relative.display()))?;
  session.settle()?;
  assert_eq!(
    session
      .app()
      .unwrap()
      .settings
      .colors(Colors::Terminal)
      .palette()
      .background,
    [0xab, 0xcd, 0xef]
  );
  std::fs::remove_file(&path)?;
  session.app().unwrap().command(&format!("theme {}", relative.display()))?;
  session.settle()?;
  assert_eq!(
    session.app().unwrap().settings.colors(Colors::Terminal),
    Colors::Terminal
  );
  assert!(session.app().unwrap().reader.message.contains("custom.toml"));
  Ok(())
}

#[test]
fn theme_file_bindings() {
  use vpdf_ui::bindings::{Context, Sequence, Target};
  let loaded =
    decoded("[keybindings.normal]\nQ = { command = 'theme missing.toml' }");
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  assert_eq!(
    loaded
      .settings
      .bindings()
      .map(Context::Normal)
      .get(&Sequence::parse("Q").unwrap()),
    Some(&Target::Command("theme missing.toml".into()))
  );
}

#[test]
fn ui_font_size_validates_range() {
  for (value, expected, warned) in [
    ("10", 10.0, false),
    ("24", 24.0, false),
    ("32", 32.0, false),
    ("9", 16.0, true),
    ("33", 16.0, true),
    ("nan", 16.0, true),
    ("16.5", 16.0, true),
    ("'large'", 16.0, true),
  ] {
    let loaded = decoded(&format!("[appearance]\nui_font_size = {value}"));
    assert_eq!(loaded.settings.ui_font_size(), expected, "{value}");
    assert_eq!(loaded.warning.is_some(), warned, "{value}");
    assert_eq!(loaded.settings.zoom(), Settings::default().zoom());
  }
}

#[test]
fn ensure_file_creates_once_and_keeps_contents() -> crate::Result<()> {
  use std::os::unix::fs::PermissionsExt;
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("vpdf/config.toml");
  ensure_file(&path)?;
  assert_eq!(std::fs::read(&path)?, b"");
  std::fs::write(&path, "[reading]\nzoom = 2.0\n")?;
  std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444))?;
  ensure_file(&path)?;
  assert_eq!(std::fs::read_to_string(&path)?, "[reading]\nzoom = 2.0\n");
  Ok(())
}
