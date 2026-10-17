use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::{Request, TabMove, Window};
use vpdf::keys::{Action, Input};
use vpdf::layout::Screen;
use vpdf::session::Session;

/// Runs `action` in the focused pane and applies its request.
fn act(session: &mut Session, action: Action) -> Result<()> {
  let input = Input { action, count: 1, explicit: false, register: '"' };
  session.app().action(input)?;
  session.settle()
}

/// Page 30 of book.pdf split with `Ctrl-w v`: columns 0-59 hold the
/// focused new pane, 61-120 the first; row 39 is the status row.
fn split() -> Result<(Session, u32, u32)> {
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
  let right = session.app().id;
  act(&mut session, Action::Window(Window::Split(vpdf::split::Axis::Columns)))?;
  let left = session.app().id;
  Ok((session, left, right))
}

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
  session.open(&root.join("book.pdf"), true, screen)?;
  session.app().command("page 30")?;
  act(&mut session, Action::Mark('a'))?;
  session.app().command("page 40")?;
  act(&mut session, Action::Mark('B'))?;
  session.open(&root.join("paper.pdf"), true, screen)?;
  act(&mut session, Action::GoMark('B', true))?;
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
  act(&mut session, Action::GoMark('a', false))?;
  assert_eq!(session.app().reader.cursor.page.0, 29);
  assert!(act(&mut session, Action::GoMark('z', false)).is_err());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn ctrl_w_splits_moves_and_closes_panes() -> Result<()> {
  use vpdf::split::{Axis, Direction};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let (mut session, left, _) = split()?;
  assert_eq!(session.tab().panes.len(), 2);
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
  act(&mut session, Action::Window(Window::Focus(Direction::Right)))?;
  assert_eq!(session.app().at.column, 61);
  act(&mut session, Action::Window(Window::Focus(Direction::Left)))?;
  act(&mut session, Action::Window(Window::Focus(Direction::Down)))?;
  assert_eq!(session.app().id, left);
  assert!(session.app().focused);
  act(&mut session, Action::Window(Window::Resize(Axis::Rows, 5)))?;
  assert_eq!(session.app().screen.rows, 25);
  act(&mut session, Action::Window(Window::Close))?;
  assert_eq!(session.tab().panes.len(), 2);
  act(&mut session, Action::Window(Window::ToTab))?;
  assert_eq!((session.tabs.len(), session.tab().panes.len()), (2, 1));
  session.app().command("q")?;
  session.settle()?;
  assert_eq!(session.tabs.len(), 1);
  act(&mut session, Action::Window(Window::Only))?;
  assert_eq!(session.tab().panes.len(), 1);
  assert_eq!(session.app().screen.columns, 121);
  Ok(())
}

mod pointer {
  use super::*;
  use MouseButton::Left;
  use MouseEventKind::{Down, Drag, ScrollDown, Up};
  use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use vpdf::app::App;
  use vpdf::keys::PanelKind;

  fn mouse(
    session: &mut Session,
    kind: MouseEventKind,
    column: u16,
    row: u16,
  ) -> Result<()> {
    let modifiers = KeyModifiers::NONE;
    session.mouse(MouseEvent { kind, column, row, modifiers })
  }

  fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
  }

  fn pane(session: &mut Session, id: u32) -> &mut App {
    session.tab().panes.iter_mut().find(|app| app.id == id).unwrap()
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn drag_across_the_separator_stays_with_the_press() -> Result<()> {
    let (mut session, left, right) = split()?;
    assert_eq!(session.app().id, left);
    let cursor = pane(&mut session, right).reader.cursor;
    mouse(&mut session, Down(Left), 30, 10)?;
    mouse(&mut session, Drag(Left), 100, 10)?;
    mouse(&mut session, Drag(Left), 100, 39)?;
    mouse(&mut session, Up(Left), 100, 39)?;
    assert_eq!(session.app().id, left, "drag and release keep focus");
    assert!(session.app().reader.selection.is_some());
    let other = pane(&mut session, right);
    assert_eq!(other.reader.cursor, cursor);
    assert!(other.reader.selection.is_none());
    assert!(other.animation.is_none());

    act(&mut session, Action::Cancel)?;
    act(&mut session, Action::Yank)?;
    assert!(session.app().keys.feed(key(KeyCode::Char('g'))).is_none());
    mouse(&mut session, ScrollDown, 90, 10)?;
    assert_eq!(session.app().id, left, "the wheel leaves focus");
    assert!(pane(&mut session, right).animation.is_some());
    assert!(session.app().reader.operator.is_none(), "wheel drops `y`");
    assert_eq!(session.app().keys.awaiting(), None, "wheel drops `g`");
    act(&mut session, Action::Yank)?;
    mouse(&mut session, Down(Left), 30, 10)?;
    assert!(session.app().reader.operator.is_none(), "press drops `y`");

    act(&mut session, Action::Search(false))?;
    session.app().keys.feed(key(KeyCode::Char('g')));
    session.app().hints = Some(vpdf::hints::Hints {
      items: Vec::new(),
      typed: String::new(),
      copy: false,
    });
    mouse(&mut session, Down(Left), 90, 10)?;
    assert_eq!(session.app().id, right, "a press focuses");
    let old = pane(&mut session, left);
    assert!(old.prompt.is_none() && old.hints.is_none());
    assert!(old.keys.awaiting().is_none());
    assert!(old.search_origin.is_none(), "the search was aborted");

    act(&mut session, Action::Yank)?;
    act(&mut session, Action::Window(Window::Next))?;
    assert_eq!(session.app().id, left);
    let old = pane(&mut session, right);
    assert!(old.reader.operator.is_none(), "`Ctrl-w` drops `y`");
    Ok(())
  }

  /// A split whose focused left pane shows an unfocused outline;
  /// returns the session and the panel width.
  fn outline() -> Result<(Session, u16)> {
    let (mut session, _, _) = split()?;
    act(&mut session, Action::Panel(PanelKind::Outline))?;
    act(&mut session, Action::Cancel)?;
    let width = session.app().left;
    assert!(width > 0);
    Ok((session, width))
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn only_presses_move_panel_focus() -> Result<()> {
    let (mut session, width) = outline()?;
    let panel =
      |session: &mut Session| session.app().panel.as_ref().unwrap().focused;
    mouse(&mut session, ScrollDown, 2, 5)?;
    assert!(!panel(&mut session), "the wheel leaves the panel unfocused");
    mouse(&mut session, Down(Left), 2, 3)?;
    mouse(&mut session, Drag(Left), 40, 10)?;
    mouse(&mut session, Up(Left), 40, 10)?;
    assert!(panel(&mut session), "a press focuses the panel");
    assert!(session.app().reader.selection.is_none(), "no drag from panel");
    mouse(&mut session, Down(Left), width + 5, 10)?;
    mouse(&mut session, Up(Left), width + 5, 10)?;
    assert!(!panel(&mut session), "a page press unfocuses the panel");
    let cursor = session.app().reader.cursor;
    mouse(&mut session, Down(Left), 40, 39)?;
    mouse(&mut session, Drag(Left), 40, 10)?;
    mouse(&mut session, Up(Left), 40, 10)?;
    assert_eq!(session.app().reader.cursor, cursor, "status row is inert");
    assert!(session.app().reader.selection.is_none());
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn clue_and_menu_rows_hide_the_page() -> Result<()> {
    let (mut session, _) = outline()?;
    let cursor = session.app().reader.cursor;
    let second = std::time::Duration::from_secs(1);
    for kind in [Down(Left), ScrollDown] {
      session.app().keys.feed(key(KeyCode::Char('g')));
      session.app().awaiting = std::time::Instant::now().checked_sub(second);
      assert!(session.app().clue_due());
      mouse(&mut session, kind, 40, 38)?;
      let app = session.app();
      assert_eq!(app.reader.cursor, cursor, "{kind:?} on clues");
      assert!(app.animation.is_none(), "{kind:?} on clues scrolls");
      assert!(app.clues.is_none(), "{kind:?} closes clues");
    }
    let mut history = vpdf::prompt::History::default();
    menu(&mut session, &mut history)?;
    mouse(&mut session, Down(Left), 40, 38)?;
    assert_eq!(session.app().reader.cursor, cursor, "menu rows are inert");
    assert!(session.app().prompt.is_some());
    session.app().key(key(KeyCode::Esc), &mut history)?;
    assert_eq!(session.app().menu_rows(), 0, "Esc closes the menu");
    menu(&mut session, &mut history)?;
    let left = session.app().id;
    mouse(&mut session, Down(Left), 90, 10)?;
    let old = pane(&mut session, left);
    assert!(old.prompt.is_none() && old.menu_rows() == 0, "blur closes it");
    Ok(())
  }

  /// Opens `:` with its completion menu.
  fn menu(
    session: &mut Session,
    history: &mut vpdf::prompt::History,
  ) -> Result<()> {
    act(session, Action::Command)?;
    for code in [KeyCode::Char('t'), KeyCode::Tab] {
      session.app().key(key(code), history)?;
    }
    assert!(session.app().menu_rows() > 0);
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn overview_takes_only_thumbnail_presses() -> Result<()> {
    let (mut session, width) = outline()?;
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Down(Left), 40, 39)?;
    assert!(session.app().overview.is_some(), "status row keeps overview");
    mouse(&mut session, Down(Left), 40, 38)?;
    assert!(session.app().overview.is_some(), "below the grid is inert");
    mouse(&mut session, Down(Left), width + 5, 2)?;
    assert!(session.app().overview.is_none(), "a thumbnail opens its page");
    mouse(&mut session, Drag(Left), 40, 10)?;
    mouse(&mut session, Up(Left), 40, 10)?;
    assert!(session.app().reader.selection.is_none(), "no drag from overview");

    mouse(&mut session, Down(Left), width + 5, 10)?;
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Drag(Left), 40, 20)?;
    mouse(&mut session, Up(Left), 40, 20)?;
    assert!(session.app().reader.selection.is_none(), "no drag under `gp`");
    let cursor = session.app().reader.cursor;
    mouse(&mut session, Down(Left), 2, 3)?;
    assert!(session.app().overview.is_none(), "a row shows its page");
    assert_ne!(session.app().reader.cursor, cursor);
    // The overview takes every key but `:`, so help opens first.
    act(&mut session, Action::Panel(PanelKind::Help))?;
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Down(Left), 2, 3)?;
    assert!(session.app().overview.is_some(), "help rows keep overview");
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn scrolls_in_text_mode_drop_a_pending_yank() -> Result<()> {
    let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
    let screen = Screen {
      columns: 120,
      rows: 40,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    };
    let mut session = Session::new("", true, None);
    session.open(&root.join("book.pdf"), true, screen)?;
    session.app().command("page 30")?;
    act(&mut session, Action::Yank)?;
    mouse(&mut session, ScrollDown, 30, 10)?;
    act(&mut session, Action::Yank)?;
    act(&mut session, Action::Rows(1))?;
    let reader = &session.app().reader;
    assert!(reader.operator.is_none() && reader.pending_yank.is_none());
    Ok(())
  }
}

/// Resizing lays out the panes and the start screen.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn resize_lays_out_panes_and_start_screen() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let small = Screen { columns: 100, rows: 30, ..screen };
  let mut session = Session::new("", false, None);
  session.open(&root.join("paper.pdf"), true, screen)?;
  session.resize(small)?;
  let app = session.app();
  assert_eq!((app.screen.columns, app.screen.rows), (100, 30));
  assert_eq!((app.layout.screen.columns, app.layout.screen.rows), (100, 30));
  session.app().command("q")?;
  session.settle()?;
  session.home = Some(vpdf::home::Home::new(&[], screen));
  session.resize(small)?;
  let home = session.home.as_ref().unwrap();
  assert_eq!((home.screen.columns, home.screen.rows), (100, 30));
  Ok(())
}
