use super::*;
use crate::screen::cells;
use vpdf_pdf::jobs::Wake;

fn reader() -> Result<(tempfile::TempDir, Session, PathBuf)> {
  let root = tempfile::tempdir()?;
  let pdf = root.path().join("book.pdf");
  crate::fixtures::pdf(&pdf, &["first", "second"], "")?;
  let pdf = pdf.canonicalize()?;
  let mut session = Session::new(Some(root.path().join("state.json")));
  session.open(&pdf, true, cells(120, 40))?;
  Ok((root, session, pdf))
}

#[test]
fn batch_forget_writes_once_and_preserves_files() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  let bytes = std::fs::read(&pdf)?;
  session.app().marks.insert('a', Position::default());
  session.state.marks.insert('A', Mark::new(pdf.clone(), Position::default()));
  session.record();
  session.flush()?;
  let writes = session.state.writes;
  session.forget_recent(&[pdf.clone(), "/other.pdf".into()])?;
  assert_eq!(session.state.writes, writes + 1);
  assert!(!session.flush()?);
  assert_eq!(session.state.recent, []);
  assert!(session.app().marks.is_empty());
  assert!(session.state.marks.contains_key(&'A'));
  assert_eq!(std::fs::read(&pdf)?, bytes);
  session.record();
  session.finish()?;
  assert_eq!(State::load(session.store.as_deref().unwrap()).recent, []);
  let (mut snapshot, history) = session.latest.lock().unwrap().clone().unwrap();
  snapshot.save(
    session.store.as_deref().unwrap(),
    history.commands(),
    history.searches(),
  )?;
  assert_eq!(State::load(session.store.as_deref().unwrap()).recent, []);
  session.open(&pdf, true, cells(120, 40))?;
  assert_eq!(session.state.recent.len(), 1);
  assert!(session.state.recent[0].marks.is_empty());
  session.close()?;
  assert_eq!(State::load(session.store.as_deref().unwrap()).recent.len(), 1);
  Ok(())
}

#[test]
fn stale_panes_cannot_restore_marks_after_reopen() -> Result<()> {
  let (_root, mut old, pdf) = reader()?;
  old.app().marks.insert('a', Position::default());
  old.record();
  old.flush()?;
  let mut newer = Session::new(old.store.clone());
  newer.forget_recent(std::slice::from_ref(&pdf))?;
  newer.open(&pdf, true, cells(120, 40))?;
  newer.flush()?;
  old.record();
  old.finish()?;
  let loaded = State::load(old.store.as_deref().unwrap());
  assert_eq!(loaded.recent.len(), 1);
  assert!(loaded.recent[0].marks.is_empty());
  old.open(&pdf, true, cells(120, 40))?;
  assert!(old.app().marks.is_empty());
  old.finish()?;
  assert!(
    State::load(old.store.as_deref().unwrap()).recent[0].marks.is_empty()
  );
  Ok(())
}

#[test]
fn copied_panes_do_not_revive_forgotten_documents() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  session.forget_recent(&[pdf])?;
  session.split(Axis::Columns, None, Place::After)?;
  session.recent_tab(cells(120, 40))?;
  session.finish()?;
  assert_eq!(session.state.recent, []);
  Ok(())
}

#[test]
fn failed_removal_stays_dirty_and_retries() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  session.flush()?;
  let store = session.store.clone().unwrap();
  let original = std::fs::read(&store)?;
  std::fs::remove_file(&store)?;
  std::fs::create_dir(&store)?;
  assert!(
    session
      .forget_recent(&[pdf])
      .unwrap_err()
      .to_string()
      .starts_with("Recent removal not saved:")
  );
  assert!(session.unsaved);
  assert_eq!(session.state.recent, []);
  std::fs::remove_dir(&store)?;
  std::fs::write(&store, original)?;
  session.record();
  assert!(session.flush()?);
  assert!(!session.unsaved);
  assert_eq!(State::load(&store).recent, []);
  Ok(())
}

#[test]
fn failed_open_does_not_revive_history() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  session.close()?;
  session.forget_recent(std::slice::from_ref(&pdf))?;
  std::fs::write(&pdf, b"invalid PDF")?;
  assert!(session.open(&pdf, true, cells(120, 40)).is_err());
  session.finish()?;
  assert_eq!(session.state.recent, []);
  Ok(())
}

#[test]
fn global_mark_reopens_forgotten_path() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  session.state.marks.insert('A', Mark::new(pdf.clone(), Position::default()));
  session.forget_recent(&[pdf])?;
  session.app().request = Some(Request::GoMark('A', true));
  session.settle()?;
  session.flush()?;
  assert_eq!(session.state.recent.len(), 1);
  Ok(())
}

#[test]
fn removal_refreshes_home_and_all_recent_panels() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  let recent = session.state.recent.clone();
  session.home = Some(Home::new(&recent, cells(120, 40), &Wake::none(), true));
  session.split(Axis::Columns, None, Place::After)?;
  for app in session.panes_mut() {
    app.show_recent(&recent)?;
  }
  session.forget_recent(&[pdf])?;
  for app in session.panes() {
    assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  }
  let home = session.home.as_mut().unwrap();
  let mut scene = vpdf_model::scene::Recorder::new(|_| false);
  home.scene(&mut scene)?;
  assert!(scene.into_scene().iter().any(|op| {
    match op {
      vpdf_model::scene::Op::Text(text) => {
        text.as_str().contains(":edit <file>")
      }
      _ => false,
    }
  }));
  Ok(())
}

#[test]
fn removal_during_load_clears_restored_marks() -> Result<()> {
  let (_root, mut first, pdf) = reader()?;
  first.app().marks.insert('a', Position::default());
  first.record();
  first.flush()?;
  let mut opening = Session::new(first.store.clone());
  let screen = cells(120, 40);
  let app = opening.load(&pdf, "", screen)?;
  assert!(app.marks.contains_key(&'a'));
  first.forget_recent(std::slice::from_ref(&pdf))?;
  opening.insert_tab(app)?;
  opening.remember_open(0, 0);
  opening.flush()?;
  assert!(opening.app().marks.is_empty());
  assert_eq!(State::load(opening.store.as_deref().unwrap()).recent.len(), 1);
  Ok(())
}

#[test]
fn forgotten_panes_stay_forgotten_on_close() -> Result<()> {
  let (_root, mut session, pdf) = reader()?;
  session.forget_recent(&[pdf])?;
  session.close()?;
  assert_eq!(State::load(session.store.as_deref().unwrap()).recent, []);
  Ok(())
}

fn recent_key(
  session: &mut Session,
  code: vpdf_model::input::KeyCode,
) -> Result<()> {
  use super::interaction::Interaction;
  session.interact(Interaction::Key(KeyEvent::new(
    code,
    vpdf_model::input::KeyModifiers::NONE,
  )))?;
  Ok(())
}

#[test]
fn recent_removal_uses_session_in_both_frontends() -> Result<()> {
  use vpdf_model::input::KeyCode;
  for native in [false, true] {
    for new_tab in [false, true] {
      let (_root, mut session, pdf) = reader()?;
      if native {
        session.frontend = crate::raster::Frontend::Native(Wake::none());
        session.app().frontend = session.frontend.clone();
      }
      if new_tab {
        session.recent_tab(cells(120, 40))?;
      } else {
        let entries = session.state.recent.clone();
        session.app().show_recent(&entries)?;
      }
      recent_key(&mut session, KeyCode::Char(' '))?;
      recent_key(&mut session, KeyCode::Char('x'))?;
      let before = session.app().reader.cursor;
      session.interact(interaction::Interaction::Paste(":q".into()))?;
      recent_key(&mut session, KeyCode::Char('j'))?;
      assert_eq!(session.app().reader.cursor, before);
      assert!(session.app().panel.as_ref().unwrap().view.confirming());
      assert!(session.app().overlays.prompt().is_none());
      let writes = session.state.writes;
      recent_key(&mut session, KeyCode::Enter)?;
      assert_eq!(session.state.writes, writes + 1);
      assert_eq!(session.state.recent, []);
      assert!(pdf.exists());
      assert_eq!(session.history.commands(), [] as [String; 0]);
      let panel = &mut session.app().panel.as_mut().unwrap().view;
      assert!(panel.rows().is_empty());
      let mut scene = vpdf_model::scene::Recorder::new(|_| false);
      panel.scene(
        &mut scene,
        vpdf_ui::panel::recent::Frame {
          at: At::default(),
          width: 120,
          screen: cells(120, 40),
          now: 0,
        },
        false,
      )?;
      let text = scene
        .into_scene()
        .iter()
        .filter_map(|op| match op {
          vpdf_model::scene::Op::Text(text) => Some(text.as_str()),
          _ => None,
        })
        .collect::<String>();
      assert!(text.contains(":edit <file>"));
      if native {
        assert!(text.contains(":open"));
      }
    }
  }
  Ok(())
}

#[test]
fn escape_clears_before_closing_recent() -> Result<()> {
  use vpdf_model::input::KeyCode;
  let (_root, mut session, _pdf) = reader()?;
  let entries = session.state.recent.clone();
  session.app().show_recent(&entries)?;
  recent_key(&mut session, KeyCode::Char(' '))?;
  recent_key(&mut session, KeyCode::Esc)?;
  assert!(session.app().panel.is_some());
  recent_key(&mut session, KeyCode::Esc)?;
  assert!(session.app().panel.is_none());
  Ok(())
}

#[test]
fn recent_batch_saves_once() -> Result<()> {
  use vpdf_model::input::KeyCode;
  let (root, mut session, first) = reader()?;
  let second = root.path().join("second.pdf");
  crate::fixtures::pdf(&second, &["second"], "")?;
  session.open(&second, true, cells(120, 40))?;
  let entries = session.state.recent.clone();
  session.app().show_recent(&entries)?;
  for code in [
    KeyCode::Char(' '),
    KeyCode::Char('j'),
    KeyCode::Char(' '),
    KeyCode::Char('x'),
  ] {
    recent_key(&mut session, code)?;
  }
  let writes = session.state.writes;
  recent_key(&mut session, KeyCode::Enter)?;
  assert_eq!(session.state.writes, writes + 1);
  assert_eq!(session.state.recent, []);
  assert!(first.exists() && second.exists());
  assert_eq!(State::load(session.store.as_deref().unwrap()).recent, []);
  Ok(())
}

/// Marks list under Local and Global headings; `x` asks first, and the
/// confirm bar answers keys and clicks alike.
#[test]
fn marks_remove_by_key_and_click() -> Result<()> {
  use super::interaction::Interaction;
  use vpdf_model::input::{
    KeyCode, Mouse, MouseButton, MouseEvent, MouseEventKind,
  };
  let (_root, mut session, pdf) = reader()?;
  session.app().marks.insert('a', Position::default());
  let cursor = Position::default();
  session.state.set_mark('A', Mark::new(pdf, cursor));
  // Saved first, so removal must outlast the merge with disk.
  session.record();
  session.flush()?;
  session.app().request = Some(Request::Panel(PanelKind::Marks));
  session.settle()?;
  let shown =
    |session: &mut Session| -> Vec<String> {
      let rows = session.app().panel.as_ref().unwrap().view.rows();
      rows
        .iter()
        .map(|row| {
          if row.key.is_empty() { row.label.clone() } else { row.key.clone() }
        })
        .collect()
    };
  assert_eq!(shown(&mut session), ["Local", "’a", "Global", "’A"]);
  // A local mark, by key.
  session.app().panel.as_mut().unwrap().view.choose(1);
  recent_key(&mut session, KeyCode::Char('x'))?;
  assert!(session.app().confirming_removal());
  recent_key(&mut session, KeyCode::Char('y'))?;
  assert!(session.app().marks.is_empty());
  assert_eq!(shown(&mut session), ["Global", "’A"]);
  // A global mark: Cancel, then Remove, by click.
  for (column, removed) in [(22, false), (2, true)] {
    session.app().panel.as_mut().unwrap().view.choose(1);
    recent_key(&mut session, KeyCode::Char('x'))?;
    session.draw_focused(&mut Vec::new())?;
    let app = session.app();
    let row = app.at.row + app.screen.rows - 2;
    let column = app.at.column + column;
    let press =
      MouseEvent::new(MouseEventKind::Down(MouseButton::Left), column, row);
    session.interact(Interaction::Mouse(Mouse::from(press)))?;
    assert!(!session.app().confirming_removal(), "{removed}");
    assert_eq!(session.state.marks.is_empty(), removed);
  }
  assert!(session.app().panel.as_ref().unwrap().view.rows().is_empty());
  // Saves keep both removals.
  session.flush()?;
  let saved = State::load(session.store.as_deref().unwrap());
  assert!(saved.marks.is_empty());
  assert!(saved.recent.iter().all(|entry| entry.marks.is_empty()));
  Ok(())
}

/// Setting a mark shows it in an open Marks panel.
#[test]
fn marks_show_new_marks() -> Result<()> {
  let (_root, mut session, _pdf) = reader()?;
  session.app().request = Some(Request::Panel(PanelKind::Marks));
  session.settle()?;
  for name in ['a', 'A'] {
    session.app().set_mark(name)?;
    session.settle()?;
  }
  let rows = session.app().panel.as_ref().unwrap().view.rows();
  let keys: Vec<_> = rows.iter().map(|row| row.key.as_str()).collect();
  assert_eq!(keys, ["", "’a", "", "’A"]);
  Ok(())
}

#[test]
fn recent_ratios_survive_panel_reopening() -> Result<()> {
  let (_root, mut session, _pdf) = reader()?;
  session.show_recent()?;
  let preferences = serde_json::from_str(r#"{"beside":600,"below":500}"#)?;
  session
    .app()
    .panel
    .as_mut()
    .unwrap()
    .view
    .shelf_mut()
    .unwrap()
    .set_preferences(preferences);
  session.record_recent_layout();
  session.app().close_panel()?;
  session.show_recent()?;
  assert_eq!(
    session.app().panel.as_ref().unwrap().view.shelf().unwrap().preferences(),
    preferences
  );
  session.finish()?;
  assert_eq!(
    State::load(session.store.as_deref().unwrap()).recent_layout,
    preferences
  );
  Ok(())
}
