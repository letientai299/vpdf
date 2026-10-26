use crate::common;
use crate::harness::corpus;

use vpdf::Result;
use vpdf::app::Request;
use vpdf::session::Session;
use vpdf_ui::keys::{Action, Input};
use vpdf_ui::layout::Screen;
use vpdf_ui::window::{TabMove, Window};

/// Runs `action` in the focused pane and applies its request.
fn act(session: &mut Session, action: Action) -> Result<()> {
  let input = Input::plain(action);
  session.app().action(input)?;
  session.settle()
}

/// Page 30 of book.pdf split with `Ctrl-w v`: columns 0-59 hold the
/// focused new pane, 61-120 the first; row 39 is the status row.
fn split() -> Result<(Session, u32, u32)> {
  let screen = common::cells(121, 40);
  let mut session = crate::harness::session(&corpus("book.pdf")?, screen)?;
  session.app().command("30")?;
  let right = session.app().id;
  act(
    &mut session,
    Action::Window(Window::Split(vpdf_ui::split::Axis::Columns)),
  )?;
  let left = session.app().id;
  Ok((session, left, right))
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn tabs_share_graphics_and_close_in_order() -> Result<()> {
  let screen = common::cells(120, 50);
  let mut session = Session::new(None);
  for name in ["book.pdf", "paper.pdf", "form.pdf"] {
    session.open(&corpus(name)?, true, screen)?;
  }
  assert_eq!((session.tabs.len(), session.active), (3, 2));
  session.open(&corpus("book.pdf")?, true, screen)?;
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
  session.app().request =
    Some(Request::Open { path: corpus("ocr.pdf")?, tab: false });
  session.settle()?;
  assert_eq!(session.app().name(), "ocr.pdf");
  assert_ne!(session.app().id, id);
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
  let directory = tempfile::tempdir()?;
  let store = directory.path().join("state.json");
  let screen = common::cells(120, 50);
  let mut session = Session::new(Some(store.clone()));
  session.open(&corpus("paper.pdf")?, true, screen)?;
  session.open(&corpus("book.pdf")?, true, screen)?;
  session.app().command("39")?;
  session.app().layout.request_zoom(vpdf_ui::layout::Zoom::To(1.5));
  session.app().command("q")?;
  session.settle()?;
  session.app().command("q")?;
  session.settle()?;
  assert!(session.tabs.is_empty());
  let mut session = Session::new(Some(store));
  let names: Vec<_> = session
    .state
    .recent
    .iter()
    .map(|entry| entry.path.file_name().unwrap().to_owned())
    .collect();
  assert_eq!(names, ["paper.pdf", "book.pdf"], "closed last, listed first");
  session.open(&corpus("book.pdf")?, true, screen)?;
  assert_eq!(session.app().reader.cursor.page.0, 38);
  assert!((session.app().layout.zoom() - 1.5).abs() < f32::EPSILON);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn marks_persist_and_cross_documents() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let store = directory.path().join("state.json");
  let screen = common::cells(120, 50);
  let mut session = Session::new(Some(store.clone()));
  session.open(&corpus("book.pdf")?, true, screen)?;
  session.app().command("30")?;
  act(&mut session, Action::Mark('a'))?;
  session.app().command("40")?;
  act(&mut session, Action::Mark('B'))?;
  session.open(&corpus("paper.pdf")?, true, screen)?;
  act(&mut session, Action::GoMark('B', true))?;
  assert_eq!(session.app().name(), "book.pdf");
  assert_eq!(session.app().reader.cursor.page.0, 39);
  session.app().command("qa")?;
  session.settle()?;
  for _ in 0..2 {
    session.app().command("q")?;
    session.settle()?;
  }
  let mut session = Session::new(Some(store));
  session.open(&corpus("book.pdf")?, true, screen)?;
  act(&mut session, Action::GoMark('a', false))?;
  assert_eq!(session.app().reader.cursor.page.0, 29);
  assert!(act(&mut session, Action::GoMark('z', false)).is_err());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn ctrl_r_resizes_pane() -> Result<()> {
  let (mut session, left, right) = split()?;
  common::keys(session.app(), b"y")?;
  for (bytes, id, column, row, columns, rows) in [
    (b"5\x12l".as_slice(), left, 0, 0, 65, 40),
    (b"l", left, 0, 0, 66, 40),
    (b"l", left, 0, 0, 67, 40),
    (b"hq:", left, 0, 0, 66, 40),
    (b"\x1b[27u\x17l", right, 67, 0, 54, 40),
    (b"3\x12h", right, 64, 0, 57, 40),
    (b"h", right, 63, 0, 58, 40),
    (b"l", right, 64, 0, 57, 40),
  ] {
    common::keys(session.app(), bytes)?;
    session.settle()?;
    let app = session.app();
    assert_eq!(app.id, id);
    assert_eq!((app.at.column, app.at.row), (column, row));
    assert_eq!((app.screen.columns, app.screen.rows), (columns, rows));
  }
  common::keys(session.app(), b"\x1b[27u\x17s")?;
  session.settle()?;
  let top = session.app().id;
  for (bytes, id, row, rows) in [
    (b"4\x12j".as_slice(), top, 0, 24),
    (b"j", top, 0, 25),
    (b"k", top, 0, 24),
    (b"\x1b[27u\x17j", right, 24, 16),
    (b"2\x12k", right, 22, 18),
    (b"k", right, 21, 19),
    (b"j", right, 22, 18),
  ] {
    common::keys(session.app(), bytes)?;
    session.settle()?;
    let app = session.app();
    assert_eq!(app.id, id);
    assert_eq!((app.at.row, app.screen.rows), (row, rows));
    assert_eq!((app.at.column, app.screen.columns), (64, 57));
  }
  let mode = session.app().reader.mode();
  let cursor = session.app().reader.cursor;
  let mut status = Vec::new();
  session.app().status(&mut status, 0, None)?;
  assert!(String::from_utf8(status)?.contains("RESIZE"));
  common::keys(session.app(), b"\x1b[27u")?;
  assert!(!session.app().keys.resizing());
  assert_eq!(session.app().reader.mode(), mode);
  assert_eq!(session.app().reader.cursor, cursor);
  Ok(())
}

#[test]
fn resize_reverses_direction() -> Result<()> {
  use vpdf_ui::split::{Axis, Direction};

  let (_root, path) =
    common::temp_pdf(|path| common::text_pdf(path, &["Resize direction"]))?;
  for (axis, peer, forward, backward) in [
    (Axis::Columns, Direction::Right, b'l', b'h'),
    (Axis::Rows, Direction::Down, b'j', b'k'),
  ] {
    let mut session = crate::harness::session(&path, common::cells(121, 40))?;
    act(&mut session, Action::Window(Window::Split(axis)))?;
    for side in 0..2 {
      let before = session.app().screen;
      let at = session.app().at;
      common::keys(session.app(), &[0x12])?;
      for (first, second) in [(forward, backward), (backward, forward)] {
        common::keys(session.app(), &[b'3', first])?;
        session.settle()?;
        assert_ne!(session.app().screen, before);
        common::keys(session.app(), &[b'3', second])?;
        session.settle()?;
        assert_eq!(session.app().screen, before);
        assert_eq!(session.app().at, at);
        assert!(session.app().keys.resizing());
      }
      common::keys(session.app(), b"\x1b[27u")?;
      if side == 0 {
        act(&mut session, Action::Window(Window::Focus(peer)))?;
      }
    }
  }
  Ok(())
}

#[test]
fn ctrl_r_resizes_sidebar() -> Result<()> {
  use vpdf_ui::keys::PanelKind;
  use vpdf_ui::split::Axis;

  let (_root, path) =
    common::temp_pdf(|path| common::text_pdf(path, &["Sidebar resizing"]))?;
  let mut session = crate::harness::session(&path, common::cells(121, 40))?;
  act(&mut session, Action::Window(Window::Split(Axis::Columns)))?;
  act(&mut session, Action::Panel(PanelKind::Version))?;
  let screen = session.app().screen;
  let width = session.app().left;
  let cursor = session.app().reader.cursor;
  for (bytes, expected) in [
    (b"3\x12l".as_slice(), width + 3),
    (b"l", width + 4),
    (b"2h", width + 2),
    (b"jk", width + 2),
    (b"99999l", screen.columns - 12),
    (b"h", screen.columns - 13),
    (b"99999h", 8),
    (b"l", 9),
  ] {
    common::keys(session.app(), bytes)?;
    session.settle()?;
    let app = session.app();
    assert_eq!(app.left, expected, "{bytes:?}");
    assert_eq!(app.panel_width, Some(expected));
    assert_eq!(app.layout.screen().columns, screen.columns - expected);
    assert_eq!(app.screen, screen);
    assert_eq!(app.reader.cursor, cursor);
    assert!(app.keys.resizing());
    assert!(app.panel_focused());
  }
  common::keys(session.app(), b"\x1b[27u")?;
  assert!(!session.app().keys.resizing());
  assert!(session.app().panel_focused());
  act(&mut session, Action::LocalFocus)?;
  common::keys(session.app(), b"\x12l")?;
  session.settle()?;
  assert_eq!(session.app().screen.columns, screen.columns + 1);
  assert_eq!(session.app().left, 9);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn ctrl_w_splits_moves_and_closes_panes() -> Result<()> {
  use vpdf_ui::split::{Axis, Direction};
  let (mut session, left, _) = split()?;
  assert_eq!(session.tab().panes.len(), 2);
  assert_eq!(session.app().reader.cursor.page.0, 29, "split keeps the view");
  assert_eq!((session.app().at.column, session.app().screen.columns), (0, 60));
  session.app().request = Some(Request::Window {
    command: Window::Split(Axis::Rows),
    count: 1,
    explicit: false,
    file: Some(corpus("paper.pdf")?.into()),
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
  use crate::harness::{drag, mouse, pane_id, press};
  use MouseButton::Left;
  use MouseEventKind::{Down, Drag, ScrollDown, Up};
  use vpdf_model::input::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind,
  };
  use vpdf_ui::keys::PanelKind;

  /// Leaves a `g` prefix pending.
  fn pend_g(session: &mut Session) -> Option<Input> {
    let g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
    session.app().keys.feed(g, vpdf_ui::keys::Grammar::Other)
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn drag_across_the_separator_stays_with_the_press() -> Result<()> {
    let (mut session, left, right) = split()?;
    assert_eq!(session.app().id, left);
    let cursor = pane_id(&mut session, right).reader.cursor;
    mouse(&mut session, Down(Left), (30, 10))?;
    mouse(&mut session, Drag(Left), (100, 10))?;
    mouse(&mut session, Drag(Left), (100, 39))?;
    mouse(&mut session, Up(Left), (100, 39))?;
    assert_eq!(session.app().id, left, "drag and release keep focus");
    assert!(session.app().reader.selection.is_some());
    let other = pane_id(&mut session, right);
    assert_eq!(other.reader.cursor, cursor);
    assert!(other.reader.selection.is_none());
    assert!(!other.reader.animation().active());

    act(&mut session, Action::Cancel)?;
    act(&mut session, Action::Yank)?;
    assert!(pend_g(&mut session).is_none());
    mouse(&mut session, ScrollDown, (90, 10))?;
    assert_eq!(session.app().id, left, "the wheel leaves focus");
    assert!(pane_id(&mut session, right).reader.animation().active());
    assert!(session.app().reader.operator.is_none(), "wheel drops `y`");
    assert_eq!(session.app().keys.awaiting(), None, "wheel drops `g`");
    act(&mut session, Action::Yank)?;
    mouse(&mut session, Down(Left), (30, 10))?;
    assert!(session.app().reader.operator.is_none(), "press drops `y`");

    act(&mut session, Action::Search(false))?;
    pend_g(&mut session);
    mouse(&mut session, Down(Left), (90, 10))?;
    assert_eq!(session.app().id, left, "prompt consumes the press");
    mouse(&mut session, Down(Left), (90, 10))?;
    assert_eq!(session.app().id, right, "a press focuses");
    let old = pane_id(&mut session, left);
    assert!(old.overlays.prompt().is_none() && old.overlays.hints().is_none());
    assert!(old.keys.awaiting().is_none());
    assert!(old.search.prompt_origin().is_none(), "the search was aborted");

    act(&mut session, Action::Yank)?;
    act(&mut session, Action::Window(Window::Next))?;
    assert_eq!(session.app().id, left);
    let old = pane_id(&mut session, right);
    assert!(old.reader.operator.is_none(), "`Ctrl-w` drops `y`");
    Ok(())
  }

  /// A split whose focused left pane shows an unfocused outline;
  /// returns the session and the panel width.
  fn outline() -> Result<(Session, u16)> {
    let (mut session, _, _) = split()?;
    common::outline_loaded(session.app())?;
    act(&mut session, Action::Panel(PanelKind::Outline))?;
    act(&mut session, Action::Cancel)?;
    assert!(session.app().panel_focused(), "Escape preserves panel focus");
    let width = session.app().left;
    assert!(width > 0);
    mouse(&mut session, Down(Left), (width + 5, 10))?;
    mouse(&mut session, Up(Left), (width + 5, 10))?;
    Ok((session, width))
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn only_presses_move_panel_focus() -> Result<()> {
    let (mut session, width) = outline()?;
    let panel = |session: &mut Session| {
      session.app().panel.as_ref().unwrap().view.focused()
    };
    mouse(&mut session, ScrollDown, (2, 5))?;
    assert!(!panel(&mut session), "the wheel leaves the panel unfocused");
    drag(&mut session, (2, 3), (40, 10))?;
    assert!(panel(&mut session), "a press focuses the panel");
    assert!(session.app().reader.selection.is_none(), "no drag from panel");
    mouse(&mut session, Down(Left), (width + 5, 10))?;
    mouse(&mut session, Up(Left), (width + 5, 10))?;
    assert!(!panel(&mut session), "a page press unfocuses the panel");
    let cursor = session.app().reader.cursor;
    drag(&mut session, (40, 39), (40, 10))?;
    assert_eq!(session.app().reader.cursor, cursor, "status row is inert");
    assert!(session.app().reader.selection.is_none());
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn clue_and_menu_rows_hide_the_page() -> Result<()> {
    let (mut session, _) = outline()?;
    let cursor = session.app().reader.cursor;
    for kind in [Down(Left), ScrollDown] {
      pend_g(&mut session);
      common::age_clues(session.app());
      assert!(session.app().clue_due(std::time::Instant::now()));
      mouse(&mut session, kind, (40, 38))?;
      let app = session.app();
      assert_eq!(app.reader.cursor, cursor, "{kind:?} on clues");
      assert!(!app.reader.animation().active(), "{kind:?} on clues scrolls");
      assert_eq!(app.overlays.clues().shown().is_some(), kind == ScrollDown);
      if kind == ScrollDown {
        assert_eq!(app.keys.pending(), "g", "wheel keeps the prefix");
      }
    }
    menu(&mut session)?;
    mouse(&mut session, Down(Left), (40, 38))?;
    assert_eq!(session.app().reader.cursor, cursor, "menu rows are inert");
    assert!(session.app().overlays.prompt().is_some());
    press(&mut session, b"\x1b")?;
    assert_eq!(session.app().menu_rows(), 0, "Esc closes the menu");
    menu(&mut session)?;
    let left = session.app().id;
    mouse(&mut session, Down(Left), (90, 10))?;
    let old = pane_id(&mut session, left);
    assert!(
      old.overlays.prompt().is_none() && old.menu_rows() == 0,
      "blur closes it"
    );
    Ok(())
  }

  /// Opens `:` with its completion menu.
  fn menu(session: &mut Session) -> Result<()> {
    act(session, Action::Command)?;
    press(session, b"t\t")?;
    assert!(session.app().menu_rows() > 0);
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn overview_takes_only_thumbnail_presses() -> Result<()> {
    let (mut session, width) = outline()?;
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Down(Left), (40, 39))?;
    assert!(
      session.app().overlays.overview().is_some(),
      "status row keeps overview"
    );
    let partial = session.app().thumbnail_at(40, 38).expect("partial row");
    mouse(&mut session, Down(Left), (40, 38))?;
    assert!(
      session.app().overlays.overview().is_none(),
      "partial thumbnail opens"
    );
    assert_eq!(session.app().reader.cursor.page, partial);
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Down(Left), (width + 5, 2))?;
    assert!(
      session.app().overlays.overview().is_none(),
      "a thumbnail opens its page"
    );
    mouse(&mut session, Drag(Left), (40, 10))?;
    mouse(&mut session, Up(Left), (40, 10))?;
    assert!(session.app().reader.selection.is_none(), "no drag from overview");

    mouse(&mut session, Down(Left), (width + 5, 10))?;
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Drag(Left), (40, 20))?;
    mouse(&mut session, Up(Left), (40, 20))?;
    assert!(session.app().reader.selection.is_none(), "no drag under `gp`");
    let cursor = session.app().reader.cursor;
    mouse(&mut session, Down(Left), (2, 3))?;
    assert!(
      session.app().overlays.overview().is_none(),
      "a row shows its page"
    );
    assert_ne!(session.app().reader.cursor, cursor);
    // Open the panel before the overview.
    act(&mut session, Action::Panel(PanelKind::Version))?;
    act(&mut session, Action::Overview)?;
    mouse(&mut session, Down(Left), (2, 3))?;
    assert!(
      session.app().overlays.overview().is_some(),
      "Panel rows keep overview"
    );
    Ok(())
  }

  #[test]
  #[ignore = "requires verified public PDF corpus"]
  fn scrolls_in_text_mode_drop_a_pending_yank() -> Result<()> {
    let screen = common::cells(120, 40);
    let mut session = crate::harness::session(&corpus("book.pdf")?, screen)?;
    session.app().text_mode = true;
    session.app().command("30")?;
    act(&mut session, Action::Yank)?;
    mouse(&mut session, ScrollDown, (30, 10))?;
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
  let screen = common::cells(120, 50);
  let small = Screen { columns: 100, rows: 30, ..screen };
  let mut session = crate::harness::session(&corpus("paper.pdf")?, screen)?;
  session.resize(small)?;
  let app = session.app();
  assert_eq!((app.screen.columns, app.screen.rows), (100, 30));
  assert_eq!(
    (app.layout.screen().columns, app.layout.screen().rows),
    (100, 30)
  );
  session.app().command("q")?;
  session.settle()?;
  session.home = Some(vpdf::home::Home::new(
    &[],
    screen,
    &vpdf_pdf::jobs::Wake::none(),
    true,
  ));
  session.resize(small)?;
  let home = session.home.as_ref().unwrap();
  assert_eq!((home.view.screen().columns, home.view.screen().rows), (100, 30));
  Ok(())
}

fn resident_tiles(session: &mut Session) -> Result<()> {
  let mut tiles = Vec::new();
  for app in &mut session.tab().panes {
    for key in vpdf_ui::layout::visible(&app.layout, app.reader.scroll) {
      tiles.push((app.id, vpdf::raster::render(&mut app.doc, key)?));
    }
  }
  let graphics = &mut session.graphics;
  for (id, tile) in tiles {
    graphics.upload(id, &tile, &mut Vec::new())?;
  }
  Ok(())
}

/// Sorted image and placement ids of `action` commands.
fn pairs(frame: &[u8], action: &str) -> Vec<(String, String)> {
  let text = String::from_utf8_lossy(frame);
  let commands = crate::kitty::commands(&text);
  let chosen = commands.iter().filter(|c| c.get("a") == Some(action));
  let ids = |c: &crate::kitty::Command| {
    Some((c.get("i")?.to_owned(), c.get("p")?.to_owned()))
  };
  let mut pairs: Vec<_> = chosen.filter_map(ids).collect();
  pairs.sort();
  pairs
}

/// Split panes share one `Graphics`: a frame starts and ends it once,
/// so the next frame keeps every pane's placements and deletes none.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn split_frames_keep_every_pane_placed() -> Result<()> {
  use vpdf::trace::Drawable;
  let (mut session, _, _) = split()?;
  resident_tiles(&mut session)?;
  let mut frame = Vec::new();
  session.draw_frame(&mut frame)?;
  let first = pairs(&frame, "p");
  // Every pane has resident tiles.
  assert!(first.len() >= 2, "{first:?}");
  frame.clear();
  session.draw_frame(&mut frame)?;
  assert_eq!(pairs(&frame, "p"), first);
  let text = String::from_utf8_lossy(&frame);
  assert!(!text.contains("a=d,"), "{text}");
  Ok(())
}

/// Hidden under tmux, the reader deletes each placement it left, which
/// tmux would leave over the window it shows; the next frame places
/// them all again.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn hiding_deletes_every_placement() -> Result<()> {
  use vpdf::trace::Drawable;
  let (mut session, _, _) = split()?;
  resident_tiles(&mut session)?;
  let mut frame = Vec::new();
  session.draw_frame(&mut frame)?;
  let placed = pairs(&frame, "p");
  assert!(placed.len() >= 2, "{placed:?}");
  let mut hidden = Vec::new();
  session.hide(&mut hidden)?;
  assert_eq!(pairs(&hidden, "d"), placed);
  frame.clear();
  session.draw_frame(&mut frame)?;
  assert_eq!(pairs(&frame, "p"), placed);
  Ok(())
}

/// After a resize, the first frame erases every pane row before it
/// places an image, which removes images `WezTerm` keeps in cells.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn resize_erases_rows_before_placing() -> Result<()> {
  use vpdf::trace::Drawable;
  let (mut session, _, _) = split()?;
  resident_tiles(&mut session)?;
  let mut frame = Vec::new();
  session.draw_frame(&mut frame)?;
  let screen = common::cells(101, 30);
  session.resize(screen)?;
  resident_tiles(&mut session)?;
  frame.clear();
  session.draw_frame(&mut frame)?;
  let text = String::from_utf8_lossy(&frame);
  // Erases (`CSI n X`) before the first placement, then in all.
  let erases = |text: &str| {
    text
      .split("\x1b[")
      .skip(1)
      .filter(|rest| {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && rest[digits..].starts_with('X')
      })
      .count()
  };
  assert!(text.contains("_Ga=p,"), "{text}");
  let first = text.split("_Ga=p,").next().unwrap_or_default();
  // The first pane drawn erases its 30 rows before it places a tile,
  // and both panes erase every row.
  assert!(erases(first) >= 30, "{}", erases(first));
  assert!(erases(&text) >= 2 * 30, "{}", erases(&text));
  Ok(())
}

/// Focus moves no images; a reload drops only its own pane's at the next
/// frame.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn focus_keeps_images_and_reload_drops_its_own() -> Result<()> {
  use vpdf::trace::Drawable;
  let (mut session, left, right) = split()?;
  resident_tiles(&mut session)?;
  let bytes = session.graphics.bytes();
  act(&mut session, Action::Window(Window::Next))?;
  assert_eq!(session.app().id, right);
  assert_eq!(session.graphics.bytes(), bytes, "focus moves nothing");
  session.app().reload_with("")?;
  session.draw_frame(&mut Vec::new())?;
  let visible = |session: &mut Session, id| {
    let app = crate::harness::pane_id(session, id);
    vpdf_ui::layout::visible(&app.layout, app.reader.scroll)
  };
  let kept = visible(&mut session, left);
  assert!(kept.iter().all(|&key| session.graphics.contains(&(left, key))));
  let dropped = visible(&mut session, right);
  assert!(!dropped.iter().any(|&key| session.graphics.contains(&(right, key))));
  Ok(())
}

/// Closing a pane deletes its images and keeps its sibling's.
#[test]
fn closed_panes_drop_their_images() -> Result<()> {
  let (_directory, path) =
    common::temp_pdf(|path| common::text_pdf(path, &["Images per pane"]))?;
  let screen = common::cells(121, 40);
  let mut session = crate::harness::session(&path, screen)?;
  let kept = session.app().id;
  act(
    &mut session,
    Action::Window(Window::Split(vpdf_ui::split::Axis::Columns)),
  )?;
  let closed = session.app().id;
  resident_tiles(&mut session)?;
  let owned = |session: &mut Session, id| {
    let app = crate::harness::pane_id(session, id);
    vpdf_ui::layout::visible(&app.layout, app.reader.scroll)
  };
  let keys = owned(&mut session, kept);
  let gone = owned(&mut session, closed);
  assert!(gone.iter().all(|&key| session.graphics.contains(&(closed, key))));
  act(&mut session, Action::Window(Window::Close))?;
  assert_eq!(session.app().id, kept);
  assert!(!gone.iter().any(|&key| session.graphics.contains(&(closed, key))));
  assert!(keys.iter().all(|&key| session.graphics.contains(&(kept, key))));
  Ok(())
}
