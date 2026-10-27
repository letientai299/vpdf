use super::*;
use crate::screen::cells;
use std::ffi::OsStr;
use vpdf_model::reference::Reference;
use vpdf_ui::find::Phase;

fn reference(path: &Path, fragment: &str) -> Result<Reference> {
  Ok(Reference::parse(OsStr::new(&format!("{}{fragment}", path.display())))?)
}

pub(super) fn finish_search(app: &mut App) -> Result<()> {
  let deadline = Instant::now() + Duration::from_secs(5);
  while app.search.phase().running() {
    anyhow::ensure!(Instant::now() < deadline, "Reference search timed out");
    app.search_updates()?;
    std::thread::sleep(Duration::from_millis(5));
  }
  Ok(())
}

#[test]
fn references_restore_and_override() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("a b%.pdf");
  let store = root.path().join("state");
  crate::fixtures::pdf(&path, &["one", "two", "three"], "")?;
  let bytes = std::fs::read(&path)?;
  let screen = cells(100, 35);
  let mut session = Session::new(Some(store.clone()));
  session.open_reference(&reference(&path, "#page=3")?, true, screen)?;
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(2));
  session.finish()?;
  drop(session);
  let mut session = Session::new(Some(store.clone()));
  session.open_reference(&reference(&path, "")?, true, screen)?;
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(2));
  session.open_reference(&reference(&path, "#page=1")?, true, screen)?;
  assert_eq!(session.tabs.len(), 1);
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(0));
  let failed =
    session.open_reference(&reference(&path, "#page=99")?, true, screen);
  assert!(failed.is_err());
  assert!(
    session
      .app()
      .unwrap()
      .error
      .as_deref()
      .is_some_and(|error| error.contains("Page outside document"))
  );
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(0));
  session.finish()?;
  let canonical = std::fs::canonicalize(&path)?;
  let state = State::load(&store);
  assert!(state.entry(&canonical).is_some());
  assert_eq!(session.app().unwrap().path, canonical);
  assert_eq!(std::fs::read(&path)?, bytes);
  Ok(())
}

#[test]
fn ordered_object_targets_and_rectangles() -> Result<()> {
  let file = crate::reference_fixture::pdf(90)?;
  let bytes = std::fs::read(file.path())?;
  let mut session = Session::new(None);
  session.open_reference(
    &reference(
      file.path(),
      concat!(
        "#page=1&comment=note&nameddest=proof&structelem=%FF",
        "&highlight=5,25,10,30"
      ),
    )?,
    true,
    cells(100, 35),
  )?;
  let app = session.app().unwrap();
  assert_eq!(app.navigation_position().page, PageId(1));
  let rectangle = app.reference.rectangle.as_ref().unwrap();
  assert_eq!(rectangle.page, PageId(1));
  assert_eq!(
    rectangle.bounds,
    app.doc.highlight_bounds(PageId(1), [5.0, 25.0, 10.0, 30.0])?
  );
  let until = rectangle.until;
  assert!(
    !app.expire_reference(until.checked_sub(Duration::from_millis(1)).unwrap())
  );
  assert!(app.expire_reference(until));
  assert!(app.reference.rectangle.is_none());
  for fragment in
    ["#nameddest=missing", "#structelem=missing", "#comment=missing"]
  {
    assert!(
      session
        .open_reference(
          &reference(file.path(), fragment)?,
          true,
          cells(100, 35)
        )
        .is_err()
    );
  }
  session.app().unwrap().text_mode = true;
  let failed = session.app().unwrap().apply_reference(
    &reference(file.path(), "#highlight=5,25,10,30")?.operations,
  );
  assert!(
    failed.unwrap_err().to_string().contains("Highlight requires page view")
  );
  session
    .app()
    .unwrap()
    .apply_reference(&reference(file.path(), "#comment=note")?.operations)?;
  assert!(session.app().unwrap().reference.rectangle.is_none());
  assert_eq!(std::fs::read(file.path())?, bytes);
  Ok(())
}

#[test]
fn searches_preserve_operation_order() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("search.pdf");
  crate::fixtures::pdf(&path, &["one C++", "two", "three"], "")?;
  let mut session = Session::new(None);
  session.open_reference(
    &reference(
      &path,
      "#page=3&search=%22two%20one%22&highlight=5,25,10,30&page=2",
    )?,
    true,
    cells(100, 35),
  )?;
  finish_search(session.app().unwrap())?;
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(1));
  assert_eq!(
    session.app().unwrap().reference.rectangle.as_ref().unwrap().page,
    PageId(0)
  );
  session.open_reference(
    &reference(&path, "#search=%22two%22&search=%22C%2B%2B%22")?,
    true,
    cells(100, 35),
  )?;
  finish_search(session.app().unwrap())?;
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(0));
  assert_eq!(session.app().unwrap().search.hits().len(), 1);
  assert!(matches!(session.app().unwrap().search.phase(), Phase::Complete));
  session.open_reference(
    &reference(&path, "#search=%22absent%22&page=3")?,
    true,
    cells(100, 35),
  )?;
  assert!(
    finish_search(session.app().unwrap())
      .unwrap_err()
      .to_string()
      .contains("Reference search found no matches")
  );
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(0));
  Ok(())
}

#[test]
fn reference_search_starts_first() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("dense.pdf");
  let words = "w ".repeat(1000);
  crate::fixtures::pdf(&path, &[words.as_str(); 20], "/Rotate 90")?;
  let data = std::fs::read_to_string(&path)?
    .replace("/MediaBox[0 0 300 200]", "/MediaBox[0 0 30000 200]");
  std::fs::write(&path, data)?;
  let mut session = Session::new(None);
  session.open_reference(
    &reference(&path, "#page=11&search=%22w%22")?,
    true,
    cells(100, 35),
  )?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(10));
  finish_search(session.app().unwrap())?;
  assert_eq!(session.app().unwrap().search.hits().len(), 10_000);
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(0));
  Ok(())
}

#[test]
fn newer_requests_cancel_reference_work() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("search.pdf");
  crate::fixtures::pdf(&path, &["one", "two", "three"], "")?;
  let screen = cells(100, 35);
  let mut session = Session::new(None);
  session.open_reference(
    &reference(&path, "#search=%22one%22&page=3")?,
    true,
    screen,
  )?;
  session.open_reference(&reference(&path, "#page=2")?, true, screen)?;
  for _ in 0..20 {
    session.app().unwrap().search_updates()?;
  }
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(1));
  session.open_reference(
    &reference(&path, "#search=%22one%22&page=3")?,
    true,
    screen,
  )?;
  session.app().unwrap().start_search("two", false, false)?;
  finish_search(session.app().unwrap())?;
  assert_eq!(session.app().unwrap().navigation_position().page, PageId(1));
  Ok(())
}

#[test]
fn explicit_targets_cancel_search() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("search.pdf");
  crate::fixtures::pdf(&path, &["one", "two", "three"], "/Rotate 90")?;
  let screen = cells(100, 35);
  let mut session = Session::new(None);
  session.open(&path, true, screen)?;
  session.app().unwrap().start_search("two", false, false)?;
  session.open_reference(&reference(&path, "#page=3")?, true, screen)?;
  finish_search(session.app().unwrap())?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(2));
  session.app().unwrap().start_search("one", false, false)?;
  finish_search(session.app().unwrap())?;
  session.open_reference(&reference(&path, "#page=3")?, true, screen)?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(2));
  assert!(matches!(session.app().unwrap().search.phase(), Phase::Complete));
  assert_eq!(session.app().unwrap().search.hits().len(), 1);
  Ok(())
}

#[test]
fn os_paths_preserve_hashes() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf#page=2");
  crate::fixtures::pdf(&path, &["one"], "")?;
  let mut session = Session::new(None);
  session.open(&path, true, cells(100, 35))?;
  assert_eq!(session.app().unwrap().path, std::fs::canonicalize(&path)?);
  assert!(
    session
      .open_reference(&reference(&path, "")?, true, cells(100, 35))
      .is_err()
  );
  Ok(())
}

#[test]
fn reference_rectangles_project_to_scene() -> Result<()> {
  use vpdf_model::scene::Op;
  for rotation in [0, 90, 180, 270] {
    let file = crate::reference_fixture::pdf(rotation)?;
    let mut session = Session::new(None);
    let screen = cells(100, 35);
    session.open_reference(
      &reference(file.path(), "#highlight=5,25,10,30")?,
      true,
      screen,
    )?;
    let app = session.app().unwrap();
    let bounds = app.reference.rectangle.as_ref().unwrap().bounds;
    let expected = app.layout.project(PageId(0), bounds, app.reader.scroll);
    let mut scene = Vec::new();
    app.scene_marks(&mut scene, screen)?;
    let rectangles: Vec<_> = scene
      .iter()
      .filter_map(|op| match op {
        Op::Fill(paint) if paint.color() == vpdf_ui::theme::MATCH => {
          Some(paint.rect())
        }
        _ => None,
      })
      .collect();
    let expected =
      vpdf_model::geometry::clip(expected, screen.bounds()).unwrap();
    assert_eq!(rectangles.len(), 1);
    let actual = rectangles[0];
    for (actual, expected) in [
      (actual.x0, expected.x0),
      (actual.x1, expected.x1),
      (actual.y0, expected.y0),
      (actual.y1, expected.y1),
    ] {
      assert!(
        (actual - expected).abs() <= 1.0,
        "{rotation}: {actual} vs {expected}"
      );
    }
  }
  Ok(())
}

#[test]
fn selected_page_is_independent_of_viewport() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("landscape.pdf");
  crate::fixtures::pdf(&path, &["one", "two", "three"], "")?;
  let mut session = Session::new(None);
  session.open_reference(
    &reference(&path, "#page=3&highlight=5,25,10,30")?,
    true,
    crate::screen::pixels(100, 40, 10, 20),
  )?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(2));
  assert_eq!(
    session.app().unwrap().reference.rectangle.as_ref().unwrap().page,
    PageId(2)
  );
  session.open_reference(
    &reference(&path, "#search=%22one%22&page=3")?,
    true,
    cells(100, 35),
  )?;
  session.app().unwrap().reload_with("")?;
  assert!(!session.app().unwrap().reference_pending());
  assert!(session.app().unwrap().reference.rectangle.is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified corpus"]
fn real_reference_targets() -> Result<()> {
  let corpus = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for name in ["paper", "book", "scan"] {
    let path = corpus.join(format!("{name}.pdf"));
    let bytes = std::fs::read(&path)?;
    let mut session = Session::new(None);
    let screen = cells(120, 40);
    session.open_reference(
      &reference(&path, "#page=1&highlight=5,25,10,30")?,
      true,
      screen,
    )?;
    assert_eq!(session.app().unwrap().reader.cursor.page, PageId(0));
    let last = session.app().unwrap().doc.pages.len();
    session.open_reference(
      &reference(&path, &format!("#page={last}&highlight=5,25,10,30"))?,
      true,
      screen,
    )?;
    assert_eq!(session.app().unwrap().reader.cursor.page, PageId(last - 1));
    assert_eq!(
      session.app().unwrap().reference.rectangle.as_ref().unwrap().page,
      PageId(last - 1)
    );
    let mut scene = Vec::new();
    session.app().unwrap().scene_marks(&mut scene, screen)?;
    assert!(
      scene.iter().any(|op| matches!(op, vpdf_model::scene::Op::Fill(_)))
    );
    assert_eq!(std::fs::read(&path)?, bytes);
  }
  Ok(())
}

#[test]
fn interactive_references_keep_targets() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("book.pdf");
  crate::fixtures::pdf(&path, &["one", "two", "three"], "/Rotate 90")?;
  let screen = cells(100, 35);
  let mut session = Session::new(None);
  session.open(&path, true, screen)?;
  session.app().unwrap().command(&format!("edit {}#page=2", path.display()))?;
  session.settle()?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(1));
  session.app().unwrap().paste(
    &format!("file://{}#page=3", path.display()),
    &mut History::default(),
  )?;
  session.settle()?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(2));
  session.dropped(Target::Pdf(reference(&path, "#page=1")?))?;
  assert_eq!(session.app().unwrap().reader.cursor.page, PageId(0));
  assert!(crate::dnd::pdf(&format!("{}#page=0", path.display())).is_err());
  Ok(())
}

#[test]
fn rectangle_reveals_offscreen_area() -> Result<()> {
  let root = tempfile::tempdir()?;
  let path = root.path().join("tall.pdf");
  crate::fixtures::pdf(&path, &["one"], "/Rotate 90")?;
  let mut session = Session::new(None);
  let screen = cells(100, 35);
  session.open_reference(
    &reference(&path, "#highlight=5,25,240,260")?,
    true,
    screen,
  )?;
  let app = session.app().unwrap();
  let rectangle = app.reference.rectangle.as_ref().unwrap();
  let visible =
    app.layout.project(rectangle.page, rectangle.bounds, app.reader.scroll);
  assert!(app.reader.scroll > 0.0);
  assert!(visible.y0 >= 0.0 && visible.y1 <= app.layout.screen().height());
  Ok(())
}
