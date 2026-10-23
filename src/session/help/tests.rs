use super::*;
use crate::screen::cells;
use std::time::{Duration, Instant};

#[test]
fn interrupted_lookup_preserves_snapshot() -> Result<()> {
  let root = tempfile::tempdir()?;
  let first = root.path().join("first.pdf");
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&first, &["first"], "")?;
  crate::fixtures::pdf(&second, &["second"], "")?;
  for outcome in ["superseded", "discarded", "failed"] {
    let mut session = Session::new(None);
    session.open(&first, true, cells(120, 40))?;
    let original = session.app().id;
    invoke(&mut session, "navigation")?;
    let help = session.active;
    let reference = session.help.reference.clone().unwrap();
    let document = session.app().manual.clone().unwrap();
    session.app().set_mark('a')?;
    session.open(&second, true, cells(120, 40))?;
    session.app().command("help config")?;
    session.settle()?;
    assert!(session.help.pending.is_some());
    if outcome == "failed" {
      session.help_result(
        Err(anyhow::anyhow!("Help generation failed")),
        &mut std::io::sink(),
      );
    }
    session.activate(help)?;
    if outcome == "discarded" {
      wait(&mut session);
      assert_eq!(session.help.reference, Some(reference.clone()));
    }
    invoke(&mut session, "sample")?;
    assert!(session.app().marks.contains_key(&'a'));
    assert!(std::sync::Arc::ptr_eq(
      session.app().manual.as_ref().unwrap(),
      &document
    ));
    assert_eq!(session.help.reference, Some(reference));
    session.close()?;
    assert_eq!(session.app().id, original);
  }
  Ok(())
}

#[test]
fn lookup_during_refresh_uses_builtins() -> Result<()> {
  let root = tempfile::tempdir()?;
  let config = root.path().join("config.toml");
  std::fs::write(&config, "")?;
  let loaded = crate::config::Options {
    config: Some(config.clone()),
    ..Default::default()
  }
  .load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let mut session = Session::configured(None, loaded);
  invoke(&mut session, "navigation")?;
  let invocation = session.help.reference.as_ref().unwrap().invocation.clone();
  std::fs::write(
    &config,
    "[keybindings.normal]\nQ = { command = 'help sample' }\n",
  )?;
  session.reload_config();
  assert!(session.help.pending.is_some());
  invoke(&mut session, "h")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    builtin_topic("h", vpdf_ui::bindings::Context::Normal)
  );
  let reference = session.help.reference.as_ref().unwrap();
  assert_eq!(reference.invocation, invocation);
  assert_eq!(reference.configuration, session.settings.reference());
  Ok(())
}

#[test]
fn topic_lookup_retains_state() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first"], "")?;
  for frontend in [
    crate::raster::Frontend::Terminal,
    crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
  ] {
    let mut session = Session { frontend, ..Session::new(None) };
    session.open(&path, true, cells(120, 40))?;
    let source = session.app().id;
    invoke(&mut session, "navigation")?;
    let reference = session.help.reference.clone().unwrap();
    let document = session.app().manual.clone().unwrap();
    session.app().set_mark('a')?;
    let mark = session.app().marks[&'a'];
    session.app().start_search("navigation", false, false)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while session.app().search.phase().running() {
      session.app().search_updates()?;
      assert!(Instant::now() < deadline, "Help search timed out");
      std::thread::sleep(Duration::from_millis(2));
    }
    let hits = session.app().search.hits().len();
    assert!(hits > 0);
    let jumps: Vec<_> = session.app().reader.jumps().collect();
    assert_ne!(jumps, [] as [vpdf_model::text::Position; 0]);
    for topic in ["sample", "sample"] {
      invoke(&mut session, topic)?;
      let app = session.app();
      assert!(std::sync::Arc::ptr_eq(app.manual.as_ref().unwrap(), &document));
      assert_eq!(app.document_generation, 0);
      assert_eq!(app.marks.get(&'a'), Some(&mark));
      assert_eq!(app.search.query().text, "navigation");
      assert_eq!(app.search.hits().len(), hits);
      assert!(jumps.iter().all(|jump| app.reader.jumps().any(|p| p == *jump)));
      app.go_mark('a', true)?;
      assert_eq!(app.navigation_position(), mark);
    }
    assert_eq!(session.help.reference, Some(reference));
    session.close()?;
    assert_eq!(session.app().id, source);
  }
  Ok(())
}

#[test]
fn lookup_and_refresh_retain_preview() -> Result<()> {
  use vpdf_ui::keys::{Action, Input};
  for refresh in [false, true] {
    for frontend in [
      crate::raster::Frontend::Terminal,
      crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
    ] {
      let mut session = Session { frontend, ..Session::new(None) };
      invoke(&mut session, "sample")?;
      let id = session.app().id;
      session.app().command("text")?;
      assert_eq!(session.app().preview, Some(false));
      if refresh {
        session.set_theme(vpdf_model::theme::Theme::Nord);
        wait(&mut session);
      } else {
        invoke(&mut session, "sample")?;
      }
      assert!(session.app().text_mode);
      session.app().action(Input::plain(Action::Quit))?;
      session.settle()?;
      assert_eq!(session.tabs.len(), 1);
      assert_eq!(session.app().id, id);
      assert!(!session.app().text_mode);
      assert!(session.app().preview.is_none());
    }
  }
  Ok(())
}

#[test]
fn nested_lookup_uses_builtin_bindings() -> Result<()> {
  let root = tempfile::tempdir()?;
  let config = root.path().join("config.toml");
  std::fs::write(
    &config,
    "[keybindings.cursor]\nh = { action = 'fit.page' }\n",
  )?;
  let loaded = crate::config::Options {
    config: Some(config.clone()),
    ..Default::default()
  }
  .load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let mut session = Session::configured(None, loaded);
  invoke(&mut session, "navigation")?;
  session.app().set_mark('a')?;
  session.app().reader.enter_cursor();
  let document = session.app().manual.clone().unwrap();
  invoke(&mut session, "h")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    builtin_topic("h", vpdf_ui::bindings::Context::Cursor)
  );
  assert!(session.app().marks.contains_key(&'a'));
  assert!(std::sync::Arc::ptr_eq(
    session.app().manual.as_ref().unwrap(),
    &document
  ));
  std::fs::write(
    &config,
    "[keybindings.cursor]\nh = { action = 'tab.next' }\n",
  )?;
  session.reload_config();
  wait(&mut session);
  session.app().reader.enter_cursor();
  let reference = session.help.reference.clone().unwrap();
  assert_eq!(reference.configuration, session.settings.reference());
  let document = session.app().manual.clone().unwrap();
  invoke(&mut session, "h")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    builtin_topic("h", vpdf_ui::bindings::Context::Cursor)
  );
  assert_eq!(session.help.reference, Some(reference));
  assert!(std::sync::Arc::ptr_eq(
    session.app().manual.as_ref().unwrap(),
    &document
  ));
  Ok(())
}

fn wait(session: &mut Session) {
  let deadline = Instant::now() + Duration::from_secs(15);
  while session.help.pending.is_some() {
    session.help_updates(&mut std::io::sink());
    assert!(Instant::now() < deadline, "Help generation timed out");
    std::thread::sleep(Duration::from_millis(2));
  }
}

fn invoke(session: &mut Session, query: &str) -> Result<()> {
  if session.tabs.is_empty() {
    session.request_help(query, Invocation::Home, cells(120, 40))?;
  } else {
    session.app().command(&format!("help {query}"))?;
    session.settle()?;
  }
  wait(session);
  assert!(session.app().error.is_none(), "{:?}", session.app().error);
  Ok(())
}

#[test]
fn home_reuses_help_and_returns() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "config")?;
  let id = session.app().id;
  assert_eq!(session.app().name(), "Help");
  assert_eq!(
    session.app().document_identity(),
    std::path::Path::new(crate::manual::ID)
  );
  assert_eq!(session.state.recent, [] as [crate::state::Entry; 0]);
  assert!(!session.app().watch()?);
  invoke(&mut session, "")?;
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.app().id, id);
  assert_eq!(session.app().navigation_position().page.0, 0);
  let path = session.app().path.clone();
  let wake = session.app().wake.clone();
  session.close()?;
  assert!(session.tabs.is_empty() && session.home.is_some());
  assert!(!session.quit);
  assert_eq!(session.state.recent, [] as [crate::state::Entry; 0]);
  drop(session);
  assert!(path.exists());
  drop(wake);
  let deadline = Instant::now() + Duration::from_secs(5);
  while path.exists() {
    assert!(Instant::now() < deadline, "Help backing file retained");
    std::thread::sleep(Duration::from_millis(2));
  }
  Ok(())
}

#[test]
fn returns_to_invoking_pane_after_splits() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first", "second"], "")?;
  let mut session = Session::new(Some(root.path().join("state.json")));
  session.open(&path, true, cells(120, 40))?;
  session.app().command("2")?;
  let original = session.app().id;
  let position = session.app().navigation_position();
  invoke(&mut session, "half_down")?;
  session.split(
    super::super::Axis::Columns,
    None,
    super::super::Place::Before,
  )?;
  assert_eq!(session.tab().panes.len(), 2);
  assert!(session.tab().panes.iter().all(|app| app.manual.is_some()));
  let help = session.app().id;
  invoke(&mut session, "h")?;
  assert_eq!(session.tabs.len(), 2);
  assert_eq!(session.app().id, help);
  session.close()?;
  assert_eq!(session.app().id, original);
  assert_eq!(session.app().navigation_position(), position);
  session.finish()?;
  assert_eq!(session.state.recent.len(), 1);
  assert!(session.state.marks.is_empty());
  Ok(())
}

#[test]
fn failed_lookup_keeps_document() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first"], "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  let id = session.app().id;
  {
    let query = "unknown-topic";
    session.app().command(&format!("help {query}"))?;
    assert!(session.settle().is_err());
    assert_eq!(session.app().id, id);
    assert_eq!(session.tabs.len(), 1);
  }
  assert!(session.help.pending.is_none());
  Ok(())
}

#[test]
fn refresh_replaces_all_split_generations() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "config")?;
  session.split(
    super::super::Axis::Columns,
    None,
    super::super::Place::Before,
  )?;
  for app in &mut session.tab().panes {
    app.marks.insert('a', vpdf_model::text::Position::default());
  }
  session.set_theme(vpdf_model::theme::Theme::Nord);
  wait(&mut session);
  for app in &session.tabs[session.active].panes {
    assert_eq!(app.document_generation, 1);
    assert!(app.marks.is_empty());
    assert!(app.manual.is_some());
  }
  assert_eq!(session.state.recent, [] as [crate::state::Entry; 0]);
  Ok(())
}

#[test]
fn failures_and_closed_sources_preserve_focus() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first"], "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  let id = session.app().id;
  let position = session.app().navigation_position();
  session.app().command("help config")?;
  session.settle()?;
  session.help_result(
    Err(anyhow::anyhow!("Help generation failed")),
    &mut std::io::sink(),
  );
  assert_eq!(session.app().error.as_deref(), Some("Help generation failed"));
  assert_eq!(session.app().id, id);
  assert_eq!(session.app().navigation_position(), position);
  session.help = Help::default();
  invoke(&mut session, "config")?;
  let help = session.active;
  session.close_tab(0)?;
  assert!(session.active < help);
  session.close()?;
  assert!(session.home.is_some());
  Ok(())
}

#[test]
fn prompt_keeps_mode_and_literal_completion() -> Result<()> {
  use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
  use vpdf_ui::prompt::History;
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first"], "")?;
  for frontend in [
    crate::raster::Frontend::Terminal,
    crate::raster::Frontend::Native(vpdf_pdf::jobs::Wake::none()),
  ] {
    let mut session = Session { frontend, ..Session::new(None) };
    session.open(&path, true, cells(120, 40))?;
    let mut history = History::default();
    session.app().reader.enter_cursor();
    let invocation = session.app().help_invocation();
    for key in vpdf_ui::keys::parse(":").unwrap() {
      session.app().key(key, &mut history)?;
    }
    session.app().paste("help h", &mut history)?;
    session
      .app()
      .key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut history)?;
    let prompt = session.app().overlays.prompt().unwrap();
    assert_eq!(prompt.invocation(), Some(&invocation));
    let names = prompt.menu().unwrap().items();
    assert!(names.iter().any(|name| name == "h"));
    assert!(names.iter().any(|name| name == "half_down"));
    assert!(names.iter().all(|name| name.starts_with('h')));
    session.app().cancel_prompt()?;
    for key in vpdf_ui::keys::parse(":").unwrap() {
      session.app().key(key, &mut history)?;
    }
    session.app().paste("help h", &mut history)?;
    session
      .app()
      .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
    let Some(super::super::Request::Help { invocation: captured, .. }) =
      &session.app().request
    else {
      panic!("Help request missing")
    };
    assert_eq!(*captured, invocation);
    session.settle()?;
    wait(&mut session);
    assert!(session.app().manual.is_some());
    assert_eq!(session.help.reference.as_ref().unwrap().invocation, invocation);
  }
  Ok(())
}

#[test]
fn newer_focus_discards_pending_open() -> Result<()> {
  let root = tempfile::tempdir()?;
  let mut session = Session::new(None);
  for name in ["one", "two"] {
    let path = root.path().join(format!("{name}.pdf"));
    crate::fixtures::pdf(&path, &[name], "")?;
    session.open(&path, true, cells(120, 40))?;
  }
  let invocation = session.app().help_invocation();
  session.request_help("config", invocation, cells(120, 40))?;
  session.activate(0)?;
  let id = session.app().id;
  wait(&mut session);
  assert_eq!(session.app().id, id);
  assert_eq!(session.tabs.len(), 2);
  Ok(())
}

#[test]
fn closing_during_refresh_keeps_home() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "config")?;
  session.set_theme(vpdf_model::theme::Theme::Nord);
  assert!(session.help.pending.is_some());
  session.close()?;
  wait(&mut session);
  assert!(session.tabs.is_empty() && session.home.is_some());
  assert!(!session.quit);
  Ok(())
}

#[test]
fn normal_refresh_preserves_field_and_discards_crops() -> Result<()> {
  use vpdf_model::document::Target;
  use vpdf_model::geometry::Crop;
  let mut session = Session::new(None);
  invoke(&mut session, "config")?;
  let app = session.app();
  let document = app.manual.clone().unwrap();
  let (page, y) = document
    .anchors
    .iter()
    .find_map(|anchor| {
      if anchor.topic == "config"
        && anchor.name.is_some()
        && let Target::Point { page, y, .. } = anchor.target
      {
        return Some((page, y));
      }
      None
    })
    .unwrap();
  app.reader.normal();
  app.restore_at((page, (0.0, y + 1.0)), (0.0, 0.0));
  let logical = app.help_location().unwrap();
  assert_ne!(logical, Location::topic(TopicId::parse("config")?));
  app.layout.restore(1.0, 0, Crop([1, 0, 0, 0]));
  let mut crops = vpdf_model::geometry::Crops::default();
  crops.insert(page, Crop([1, 10, 10, 10]));
  app.layout.resolve_crops(std::sync::Arc::new(crops));
  session.set_theme(vpdf_model::theme::Theme::Nord);
  wait(&mut session);
  let app = session.app();
  assert!(app.layout.crops().is_empty());
  assert_eq!(app.help_location(), Some(logical));
  let deadline = Instant::now() + Duration::from_secs(10);
  while !app.crop_updates()? {
    assert!(Instant::now() < deadline, "Help crop scan timed out");
    std::thread::sleep(Duration::from_millis(2));
  }
  assert!(app.layout.crops().is_empty());
  Ok(())
}

#[test]
fn configured_command_preserves_operator_context() -> Result<()> {
  use vpdf_ui::keys::{Action, Input};
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  let config = root.path().join("config.toml");
  crate::fixtures::pdf(&path, &["first"], "")?;
  std::fs::write(
    &config,
    "[keybindings.operator]\n'<F2>' = { command = 'help p' }\n",
  )?;
  let loaded =
    crate::config::Options { config: Some(config), ..Default::default() }
      .load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let mut session = Session::configured(None, loaded);
  session.open(&path, true, cells(120, 40))?;
  session.app().action(Input::plain(Action::Yank))?;
  let invocation = session.app().help_invocation();
  assert_eq!(invocation.context(), vpdf_ui::bindings::Context::Operator);
  for &key in vpdf_ui::bindings::Sequence::parse("<F2>")?.keys() {
    session.type_key(key)?;
  }
  session.settle()?;
  wait(&mut session);
  assert_eq!(session.help.reference.as_ref().unwrap().invocation, invocation);
  assert_eq!(session.app().help_location().unwrap().id().as_str(), "yank.path");
  Ok(())
}

#[test]
fn remaps_unbinding_and_path_policy() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  let config = root.path().join("config.toml");
  crate::fixtures::pdf(&path, &["first"], "")?;
  std::fs::write(
    &config,
    concat!(
      "[keybindings.normal]\n",
      "h = { action = 'tab.next' }\n",
      "q = { unbind = true }\n"
    ),
  )?;
  let loaded =
    crate::config::Options { config: Some(config), ..Default::default() }
      .load();
  assert!(loaded.warning.is_none(), "{:?}", loaded.warning);
  let mut session = Session::configured(None, loaded);
  session.open(&path, true, cells(120, 40))?;
  invoke(&mut session, "h")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    builtin_topic("h", vpdf_ui::bindings::Context::Normal)
  );
  invoke(&mut session, "q")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    builtin_topic("q", vpdf_ui::bindings::Context::Normal)
  );
  let app = session.app();
  assert!(!app.info().contains(&app.path.display().to_string()));
  assert!(app.command("reload").is_err());
  assert!(app.region_operation(vpdf_ui::region::Operation::Save).is_err());
  app.request = Some(super::super::Request::SetMark('A'));
  assert!(session.settle().is_err());
  assert!(session.state.marks.is_empty());
  Ok(())
}

#[test]
fn help_open_releases_home_preview() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("source.pdf");
  crate::fixtures::pdf(&path, &["first"], "")?;
  let entry = crate::state::tests::entry(&path.to_string_lossy(), 0, 0);
  let screen = cells(120, 40);
  let mut home = Home::new(&[entry], screen, &vpdf_pdf::jobs::Wake::none());
  let mut drawn = Vec::new();
  home.draw(&mut drawn)?;
  let deadline = Instant::now() + Duration::from_secs(10);
  while !home.updates() {
    assert!(Instant::now() < deadline, "Home thumbnail timed out");
    std::thread::sleep(Duration::from_millis(2));
  }
  drawn.clear();
  home.draw(&mut drawn)?;
  assert!(String::from_utf8_lossy(&drawn).contains("a=t,"));
  let mut session = Session { home: Some(home), ..Session::new(None) };
  session.request_help("introduction", Invocation::Home, screen)?;
  let mut cleared = Vec::new();
  while session.help.pending.is_some() {
    session.help_updates(&mut cleared);
    assert!(Instant::now() < deadline, "Help open timed out");
    std::thread::sleep(Duration::from_millis(2));
  }
  assert!(session.home.is_none());
  assert!(String::from_utf8_lossy(&cleared).contains("a=d,"));
  Ok(())
}

#[test]
fn help_close_clears_images() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "config")?;
  let screen = session.app().screen;
  let id = session.app().id;
  let tile = vpdf_terminal::payload::Tile {
    key: vpdf_model::tile::TileKey {
      width: 1,
      height: 1,
      scale: 1024,
      ..Default::default()
    },
    payload: vpdf_terminal::payload::Encoder::default().encode(&[255; 3])?,
    render_ms: 0.0,
  };
  let mut drawn = Vec::new();
  session.graphics.begin(&mut drawn)?;
  session.graphics.upload(id, &tile, &mut drawn)?;
  session.graphics.picture(
    1,
    (1, 1, &[255; 3]),
    vpdf_model::text::Bounds::placed((0.0, 0.0), (1.0, 1.0)),
    screen,
    0,
    &mut drawn,
  )?;
  session.graphics.end(&mut drawn)?;
  assert!(String::from_utf8_lossy(&drawn).contains("a=t,"));
  session.app().command("quit")?;
  session.settle()?;
  assert!(session.tabs.is_empty());
  let mut home = session.home.take().expect("Returned start screen");
  let mut returned = Vec::new();
  session.draw_home(&mut home, &mut returned)?;
  let output = String::from_utf8_lossy(&returned);
  assert_eq!(output.matches("a=d,d=I,").count(), 2);
  assert!(output.find("a=d,d=I,").unwrap() < output.find("RECENT").unwrap());
  returned.clear();
  session.draw_home(&mut home, &mut returned)?;
  assert!(!String::from_utf8_lossy(&returned).contains("a=d,d=I,"));
  Ok(())
}

#[test]
fn help_beside_preserves_book() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(
    &path,
    &["First", "Second", "Third", "Fourth", "Fifth"],
    "",
  )?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  session.app().command("2")?;
  let original = session.app().id;
  let position = session.app().navigation_position();
  invoke(&mut session, "navigation")?;
  session.activate(0)?;
  session.app().command("help! search")?;
  session.settle()?;
  wait(&mut session);
  assert_eq!(session.active, 0);
  assert_eq!(session.tabs.len(), 2);
  assert_eq!(session.tab().panes.len(), 2);
  assert!(session.app().manual.is_some());
  assert_eq!(session.app().help_location().unwrap().id().as_str(), "search");
  session.close_pane()?;
  assert_eq!(session.app().id, original);
  assert_eq!(session.app().navigation_position(), position);
  Ok(())
}

#[test]
fn ambiguity_offers_persistent_choices() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "sample")?;
  let original = session.app().navigation_position();
  session.app().command("help panel.")?;
  session.settle()?;
  assert_eq!(session.app().navigation_position(), original);
  let prompt = session.app().overlays.prompt().unwrap();
  assert_eq!(prompt.text(), "help panel.");
  let menu = prompt.menu().unwrap();
  assert!(menu.items().len() > 1);
  assert!(menu.labels().iter().all(|label| label.contains("[Action]")));
  let topic = menu.items()[0].clone();
  session.app().key(
    vpdf_model::input::KeyEvent::new(
      vpdf_model::input::KeyCode::Tab,
      vpdf_model::input::KeyModifiers::NONE,
    ),
    &mut vpdf_ui::prompt::History::default(),
  )?;
  assert!(session.app().overlays.prompt().unwrap().text().ends_with(&topic));
  Ok(())
}

#[test]
fn explicit_context_retains_document() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "sample")?;
  session.app().command("help search:<Tab>")?;
  session.settle()?;
  wait(&mut session);
  assert_eq!(
    session.help.reference.as_ref().unwrap().invocation.context(),
    vpdf_ui::bindings::Context::Search
  );
  Ok(())
}

#[test]
fn refresh_retains_wrapped_position() -> Result<()> {
  for cursor in [false, true] {
    let root = tempfile::tempdir()?;
    let config = root.path().join("config.toml");
    std::fs::write(&config, "")?;
    let loaded =
      crate::config::Options { config: Some(config), ..Default::default() }
        .load();
    let mut session = Session::configured(None, loaded);
    invoke(&mut session, "introduction")?;
    session.resize(cells(36, 8))?;
    session.app().command("text")?;
    for _ in 0..20 {
      if session.app().text_view.top().glyph > 0 {
        break;
      }
      session.app().reading_view().edge_scroll(1)?;
    }
    let position = session.app().text_view.top();
    assert!(position.glyph > 0);
    if cursor {
      session.app().land_at(position)?;
      session.app().reader.enter_cursor();
    }
    for reload in [false, true] {
      if reload {
        session.reload_config();
      } else {
        session.set_theme(vpdf_model::theme::Theme::Nord);
      }
      wait(&mut session);
      let app = session.app();
      assert!(app.error.is_none(), "{:?}", app.error);
      assert_eq!(app.reader.viewport_mode(), !cursor);
      if cursor {
        assert_eq!(app.reader.cursor, position);
      } else {
        assert_eq!(app.text_view.top(), position);
      }
    }
  }
  Ok(())
}

#[test]
fn help_reuses_correct_placement() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["Book"], "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  let book = session.app().id;
  session.app().command("help! search")?;
  session.settle()?;
  wait(&mut session);
  let split = session.app().id;
  invoke(&mut session, "navigation")?;
  assert_eq!(session.app().id, split);
  assert_eq!(session.tabs.len(), 1);
  session.focus(0, 0)?;
  invoke(&mut session, "zoom")?;
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.active, 0);
  assert_eq!(session.app().id, split);
  assert_eq!(session.app().help_location().unwrap().id().as_str(), "zoom");
  session.focus(0, 0)?;
  invoke(&mut session, "search")?;
  assert_eq!(session.app().id, split);
  session.focus(0, 0)?;
  session.resize(cells(40, 12))?;
  assert!(session.app().screen.columns <= 20);
  session.app().command("help! zoom")?;
  session.settle()?;
  wait(&mut session);
  assert!(session.app().error.is_none(), "{:?}", session.app().error);
  assert_eq!(session.app().id, split);
  assert_eq!(session.tabs[0].panes.len(), 2);
  assert_eq!(session.tabs[0].panes[0].id, book);
  session.set_theme(vpdf_model::theme::Theme::Nord);
  wait(&mut session);
  assert!(session.app().error.is_none(), "{:?}", session.app().error);
  assert_eq!(session.app().id, split);
  session.close_pane()?;
  assert_eq!(session.app().id, book);
  Ok(())
}

#[test]
fn refresh_retains_page_separator() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "introduction")?;
  session.resize(cells(36, 8))?;
  let app = session.app();
  app.command("text")?;
  let page = vpdf_model::text::PageId(0);
  let position = vpdf_model::text::Position {
    page,
    line: app.doc.text(page)?.lines.len(),
    glyph: 0,
  };
  let frame = vpdf_ui::text_view::Frame {
    screen: app.screen,
    at: app.at,
    left: app.left,
  };
  app
    .text_view
    .bind(crate::pdf_text::PdfText(&mut app.doc), frame)
    .origin(position)?;
  assert_eq!(app.text_view.top(), position);
  let document = app.manual.clone().unwrap();
  session.set_theme(vpdf_model::theme::Theme::Nord);
  assert!(session.help.pending.is_some());
  wait(&mut session);
  let app = session.app();
  assert!(app.error.is_none(), "{:?}", app.error);
  assert!(!std::sync::Arc::ptr_eq(app.manual.as_ref().unwrap(), &document));
  assert_eq!(app.text_view.top(), position);
  Ok(())
}

#[test]
fn pending_help_checks_size() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["Book"], "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  let book = session.app().id;
  invoke(&mut session, "navigation")?;
  session.app().set_mark('a')?;
  let manual = session.app().manual.clone().unwrap();
  let reference = session.help.reference.clone();
  session.activate(0)?;
  session.app().reader.enter_cursor();
  session.app().command("help! search")?;
  session.settle()?;
  assert!(session.help.pending.is_some());
  session.resize(cells(20, 10))?;
  wait(&mut session);
  assert_eq!(session.app().error.as_deref(), Some("Not enough room"));
  assert_eq!(session.tab().panes.len(), 1);
  assert_eq!(session.app().id, book);
  assert_eq!(session.help.reference, reference);
  assert!(session.tabs[1].panes[0].marks.contains_key(&'a'));
  assert!(std::sync::Arc::ptr_eq(
    session.tabs[1].panes[0].manual.as_ref().unwrap(),
    &manual
  ));
  Ok(())
}

#[test]
fn shorthand_lookup_and_completion() -> Result<()> {
  use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["Book"], "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(120, 40))?;
  let mut history = vpdf_ui::prompt::History::default();
  for key in vpdf_ui::keys::parse(":").unwrap() {
    session.app().key(key, &mut history)?;
  }
  session.app().paste("h c-d", &mut history)?;
  session
    .app()
    .key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), &mut history)?;
  assert_eq!(session.app().overlays.prompt().unwrap().text(), "h c-d");
  session
    .app()
    .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  session.settle()?;
  wait(&mut session);
  assert!(session.app().error.is_none(), "{:?}", session.app().error);
  assert_eq!(session.app().help_location().unwrap().id().as_str(), "half_down");
  Ok(())
}

#[test]
fn lookup_recovery_keeps_position() -> Result<()> {
  let mut session = Session::new(None);
  invoke(&mut session, "sample")?;
  let id = session.app().id;
  let position = session.app().navigation_position();
  session.app().command("help serach")?;
  let error = session.settle().unwrap_err().to_string();
  assert!(error.contains("serach"), "{error}");
  assert!(error.contains(":help search"), "{error}");
  assert_eq!(session.app().id, id);
  assert_eq!(session.app().navigation_position(), position);
  invoke(&mut session, "'reading.zoom'")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    "configuration"
  );
  invoke(&mut session, "helphelp")?;
  assert_eq!(
    session.app().help_location().unwrap().id().as_str(),
    "introduction"
  );
  invoke(&mut session, "quickref")?;
  assert_eq!(session.app().help_location().unwrap().id().as_str(), "keys");
  Ok(())
}

fn builtin_topic(query: &str, context: vpdf_ui::bindings::Context) -> String {
  let registry = Registry::new(
    vpdf_ui::topics::authored(),
    vpdf_ui::bindings::Bindings::default().snapshot(),
  )
  .unwrap();
  match registry.lookup(query, context) {
    Lookup::Found(Destination::Action(id)) => id.as_str().into(),
    other => panic!("Unexpected built-in lookup: {other:?}"),
  }
}
