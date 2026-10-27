use super::*;
use crate::screen::cells;
use vpdf_model::text::PageId;
use vpdf_pdf::jobs::Wake;
use vpdf_ui::split::{Axis, Place};

const PAGES: &[&str] = &["1", "2", "3", "4", "5", "6", "7", "8"];

fn session(root: &Path) -> Result<Session> {
  let mut session = Session::new(Some(root.join("state.json")));
  session.frontend = crate::raster::Frontend::Native(Wake::none());
  session.resize(cells(80, 24))?;
  Ok(session)
}

fn pdfs(root: &Path, names: &[&str]) -> Result<Vec<PathBuf>> {
  names
    .iter()
    .map(|name| {
      let path = root.join(name);
      crate::fixtures::pdf(&path, PAGES, "")?;
      Ok(std::fs::canonicalize(path)?)
    })
    .collect()
}

/// Tabs a, b | b at page 4 (focused), c; b's tab active.
fn saved(root: &Path) -> Result<(Vec<PathBuf>, Layout)> {
  let paths = pdfs(root, &["a.pdf", "b.pdf", "c.pdf"])?;
  let mut session = session(root)?;
  for path in &paths {
    session.open(path, true, cells(80, 24))?;
  }
  session.activate(1)?;
  session.split(Axis::Columns, None, Place::After)?;
  session.app().reading_view().open_page(PageId(3))?;
  session.settle()?;
  let layout = session.layout();
  session.finish()?;
  Ok((paths, layout))
}

fn restoring(root: &Path, layout: Layout) -> Result<Session> {
  let mut session = session(root)?;
  session.windows.restoring =
    Some(Restoring { sized: true, ..Restoring::new(layout) });
  Ok(session)
}

fn restore_all(session: &mut Session) {
  while session.restore_next(cells(80, 24)) {}
  assert!(session.windows.restoring.is_none());
}

fn paths(session: &Session) -> Vec<Vec<PathBuf>> {
  session
    .tabs
    .iter()
    .map(|tab| tab.panes.iter().map(|app| app.path.clone()).collect())
    .collect()
}

#[test]
fn layouts_reopen_active_tab_first() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (files, layout) = saved(root.path())?;
  let json = serde_json::to_vec(&layout)?;
  assert_eq!(serde_json::from_slice::<Layout>(&json)?, layout);
  let mut session = restoring(root.path(), layout.clone())?;
  assert!(session.restore_next(cells(80, 24)));
  assert_eq!(paths(&session), [vec![files[1].clone(), files[1].clone()]]);
  assert!(session.home.is_none());
  // Unopened tabs keep their saved places.
  assert_eq!(session.layout(), layout);
  restore_all(&mut session);
  assert_eq!(
    paths(&session),
    [
      vec![files[0].clone()],
      vec![files[1].clone(), files[1].clone()],
      vec![files[2].clone()],
    ]
  );
  assert_eq!(session.active, 1);
  let tab = &session.tabs[1];
  assert_eq!(tab.focus, 1);
  assert_eq!(
    tab.order(),
    tab.panes.iter().map(|app| app.id).collect::<Vec<_>>()
  );
  let pages: Vec<_> =
    tab.panes.iter().map(|app| app.navigation_position().page).collect();
  assert_eq!(pages, [PageId(0), PageId(3)]);
  assert!(session.app().error.is_none());
  assert_eq!(session.layout(), layout);
  Ok(())
}

#[test]
fn closing_mid_restore_keeps_running() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (files, layout) = saved(root.path())?;
  let mut session = restoring(root.path(), layout)?;
  session.restore_next(cells(80, 24));
  session.close()?;
  super::super::quit_when_closed(&mut session);
  assert!(!session.quit);
  restore_all(&mut session);
  assert_eq!(paths(&session), [vec![files[0].clone()], vec![files[2].clone()]]);
  Ok(())
}

#[test]
fn unavailable_files_are_reported() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (files, layout) = saved(root.path())?;
  std::fs::remove_file(&files[0])?;
  let locked = super::super::tests::protected(&files[2])?;
  std::fs::copy(locked.path(), &files[2])?;
  let mut session = restoring(root.path(), layout)?;
  restore_all(&mut session);
  assert_eq!(paths(&session), [vec![files[1].clone(), files[1].clone()]]);
  assert_eq!(
    session.app().error.as_deref(),
    Some("Not reopened: a.pdf (missing), c.pdf (locked)")
  );
  // Missing files keep their tab for later launches; locked ones do not.
  let mut saved = session.layout();
  for missed in 1..ATTEMPTS {
    let kept: Vec<_> = saved.tabs.iter().map(|tab| tab.missed).collect();
    assert_eq!(kept, [0, missed]);
    assert_eq!(saved.tabs[1].panes[0].path, files[0]);
    let mut session = restoring(root.path(), saved)?;
    restore_all(&mut session);
    saved = session.layout();
  }
  assert_eq!(saved.tabs.len(), 1);
  Ok(())
}

#[test]
fn newer_reading_state_wins() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (files, layout) = saved(root.path())?;
  let mut other = session(root.path())?;
  other.open(&files[1], true, cells(80, 24))?;
  other.app().reading_view().open_page(PageId(6))?;
  other.settle()?;
  let moved = other.app().navigation_position();
  other.finish()?;
  let mut session = restoring(root.path(), layout.clone())?;
  restore_all(&mut session);
  let views: Vec<_> = session.tabs[1]
    .panes
    .iter()
    .map(crate::app::App::navigation_position)
    .collect();
  // The first pane follows reading state; the split keeps its view.
  assert_eq!(views, [moved, layout.tabs[1].panes[1].cursor]);
  Ok(())
}

#[test]
fn split_views_survive_relaunch() -> Result<()> {
  let root = tempfile::tempdir()?;
  let [path] = pdfs(root.path(), &["s.pdf"])?.try_into().unwrap();
  let mut first = session(root.path())?;
  first.open(&path, true, cells(80, 24))?;
  first.split(Axis::Columns, None, Place::After)?;
  first.app().reading_view().open_page(PageId(5))?;
  first.settle()?;
  let layout = first.layout();
  first.finish()?;
  let mut session = restoring(root.path(), layout.clone())?;
  restore_all(&mut session);
  let views: Vec<_> = session.tabs[0]
    .panes
    .iter()
    .map(crate::app::App::navigation_position)
    .collect();
  let saved: Vec<_> = layout.tabs[0].panes.iter().map(|p| p.cursor).collect();
  assert_ne!(saved[0], saved[1]);
  assert_eq!(views, saved);
  Ok(())
}

#[test]
fn unopened_active_tab_keeps_focus() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (_, layout) = saved(root.path())?;
  let session = restoring(root.path(), layout.clone())?;
  assert_eq!(session.layout().active, layout.active);
  Ok(())
}

#[test]
fn explicit_opens_keep_focus() -> Result<()> {
  let root = tempfile::tempdir()?;
  let (files, layout) = saved(root.path())?;
  let [explicit] = pdfs(root.path(), &["x.pdf"])?.try_into().unwrap();
  for (opened, expected, active) in [
    (&explicit, vec![&files[0], &files[1], &files[2], &explicit], 3),
    // The saved tab of an opened document is skipped but holds its place.
    (&files[2], vec![&files[0], &files[1], &files[2]], 2),
    (&files[0], vec![&files[0], &files[1], &files[2]], 0),
  ] {
    let mut session = restoring(root.path(), layout.clone())?;
    session.open(opened, true, cells(80, 24))?;
    restore_all(&mut session);
    let firsts: Vec<_> =
      session.tabs.iter().map(|tab| &tab.panes[0].path).collect();
    assert_eq!(firsts, expected);
    assert_eq!(session.active, active);
    assert_eq!(&session.app().path, opened);
  }
  Ok(())
}

#[test]
fn stores_are_claimed_once() -> Result<()> {
  let root = tempfile::tempdir()?;
  let state = root.path().join("vpdf/state.json");
  let mut store = Store::claim(&state).unwrap();
  assert!(Store::claim(&state).is_none());
  assert!(store.load().is_none());
  let (_, layout) = saved(root.path())?;
  store.save(layout.clone())?;
  let file = state.with_file_name("native-session.json");
  let modified = std::fs::metadata(&file)?.modified()?;
  store.save(layout.clone())?;
  assert_eq!(std::fs::metadata(&file)?.modified()?, modified);
  drop(store);
  let mut store = Store::claim(&state).unwrap();
  assert_eq!(store.load(), Some(layout.clone()));
  for corrupt in [
    b"{".to_vec(),
    serde_json::to_vec(&Layout { version: VERSION + 1, ..layout.clone() })?,
  ] {
    std::fs::write(&file, corrupt)?;
    assert!(store.load().is_none());
  }
  // Tabs whose shape misses or repeats panes are dropped.
  let mut broken = layout.clone();
  broken.tabs[0].panes.clear();
  broken.tabs[2].shape = Shape::Split {
    axis: Axis::Rows,
    ratio: 0.5,
    first: Box::new(Shape::Pane(0)),
    second: Box::new(Shape::Pane(0)),
  };
  std::fs::write(&file, serde_json::to_vec(&broken)?)?;
  let loaded = store.load().unwrap();
  assert_eq!(loaded.tabs, layout.tabs[1..2]);
  // The active tab keeps focus after dropped tabs.
  assert_eq!(loaded.active, 0);
  Ok(())
}

#[test]
fn reopening_follows_settings() {
  use crate::config::Restore;
  for (setting, system, expected) in [
    ("system", true, true),
    ("system", false, false),
    ("always", false, true),
    ("never", true, false),
  ] {
    let config = crate::config::parse(
      Path::new("settings.toml"),
      &format!("[reading]\nrestore_session = '{setting}'"),
    );
    assert_eq!(
      config.settings.restore_session(),
      setting.parse::<Restore>().unwrap()
    );
    let launch = super::super::Launch {
      config,
      system_reopen: system,
      ..super::super::Launch::default()
    };
    assert_eq!(launch.reopen(), expected, "{setting} {system}");
  }
}

#[test]
fn bridges_save_on_close_and_reopen() -> Result<()> {
  use super::super::{Bridge, Input, Launch};
  let root = tempfile::tempdir()?;
  let files = pdfs(root.path(), &["a.pdf", "b.pdf"])?;
  let store = Some(root.path().join("state.json"));
  let launch = |references: Vec<PathBuf>| {
    let (wake, wakes) = crate::fixtures::wake();
    let config = crate::config::parse(
      Path::new("settings.toml"),
      "[reading]\nrestore_session = 'always'",
    );
    let references = references.into_iter().map(Into::into).collect();
    let launch =
      Launch { config, references, store: store.clone(), system_reopen: false };
    Ok::<_, anyhow::Error>((
      Bridge::configured(launch, move || wake.wake())?,
      wakes,
    ))
  };
  let (mut bridge, _) = launch(files.clone())?;
  bridge.send(Input::Fullscreen(true));
  bridge.close()?;
  let (mut bridge, wakes) = launch(Vec::new())?;
  assert!(bridge.fullscreen());
  bridge.send(Input::Resize(cells(80, 24)));
  super::super::tests::wait(&bridge, &wakes, |frame| {
    frame.ui.path.as_deref() == Some(files[0].as_path())
  })?;
  bridge.close()?;
  let saved: Layout = serde_json::from_slice(&std::fs::read(
    root.path().join("native-session.json"),
  )?)?;
  let opened: Vec<_> =
    saved.tabs.iter().map(|tab| tab.panes[0].path.clone()).collect();
  assert_eq!(opened, files);
  assert_eq!((saved.active, saved.fullscreen), (0, true));
  Ok(())
}
