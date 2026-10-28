use super::*;
use crate::screen::cells;
use crate::watch::SETTLE;

/// A session with a document pane and a Recent pane.
fn configured(
  root: &Path,
  text: Option<&str>,
  frontend: crate::raster::Frontend,
) -> Result<(Session, PathBuf)> {
  let config = root.join("vpdf config.toml");
  if let Some(text) = text {
    std::fs::write(&config, text)?;
  }
  let loaded = crate::config::Options { config: Some(config.clone()) }.load();
  let loaded = crate::config::Loaded {
    source: crate::config::Source::File {
      path: config.clone(),
      explicit: false,
    },
    ..loaded
  };
  let mut session = Session { frontend, ..Session::configured(None, loaded) };
  let pdf = root.join("book.pdf");
  crate::fixtures::pdf(&pdf, &["one", "two"], "")?;
  session.open(&pdf, true, cells(100, 30))?;
  session.recent_tab(cells(100, 30))?;
  session.activate(0)?;
  Ok((session, config))
}

fn frontends() -> [crate::raster::Frontend; 2] {
  [
    crate::raster::Frontend::Terminal,
    crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
  ]
}

/// Runs fallback polls until the watcher settles.
fn settle(session: &mut Session) -> bool {
  let start = Instant::now();
  let first = session.watch_config(true, start);
  first || session.watch_config(false, start + SETTLE)
}

fn opacity(session: &mut Session) -> Option<f64> {
  session
    .app()
    .unwrap()
    .settings
    .opacity()
    .inactive
    .map(vpdf_model::theme::UnitInterval::get)
}

#[test]
fn edit_command_parses() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (mut session, _) =
    configured(root.path(), Some(""), crate::raster::Frontend::Terminal)?;
  session.app().unwrap().command("config edit")?;
  assert_eq!(session.app().unwrap().request, Some(Request::ConfigEdit));
  Ok(())
}

#[test]
fn native_edit_matches_settings() -> Result<()> {
  use super::native::commands::Effect;
  let root = tempfile::tempdir()?;
  let native = crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none());
  let (mut session, config) = configured(root.path(), None, native)?;
  session.edit_config()?;
  assert_eq!(std::fs::read(&config)?, b"");
  let opened = Effect::EditConfig(config.clone());
  assert_eq!(session.native.effects.last(), Some(&opened));
  std::fs::write(&config, "[reading]\nzoom = 2.0\n")?;
  session.app().unwrap().command("config edit")?;
  session.settle()?;
  assert_eq!(session.native.effects.last(), Some(&opened));
  assert_eq!(std::fs::read_to_string(&config)?, "[reading]\nzoom = 2.0\n");
  let mut disabled = Session::new(None);
  let error = disabled.edit_config().unwrap_err();
  assert_eq!(error.to_string(), "Configuration disabled; nothing to edit");
  Ok(())
}

#[test]
fn terminal_edit_applies_on_return() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (mut session, config) =
    configured(root.path(), Some(""), crate::raster::Frontend::Terminal)?;
  let position = session.app().unwrap().reader.cursor;
  let generation = session.app().unwrap().document_generation;
  session.edit_config_with(|path| {
    assert_eq!(path, config);
    std::fs::write(path, "[appearance.opacity]\ninactive = 0.3\n")?;
    Ok(())
  })?;
  assert_eq!(opacity(&mut session), Some(0.3));
  assert_eq!(session.app().unwrap().reader.cursor, position);
  assert_eq!(session.app().unwrap().document_generation, generation);
  assert!(!settle(&mut session), "editor save reloaded twice");
  let error = session
    .edit_config_with(|path| {
      std::fs::write(path, "[appearance.opacity]\ninactive = 0.6\n")?;
      anyhow::bail!("vim exited with status 1")
    })
    .unwrap_err();
  assert_eq!(error.to_string(), "vim exited with status 1");
  assert_eq!(opacity(&mut session), Some(0.6));
  Ok(())
}

#[test]
fn files_created_by_edit_join_the_watch() -> Result<()> {
  for frontend in frontends() {
    let root = tempfile::tempdir()?;
    let native = matches!(frontend, crate::raster::Frontend::Native(_));
    let (mut session, config) = configured(root.path(), None, frontend)?;
    assert!(!config.exists());
    if native {
      session.edit_config()?;
      assert_eq!(std::fs::read(&config)?, b"");
      std::fs::write(&config, "[appearance.opacity]\ninactive = 0.3\n")?;
      assert!(settle(&mut session));
    } else {
      session.edit_config_with(|path| {
        std::fs::write(path, "[appearance.opacity]\ninactive = 0.3\n")?;
        Ok(())
      })?;
    }
    assert_eq!(opacity(&mut session), Some(0.3));
  }
  Ok(())
}

#[test]
fn saved_config_reloads_everywhere() -> Result<()> {
  for frontend in frontends() {
    let root = tempfile::tempdir()?;
    let (mut session, config) = configured(root.path(), Some(""), frontend)?;
    let position = session.app().unwrap().reader.cursor;
    let generation = session.app().unwrap().document_generation;
    assert!(!settle(&mut session), "unchanged file reloaded");
    std::fs::write(&config, "[appearance.opacity]\ninactive = 0.3\n")?;
    assert!(settle(&mut session));
    assert_eq!(opacity(&mut session), Some(0.3));
    std::fs::write(&config, "[appearance.opacity\n")?;
    assert!(settle(&mut session));
    assert_eq!(opacity(&mut session), Some(0.3));
    assert!(session.config_warning.is_some());
    assert!(!settle(&mut session), "warning repeated");
    std::fs::write(&config, "")?;
    assert!(settle(&mut session));
    assert_eq!(opacity(&mut session), None);
    assert!(session.config_warning.is_none());
    assert_eq!(session.app().unwrap().reader.cursor, position);
    assert_eq!(session.app().unwrap().document_generation, generation);
  }
  Ok(())
}

#[test]
fn start_screen_reloads() -> Result<()> {
  for frontend in frontends() {
    let root = tempfile::tempdir()?;
    let config = root.path().join("config.toml");
    std::fs::write(&config, "")?;
    let loaded = crate::config::Options { config: Some(config.clone()) }.load();
    let mut session = Session { frontend, ..Session::configured(None, loaded) };
    session.recent_tab(cells(80, 24))?;
    std::fs::write(&config, "[appearance.opacity]\ninactive = 0.4\n")?;
    assert!(settle(&mut session));
    assert_eq!(session.settings.opacity().inactive.unwrap().get(), 0.4);
  }
  let mut disabled = Session::configured(
    None,
    crate::config::Options { config: Some("/nonexistent/vpdf.toml".into()) }
      .load(),
  );
  assert!(disabled.config_watch.is_none());
  assert!(!disabled.watch_config(true, Instant::now()));
  Ok(())
}
