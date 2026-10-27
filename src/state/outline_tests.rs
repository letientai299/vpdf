use super::tests::entry;
use super::*;
use vpdf_model::document::{Heading, Target};
use vpdf_model::text::PageId;
use vpdf_ui::keys::{Fold, PanelKind};
use vpdf_ui::panel::{Panel, Row};
use vpdf_ui::reading::{Outline, Preferences};

fn preferences(operation: Fold) -> Preferences {
  let headings: Vec<_> = [0, 1, 2, 0, 1]
    .into_iter()
    .map(|depth| Heading {
      title: "Duplicate".into(),
      depth,
      target: Target::Page(PageId(0)),
    })
    .collect();
  let mut outline = Outline::new(&headings);
  let mut panel = Panel::new(
    PanelKind::Outline,
    headings.iter().map(|heading| Row::outline(heading, "i".into())).collect(),
  );
  outline.fold(&mut panel, operation, 1);
  outline.preferences().unwrap().clone()
}

#[test]
fn outline_defaults_and_edits() -> Result<()> {
  let mut old = serde_json::to_value(entry("/a.pdf", 0, 1))?;
  old.as_object_mut().unwrap().remove("outline");
  let mut saved: Entry = serde_json::from_value(old)?;
  assert!(saved.outline.is_none());
  assert_eq!(
    serde_json::from_str::<Preferences>("{}")?,
    Preferences::default()
  );
  let mut state = State::default();
  assert!(state.remember(saved.clone()));
  assert!(!state.remember(saved.clone()));
  saved.outline = Some(preferences(Fold::CollapseAll));
  assert!(state.remember(saved.clone()));
  assert!(!state.remember(saved.clone()));
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  state.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent[0].outline, saved.outline);
  assert!(State::load(&file).recent[0].outline.is_some());
  Ok(())
}

#[test]
fn cursor_saves_preserve_folds() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut initial = entry("/a.pdf", 0, 1);
  initial.outline = Some(preferences(Fold::CollapseAll));
  let mut state = State::default();
  state.opened(initial.clone());
  state.save(&file, &[], &[])?;
  let mut older = State::load(&file);
  let mut explicit = initial.clone();
  explicit.outline = Some(preferences(Fold::ExpandAll));
  state.remember(explicit.clone());
  state.save(&file, &[], &[])?;
  initial.cursor.page = PageId(2);
  initial.used = u64::MAX;
  older.remember(initial.clone());
  older.save(&file, &[], &[])?;
  let saved = State::load(&file);
  assert_eq!(saved.recent[0].cursor.page, PageId(2));
  assert_eq!(saved.recent[0].outline, explicit.outline);
  // Idle panes cannot restore older folds.
  assert!(!older.remember(initial));
  assert_eq!(older.recent[0].outline, explicit.outline);
  // Older cursors cannot discard newer edits.
  explicit.outline = Some(preferences(Fold::CollapseAll));
  let latest = explicit.outline.clone();
  state.remember(explicit);
  state.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent[0].cursor.page, PageId(2));
  assert_eq!(State::load(&file).recent[0].outline, latest);
  Ok(())
}

#[test]
fn folds_follow_edit_order() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut first = State::default();
  first.opened(entry("/a.pdf", 0, 1));
  first.save(&file, &[], &[])?;
  let mut second = State::load(&file);
  first.recent[0].outline = Some(preferences(Fold::CollapseAll));
  second.recent[0].outline = Some(preferences(Fold::ExpandAll));
  let latest = second.recent[0].outline.clone();
  second.save(&file, &[], &[])?;
  first.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent[0].outline, latest);
  first.opened(entry("/b.pdf", 0, 1));
  first.save(&file, &[], &[])?;
  assert!(
    State::load(&file).entry(Path::new("/b.pdf")).unwrap().outline.is_none()
  );
  Ok(())
}

#[test]
fn reopening_discards_forgotten_folds() -> Result<()> {
  let root = tempfile::tempdir()?;
  let file = root.path().join("state.json");
  let mut state = State::default();
  let mut saved = entry("/a.pdf", 0, 1);
  saved.outline = Some(preferences(Fold::CollapseAll));
  state.opened(saved);
  state.save(&file, &[], &[])?;
  let mut older = State::load(&file);
  let mut unopened = State::load(&file);
  state.forget(&["/a.pdf".into()]);
  state.save(&file, &[], &[])?;
  older.recent[0].outline = Some(preferences(Fold::ExpandAll));
  older.save(&file, &[], &[])?;
  assert_eq!(State::load(&file).recent, []);
  state.opened(entry("/a.pdf", 0, 2));
  state.save(&file, &[], &[])?;
  older.save(&file, &[], &[])?;
  unopened.recent[0].outline = Some(preferences(Fold::ExpandAll));
  unopened.save(&file, &[], &[])?;
  assert!(State::load(&file).recent[0].outline.is_none());
  Ok(())
}

#[test]
fn older_panes_commit_folds() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("reading.pdf");
  crate::fixtures::pdf(&path, &["first", "second"], "")?;
  let mut app =
    App::with_screen(&path, "", false, crate::screen::cells(80, 24))?;
  let mut state = State::default();
  app.opened = state.opened(app.reading_entry());
  let mut newer = app.reading_entry();
  newer.cursor.page = PageId(1);
  state.opened(newer.clone());
  app.outline.restore_preferences(Some(preferences(Fold::CollapseAll)));
  assert!(state.remember_pane(&app));
  assert_eq!(state.recent[0].cursor, newer.cursor);
  assert_eq!(state.recent[0].outline.as_ref(), app.outline.preferences());
  assert!(!state.remember_pane(&app));
  state.forget(std::slice::from_ref(&path));
  state.opened(entry(path.to_str().unwrap(), 1, 2));
  app.outline.restore_preferences(Some(preferences(Fold::ExpandAll)));
  assert!(!state.remember_pane(&app));
  assert!(state.recent[0].outline.is_none());
  Ok(())
}

#[test]
fn reopening_resets_forgotten_preferences() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("reading.pdf");
  crate::fixtures::pdf(&path, &["body"], "")?;
  let path = path.canonicalize()?;
  let screen = crate::screen::cells(100, 40);
  let mut session = crate::session::Session::new(None);
  session.open(&path, true, screen)?;
  session
    .app()
    .unwrap()
    .outline
    .restore_preferences(Some(preferences(Fold::CollapseAll)));
  session.open(&path, true, screen)?;
  assert!(session.app().unwrap().outline.preferences().is_some());
  session.forget_recent(std::slice::from_ref(&path))?;
  session.open(&path, true, screen)?;
  assert!(session.app().unwrap().outline.preferences().is_none());
  Ok(())
}
