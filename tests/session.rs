use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::{Request, TabMove};
use vpdf::layout::Screen;
use vpdf::session::Session;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn tabs_share_graphics_and_close_in_order() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new("", false, None);
  for name in ["book.pdf", "paper.pdf", "form.pdf"] {
    session.open(&root.join(name), true, screen)?;
  }
  assert_eq!((session.tabs.len(), session.active), (3, 2));
  session.open(&root.join("book.pdf"), true, screen)?;
  assert_eq!((session.tabs.len(), session.active), (3, 0));
  let names = &session.app().tabline.names;
  assert_eq!(names, &["book.pdf", "paper.pdf", "form.pdf"]);
  let steps = [
    (TabMove::Previous(1), 2),
    (TabMove::Next(2), 1),
    (TabMove::Go(9), 2),
    (TabMove::Go(1), 0),
  ];
  for (step, expected) in steps {
    session.app().request = Some(Request::Tab(step));
    session.settle()?;
    assert_eq!(session.active, expected, "{step:?}");
  }
  let id = session.app().id;
  assert_eq!(session.app().graphics.doc, id);
  session.app().request =
    Some(Request::Open { path: root.join("ocr.pdf"), tab: false });
  session.settle()?;
  assert_eq!(session.app().name(), "ocr.pdf");
  assert_ne!(session.app().graphics.doc, id);
  session.app().command("tabclose")?;
  session.settle()?;
  assert_eq!((session.tabs.len(), session.active), (2, 0));
  assert_eq!(session.app().name(), "paper.pdf");
  session.app().command("qa")?;
  session.settle()?;
  assert!(session.quit);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn reopening_restores_position_and_recent_order() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let store = directory.path().join("state.json");
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new("", false, Some(store.clone()));
  session.open(&root.join("paper.pdf"), true, screen)?;
  session.open(&root.join("book.pdf"), true, screen)?;
  session.app().command("page 39")?;
  session.app().layout.zoom = 1.5;
  session.app().command("q")?;
  session.settle()?;
  session.app().command("q")?;
  session.settle()?;
  assert!(session.tabs.is_empty());
  let mut session = Session::new("", false, Some(store));
  let names: Vec<_> = session
    .state
    .recent
    .iter()
    .map(|entry| entry.path.file_name().unwrap().to_owned())
    .collect();
  assert_eq!(names, ["paper.pdf", "book.pdf"], "closed last, listed first");
  session.open(&root.join("book.pdf"), true, screen)?;
  assert_eq!(session.app().reader.cursor.page.0, 38);
  assert!((session.app().layout.zoom - 1.5).abs() < f32::EPSILON);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn marks_persist_and_cross_documents() -> Result<()> {
  use vpdf::keys::{Action, Input};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let store = directory.path().join("state.json");
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let press = |session: &mut Session, action| -> Result<()> {
    let input = Input { action, count: 1, explicit: false, register: '"' };
    session.app().action(input)?;
    session.settle()
  };
  let mut session = Session::new("", false, Some(store.clone()));
  session.open(&root.join("book.pdf"), true, screen)?;
  session.app().command("page 30")?;
  press(&mut session, Action::Mark('a'))?;
  session.app().command("page 40")?;
  press(&mut session, Action::Mark('B'))?;
  session.open(&root.join("paper.pdf"), true, screen)?;
  press(&mut session, Action::GoMark('B', true))?;
  assert_eq!(session.app().name(), "book.pdf");
  assert_eq!(session.app().reader.cursor.page.0, 39);
  session.app().command("qa")?;
  session.settle()?;
  for _ in 0..2 {
    session.app().command("q")?;
    session.settle()?;
  }
  let mut session = Session::new("", false, Some(store));
  session.open(&root.join("book.pdf"), true, screen)?;
  press(&mut session, Action::GoMark('a', false))?;
  assert_eq!(session.app().reader.cursor.page.0, 29);
  assert!(press(&mut session, Action::GoMark('z', false)).is_err());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn ctrl_w_splits_moves_and_closes_panes() -> Result<()> {
  use vpdf::app::Window;
  use vpdf::keys::{Action, Input};
  use vpdf::split::{Axis, Direction};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 121,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut session = Session::new("", false, None);
  session.open(&root.join("book.pdf"), true, screen)?;
  session.app().command("page 30")?;
  let window = |session: &mut Session, command| -> Result<()> {
    let input = Input {
      action: Action::Window(command),
      count: 1,
      explicit: false,
      register: '"',
    };
    session.app().action(input)?;
    session.settle()
  };
  window(&mut session, Window::Split(Axis::Columns))?;
  assert_eq!(session.tab().panes.len(), 2);
  let left = session.app().id;
  assert_eq!(session.app().reader.cursor.page.0, 29, "split keeps the view");
  assert_eq!((session.app().at.column, session.app().screen.columns), (0, 60));
  session.app().request = Some(Request::Window {
    command: Window::Split(Axis::Rows),
    count: 1,
    explicit: false,
    file: Some(root.join("paper.pdf")),
  });
  session.settle()?;
  assert_eq!(session.app().name(), "paper.pdf");
  assert_eq!(session.app().screen.rows, 20);
  window(&mut session, Window::Focus(Direction::Right))?;
  assert_eq!(session.app().at.column, 61);
  window(&mut session, Window::Focus(Direction::Left))?;
  window(&mut session, Window::Focus(Direction::Down))?;
  assert_eq!(session.app().id, left);
  assert!(session.app().focused);
  window(&mut session, Window::Resize(Axis::Rows, 5))?;
  assert_eq!(session.app().screen.rows, 25);
  window(&mut session, Window::Close)?;
  assert_eq!(session.tab().panes.len(), 2);
  window(&mut session, Window::ToTab)?;
  assert_eq!((session.tabs.len(), session.tab().panes.len()), (2, 1));
  session.app().command("q")?;
  session.settle()?;
  assert_eq!(session.tabs.len(), 1);
  window(&mut session, Window::Only)?;
  assert_eq!(session.tab().panes.len(), 1);
  assert_eq!(session.app().screen.columns, 121);
  Ok(())
}
