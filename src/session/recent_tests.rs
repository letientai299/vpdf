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
  session
    .state
    .marks
    .insert('A', Mark { path: pdf.clone(), cursor: Position::default() });
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
  session
    .state
    .marks
    .insert('A', Mark { path: pdf.clone(), cursor: Position::default() });
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
  session.home = Some(Home::new(&recent, cells(120, 40), &Wake::none()));
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
        text.as_str().contains("No recent documents")
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
