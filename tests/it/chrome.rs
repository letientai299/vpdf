//! Middle button and clickable chrome.

use crate::mouse::{cell, cell_in, labels, mouse, press, session, status};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use vpdf::Result;
use vpdf::pointer::Hit;
use vpdf::session::Session;
use vpdf::text::PageId;

/// A middle click, settled after each event.
fn middle(session: &mut Session, at: (u16, u16)) -> Result<()> {
  mouse(session, MouseEventKind::Down(MouseButton::Middle), at)?;
  session.settle()?;
  mouse(session, MouseEventKind::Up(MouseButton::Middle), at)?;
  session.settle()
}

fn keys(session: &mut Session, keys: &str) -> Result<()> {
  for ch in keys.chars() {
    press(session, KeyCode::Char(ch))?;
  }
  session.settle()
}

/// Panes, focused id, other pane's page.
fn panes(session: &mut Session) -> (usize, u32, Option<PageId>) {
  let focused = session.app().id;
  let tab = session.tab();
  let other = tab.panes.iter().find(|app| app.id != focused);
  (tab.panes.len(), focused, other.map(|app| app.reader.cursor.page))
}

#[test]
fn middle_clicks_split_links_and_copy_addresses() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let id = session.app().id;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let web = cell(&mut session, Hit::Link(PageId(0), 1))?;
    let plain = cell(&mut session, Hit::Text)?;
    // (cell) -> (panes, other pane's page, register)
    let cases = [
      (plain, (1, None), None),
      (web, (1, None), Some("https://example.com/")),
      (internal, (2, Some(PageId(1))), Some("https://example.com/")),
    ];
    for (at, (count, page), register) in cases {
      middle(&mut session, at)?;
      let app = session.app();
      let got = app.reader.registers.get(&'"').map(ToString::to_string);
      assert_eq!(got.as_deref(), register, "text={text_mode} {at:?}");
      assert_eq!(panes(&mut session), (count, id, page), "text={text_mode}");
    }
    // The source pane keeps its left edge.
    assert_eq!(session.app().at.column, 0, "text={text_mode}");
  }
  Ok(())
}

#[test]
fn middle_drags_pan_both_axes() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  app.layout.zoom = 3.0;
  app.relayout()?;
  app.reader.scroll = 64.0;
  let (scroll, pan) = (app.reader.scroll, app.layout.pan);
  let link = cell(&mut session, Hit::Link(PageId(0), 0))?;
  let (column, row) = (link.0 + 20, link.1 + 10);
  let down = MouseEventKind::Down(MouseButton::Middle);
  mouse(&mut session, down, (column, row))?;
  assert!(!session.app().grabbing(), "a press alone does not pan");
  // (drag to) -> (scroll, pan) moved by
  let cases = [
    ((column - 5, row), (0.0, 40.0)),
    ((column - 5, row - 2), (32.0, 40.0)),
    ((column + 1, row + 1), (-16.0, -8.0)),
  ];
  for (at, (down, right)) in cases {
    let drag = MouseEventKind::Drag(MouseButton::Middle);
    assert!(mouse(&mut session, drag, at)?, "{at:?} draws");
    let app = session.app();
    assert!(app.grabbing());
    assert_eq!(app.reader.scroll, scroll + down, "{at:?}");
    assert_eq!(app.layout.pan, pan + right, "{at:?}");
  }
  // Releasing a dragged link opens nothing.
  mouse(&mut session, MouseEventKind::Up(MouseButton::Middle), link)?;
  session.settle()?;
  assert!(!session.app().grabbing());
  assert_eq!(session.tab().panes.len(), 1);
  Ok(())
}

#[test]
fn menus_open_links_in_splits() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let id = session.app().id;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  crate::mouse::right(&mut session, internal)?;
  let at = labels(&mut session).iter().position(|l| *l == "Open in split");
  for _ in 0..at.expect("Open in split") {
    press(&mut session, KeyCode::Char('j'))?;
  }
  press(&mut session, KeyCode::Enter)?;
  session.settle()?;
  assert_eq!(panes(&mut session), (2, id, Some(PageId(1))));
  Ok(())
}

#[test]
fn middle_clicks_split_rows_and_thumbnails() -> Result<()> {
  // (keys, row hit) -> the split's page
  let cases = [
    ("go", Hit::Row(0), Some(PageId(1))),
    ("gp", Hit::Thumbnail(PageId(1)), Some(PageId(1))),
    ("mAgg", Hit::Row(0), Some(PageId(1))),
  ];
  for (typed, hit, page) in cases {
    let (_directory, mut session) = session(false)?;
    let id = session.app().id;
    if typed == "mAgg" {
      // A listed global mark on page 2.
      session.app().command("2")?;
      keys(&mut session, "mA")?;
      session.app().command("1")?;
      session.app().request =
        Some(vpdf::app::Request::Panel(vpdf::keys::PanelKind::Marks));
      session.settle()?;
    } else {
      keys(&mut session, typed)?;
    }
    status(&mut session)?;
    let at = cell(&mut session, hit)?;
    // A double click splits once.
    middle(&mut session, at)?;
    middle(&mut session, at)?;
    assert_eq!(panes(&mut session), (2, id, page), "{typed}");
  }
  Ok(())
}

#[test]
fn middle_clicks_copy_web_rows() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  keys(&mut session, "gl")?;
  status(&mut session)?;
  let rows = session.app().panel.as_ref().map(|p| p.rows.len());
  let web = (0..rows.unwrap_or(0)).find(|&index| {
    let panel = session.tab().panes[0].panel.as_ref();
    panel.is_some_and(|panel| {
      matches!(
        panel.rows[index].destination,
        vpdf::panel::Destination::Link(vpdf::document::Target::Uri(_))
      )
    })
  });
  let at = cell(&mut session, Hit::Row(web.expect("a web row")))?;
  middle(&mut session, at)?;
  let app = session.app();
  let got = app.reader.registers.get(&'"').map(ToString::to_string);
  assert_eq!(got.as_deref(), Some("https://example.com/"));
  assert_eq!(session.tab().panes.len(), 1);
  Ok(())
}

#[test]
fn text_drags_split_nothing() -> Result<()> {
  let (_directory, mut session) = session(true)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  let view = session.app().navigation_position();
  mouse(&mut session, MouseEventKind::Down(MouseButton::Middle), internal)?;
  let to = (internal.0 + 3, internal.1 + 1);
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Middle), to)?;
  assert!(!session.app().grabbing(), "text drags show no grab");
  assert_eq!(session.app().navigation_position(), view);
  mouse(&mut session, MouseEventKind::Up(MouseButton::Middle), to)?;
  session.settle()?;
  assert_eq!(session.tab().panes.len(), 1);
  Ok(())
}

#[test]
fn middle_presses_respect_overlays_and_focus() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  // An open menu or prompt only closes.
  let web = cell(&mut session, Hit::Link(PageId(0), 1))?;
  for overlay in ["menu", "prompt"] {
    match overlay {
      "menu" => crate::mouse::right(&mut session, (web.0, web.1 + 1))?,
      _ => keys(&mut session, ":")?,
    }
    middle(&mut session, internal)?;
    let app = session.app();
    let open = app.context_menu.is_some() || app.peek.is_some();
    assert!(!open && app.prompt.is_none(), "{overlay}");
    assert_eq!(session.tab().panes.len(), 1, "{overlay}");
  }
  // Middle presses focus and split their pane.
  session.app().command("vsplit")?;
  session.settle()?;
  let index = (session.tab().focus + 1) % 2;
  let id = session.tab().panes[index].id;
  let link = cell_in(&mut session, index, Hit::Link(PageId(0), 0))?;
  middle(&mut session, link)?;
  assert_eq!(session.tab().panes.len(), 3);
  assert_eq!(session.app().id, id);
  // Lost releases end pans.
  let link = cell(&mut session, Hit::Text)?;
  mouse(&mut session, MouseEventKind::Down(MouseButton::Middle), link)?;
  let to = (link.0 + 2, link.1);
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Middle), to)?;
  assert!(session.app().grabbing());
  assert!(mouse(&mut session, MouseEventKind::Moved, to)?);
  assert!(!session.app().grabbing());
  Ok(())
}

#[test]
fn clicks_still_work_after_middle_presses() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  mouse(&mut session, MouseEventKind::Moved, internal)?;
  mouse(&mut session, MouseEventKind::Down(MouseButton::Middle), internal)?;
  // Unmoved lost presses draw nothing.
  assert!(!mouse(&mut session, MouseEventKind::Moved, internal)?);
  crate::mouse::click(&mut session, internal)?;
  assert_eq!(session.app().reader.cursor.page, PageId(1));
  Ok(())
}

#[test]
fn double_clicks_on_the_new_half_split_once() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let id = session.app().id;
  let app = session.app();
  let mut right = None;
  for column in 0..app.screen.columns {
    for row in 0..app.screen.rows - 1 {
      let hit = app.hovered(vpdf::pointer::Region::Page, column, row, None)?;
      if hit == Hit::Link(PageId(0), 0) {
        right = Some((column, row));
      }
    }
  }
  let right = right.expect("a link cell");
  middle(&mut session, right)?;
  middle(&mut session, right)?;
  assert_eq!(panes(&mut session), (2, id, Some(PageId(1))));
  Ok(())
}

#[test]
fn mark_rows_of_other_documents_split_beside() -> Result<()> {
  let (directory, mut session) = session(false)?;
  session.app().command("2")?;
  keys(&mut session, "mA")?;
  let other = directory.path().join("other.pdf");
  crate::common::text_pdf(&other, &["Other words"])?;
  session.open(&other, false, crate::focus::screen())?;
  let id = session.app().id;
  session.app().request =
    Some(vpdf::app::Request::Panel(vpdf::keys::PanelKind::Marks));
  session.settle()?;
  status(&mut session)?;
  let at = cell(&mut session, Hit::Row(0))?;
  middle(&mut session, at)?;
  middle(&mut session, at)?;
  assert_eq!(panes(&mut session), (2, id, Some(PageId(1))));
  assert_eq!(session.app().at.column, 0, "the source stays left");
  Ok(())
}
