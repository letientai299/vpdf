use super::history::Revision;
use super::tests::entry;
use super::*;

#[test]
fn forget_survives_reload_and_stale_saves() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut state = State::default();
  let mut saved = entry("/a.pdf", 4, 10);
  saved.marks.insert('a', saved.cursor);
  saved.zoom = 2.0;
  saved.rotation = 90;
  saved.crop = Crop([1, 2, 3, 4]);
  state.opened(saved.clone());
  state.marks.insert('A', Mark::new(saved.path.clone(), saved.cursor));
  state.save(&file, &[], &[])?;
  let mut older = State::load(&file);
  state.forget(&[saved.path.clone()]);
  state.save(&file, &[], &[])?;
  saved.used = u64::MAX;
  older.remember(saved.clone());
  older.save(&file, &[], &[])?;
  let loaded = State::load(&file);
  assert_eq!(loaded.recent, []);
  assert!(loaded.marks.contains_key(&'A'));
  state.refresh_history(Some(&file));
  state.opened(entry("/a.pdf", 0, 11));
  state.save(&file, &[], &[])?;
  older.save(&file, &[], &[])?;
  let loaded = State::load(&file);
  assert_eq!(loaded.recent[0].cursor.page.0, 0);
  assert!(loaded.recent[0].marks.is_empty());
  assert_eq!(loaded.recent[0].zoom, 1.0);
  assert_eq!(loaded.recent[0].rotation, 0);
  assert_eq!(loaded.recent[0].crop, Crop::default());
  assert!(loaded.marks.contains_key(&'A'));
  Ok(())
}

#[test]
fn old_json_loads_and_forgets() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let old = serde_json::json!({"recent": [entry("/a.pdf", 2, 10)]});
  std::fs::write(&file, serde_json::to_vec(&old)?)?;
  let mut state = State::load(&file);
  assert_eq!(state.open_revision(Path::new("/a.pdf")), Visit::default());
  state.forget(&["/a.pdf".into()]);
  state.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent, []);
  Ok(())
}

#[test]
fn compacted_removals_refresh_old_baselines() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut state = State::default();
  state.opened(entry("/a.pdf", 2, 10));
  state.save(&file, &[], &[])?;
  let mut older = State::load(&file);
  state.forget(&["/a.pdf".into()]);
  for index in 0..=RECENT {
    state.forget(&[format!("/{index}.pdf").into()]);
  }
  state.opened(entry("/b.pdf", 3, 20));
  state.save(&file, &[], &[])?;
  let encoded = serde_json::to_value(&state)?;
  let forgotten = encoded["history"]["forgotten"].as_object().unwrap();
  assert!(forgotten.len() <= RECENT);
  assert!(state.history.floor > Revision::default());
  assert!(!forgotten.contains_key("/a.pdf"));
  older.recent[0].used = u64::MAX;
  older.save(&file, &[], &[])?;
  assert!(older.entry(Path::new("/a.pdf")).is_none());
  assert!(older.entry(Path::new("/b.pdf")).is_some());
  assert!(!older.remember(entry("/a.pdf", 9, u64::MAX)));
  older.refresh_history(Some(&file));
  older.opened(entry("/a.pdf", 0, 30));
  older.save(&file, &[], &[])?;
  state.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent.len(), 2);
  Ok(())
}

#[test]
fn failed_save_retains_removal_intent() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut state = State::default();
  state.opened(entry("/a.pdf", 1, 10));
  state.save(&file, &[], &[])?;
  let original = std::fs::read(&file)?;
  state.forget(&["/a.pdf".into()]);
  std::fs::remove_file(&file)?;
  std::fs::create_dir(&file)?;
  assert!(state.save(&file, &[], &[]).is_err());
  assert!(state.history.excludes(Path::new("/a.pdf")));
  std::fs::remove_dir(&file)?;
  std::fs::write(&file, original)?;
  state.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent, []);
  Ok(())
}

#[test]
fn simultaneous_writers_keep_all_operations() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let barrier = Arc::new(std::sync::Barrier::new(8));
  let writers: Vec<_> = (0..8)
    .map(|index| {
      let barrier = barrier.clone();
      let file = file.clone();
      thread::spawn(move || -> Result<()> {
        let mut state = State::default();
        state.opened(entry(&format!("/{index}.pdf"), index, index as u64));
        barrier.wait();
        state.save(&file, &[], &[])
      })
    })
    .collect();
  for writer in writers {
    writer.join().unwrap()?;
  }
  assert_eq!(State::load(&file).recent.len(), 8);
  Ok(())
}

#[test]
fn unsaved_opens_cannot_overtake_removals() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut state = State::default();
  state.opened(entry("/a.pdf", 1, 10));
  state.save(&file, &[], &[])?;
  let mut older = State::load(&file);
  for _ in 0..RECENT + 10 {
    older.opened(entry("/a.pdf", 2, u64::MAX));
  }
  state.forget(&["/a.pdf".into()]);
  state.save(&file, &[], &[])?;
  older.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent, []);
  state.opened(entry("/a.pdf", 0, 20));
  state.save(&file, &[], &[])?;
  older.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent[0].cursor.page.0, 0);
  Ok(())
}

/// A removed global mark stays removed through stale sessions' saves;
/// setting it again later wins.
#[test]
fn removed_marks_survive_stale_saves() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mark = |cursor: Position| Mark::new("/a.pdf".into(), cursor);
  let mut state = State::default();
  state.set_mark('A', mark(Position::default()));
  state.set_mark('B', mark(Position::default()));
  state.save(&file, &[], &[])?;
  let mut older = State::load(&file);
  state.remove_mark('A');
  state.save(&file, &[], &[])?;
  older.save(&file, &[], &[])?;
  let loaded = State::load(&file);
  assert!(!loaded.marks.contains_key(&'A'));
  assert!(loaded.marks.contains_key(&'B'));
  // The stale session sees the removal too.
  assert!(!older.marks.contains_key(&'A'));
  let mut later = mark(Position { line: 3, ..Position::default() });
  later.set += 1_000;
  older.set_mark('A', later);
  older.save(&file, &[], &[])?;
  state.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).marks[&'A'].cursor.line, 3);
  Ok(())
}
