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
    "[reading]\ncursorline = true\nzoom = 1.0\n\n[ocr]\nlanguage = \"eng\"\n"
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
