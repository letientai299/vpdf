use super::*;
use crate::screen::cells;
use crate::state::Sidebar;

/// Opens `pdf` in a new session on `store`; the restored sidebar.
fn reopen(store: &Path, pdf: &Path) -> Result<(Session, Option<PanelKind>)> {
  let mut session = Session::new(Some(store.to_path_buf()));
  session.open(pdf, true, cells(120, 40))?;
  let panel = session.app().unwrap().panel.as_ref();
  assert!(panel.is_none_or(|panel| !panel.view.focused()));
  let kind = panel.map(|panel| panel.view.kind());
  Ok((session, kind))
}

#[test]
fn sidebars_reopen_per_document() -> Result<()> {
  let root = tempfile::tempdir()?;
  let store = root.path().join("state.json");
  let [a, b] = ["a.pdf", "b.pdf"].map(|name| root.path().join(name));
  for pdf in [&a, &b] {
    crate::fixtures::pdf(pdf, &["first", "second"], "")?;
  }
  // (document, panel left open, reopened panel)
  let cases = [
    (&a, None, None),
    (&a, Some(PanelKind::Outline), Some(PanelKind::Outline)),
    (&b, Some(PanelKind::Info), Some(PanelKind::Info)),
    (&a, None, None),
    (&b, Some(PanelKind::Results), None),
    (&a, Some(PanelKind::Annotations), Some(PanelKind::Annotations)),
    (&b, Some(PanelKind::Jumps), None),
    (&a, Some(PanelKind::Version), None),
  ];
  let (_, kind) = reopen(&store, &a)?;
  assert_eq!(kind, None, "unsaved");
  let mut last = [None, None];
  for (pdf, open, want) in cases {
    let (mut session, _) = reopen(&store, pdf)?;
    session.app().unwrap().close_panel()?;
    if let Some(kind) = open {
      session.app().unwrap().open_panel(kind)?;
    }
    session.close()?;
    let (mut session, kind) = reopen(&store, pdf)?;
    assert_eq!(kind, want, "{pdf:?} {open:?}");
    last[usize::from(pdf == &b)] = want;
    let other = if pdf == &a { &b } else { &a };
    session.close()?;
    let (_, kind) = reopen(&store, other)?;
    assert_eq!(kind, last[usize::from(other == &b)], "{other:?}");
  }
  Ok(())
}

#[test]
fn marks_and_unknown_sidebars_restore() -> Result<()> {
  let root = tempfile::tempdir()?;
  let store = root.path().join("state.json");
  let pdf = root.path().join("a.pdf");
  crate::fixtures::pdf(&pdf, &["first"], "")?;
  let (mut session, _) = reopen(&store, &pdf)?;
  session.close()?;
  let mut state = State::load(&store);
  state.recent[0].sidebar = Some(Sidebar::Open(PanelKind::Marks));
  state.save(&store, &[], &[])?;
  let (mut session, kind) = reopen(&store, &pdf)?;
  assert_eq!(kind, Some(PanelKind::Marks));
  session.close()?;
  let text = std::fs::read_to_string(&store)?;
  assert!(text.contains(r#""sidebar":{"open":"marks"}"#), "{text}");
  let text = text.replace(r#"{"open":"marks"}"#, r#"{"open":"future"}"#);
  std::fs::write(&store, text)?;
  let state = State::load(&store);
  assert_eq!(state.recent.len(), 1);
  assert_eq!(state.recent[0].sidebar, None);
  assert_eq!(reopen(&store, &pdf)?.1, None);
  Ok(())
}

#[test]
fn splits_of_an_open_view_skip_the_sidebar() -> Result<()> {
  let root = tempfile::tempdir()?;
  let store = root.path().join("state.json");
  let pdf = root.path().join("a.pdf");
  crate::fixtures::pdf(&pdf, &["first"], "")?;
  let (mut session, _) = reopen(&store, &pdf)?;
  session.app().unwrap().open_panel(PanelKind::Info)?;
  session.close()?;
  let (mut session, kind) = reopen(&store, &pdf)?;
  assert_eq!(kind, Some(PanelKind::Info));
  session.split(Axis::Columns, None, Place::After)?;
  assert!(session.app().unwrap().panel.is_none());
  Ok(())
}

#[test]
fn results_note_follows_the_search() -> Result<()> {
  let root = tempfile::tempdir()?;
  let pdf = root.path().join("a.pdf");
  crate::fixtures::pdf(&pdf, &["first"], "")?;
  let (mut session, _) = reopen(&root.path().join("state.json"), &pdf)?;
  let app = session.app().unwrap();
  app.open_panel(PanelKind::Results)?;
  let query =
    vpdf_search::query::Query { text: "first".into(), ..Default::default() };
  app.start_query(&query)?;
  let view = |app: &App| {
    let view = &app.panel.as_ref().unwrap().view;
    (view.note(), view.explains_empty())
  };
  assert_eq!(view(app), ("loading", true));
  app.cancel_search();
  assert_eq!(view(app), ("", true));
  Ok(())
}
