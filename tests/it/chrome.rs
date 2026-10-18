//! Middle button, chrome, and dividers.

use crate::mouse::{cell, cell_in, labels, mouse, press, session, status};
use crossterm::event::{KeyCode, MouseButton, MouseEventKind};
use vpdf::Result;
use vpdf::pointer::{Hit, Region};
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
      let hit = app.hovered(Region::Page, column, row, None)?;
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

/// Pane rects in tree order.
fn layout(session: &mut Session) -> Vec<(u16, u16, u16, u16)> {
  let tab = session.tab();
  tab
    .order()
    .iter()
    .filter_map(|id| tab.panes.iter().find(|app| app.id == *id))
    .map(|app| (app.at.column, app.at.row, app.screen.columns, app.screen.rows))
    .collect()
}

/// One settled pointer event; whether it draws.
fn left(session: &mut Session, kind: MouseEventKind, at: (u16, u16)) -> bool {
  let drawn = mouse(session, kind, at).expect("pointer event");
  session.settle().expect("settled event");
  drawn
}

#[test]
fn dividers_drag_and_double_clicks_equalize() -> Result<()> {
  use MouseEventKind::{Down, Drag, Moved, Up};
  const L: MouseButton = MouseButton::Left;
  // (command, divider, drag to) -> layout
  let cases = [
    ("vsplit", (60, 5), (40, 9), [(0, 0, 40, 40), (41, 0, 80, 40)]),
    ("vsplit", (60, 5), (2, 9), [(0, 0, 10, 40), (11, 0, 110, 40)]),
    ("split", (5, 19), (9, 29), [(0, 0, 121, 30), (0, 30, 121, 10)]),
  ];
  for (command, divider, to, want) in cases {
    let (_directory, mut session) = session(false)?;
    session.app().command(command)?;
    session.settle()?;
    let focused = session.app().id;
    let equal = layout(&mut session);
    // Motion onto a divider draws its shape.
    left(&mut session, Moved, (30, 10));
    assert!(left(&mut session, Moved, divider), "{command} hover");
    let hover = session.app().hover;
    assert!(!matches!(hover, Hit::Edge { .. }), "no pane hovers edges");
    assert!(left(&mut session, Down(L), divider), "{command}");
    // One button at a time.
    left(&mut session, Down(MouseButton::Middle), (100, 35));
    left(&mut session, Drag(MouseButton::Middle), (90, 30));
    assert_eq!(session.app().id, focused, "{command} middle");
    assert!(!session.app().grabbing(), "{command} middle");
    assert!(left(&mut session, Drag(L), to), "{command} {to:?}");
    assert!(!left(&mut session, Drag(L), to), "unchanged drags draw nothing");
    left(&mut session, Up(L), to);
    assert_eq!(layout(&mut session), want, "{command} {to:?}");
    assert_eq!(session.app().id, focused, "dividers take no focus");
    // A lost release ends the drag.
    let first = want[0];
    let moved =
      if command == "split" { (5, first.3 - 1) } else { (first.2, 5) };
    left(&mut session, Down(L), moved);
    left(&mut session, Moved, (50, 5));
    left(&mut session, Drag(L), divider);
    assert_eq!(layout(&mut session), want, "{command} after Moved");
    // Double clicks equalize; single ones do not.
    left(&mut session, Up(L), moved);
    std::thread::sleep(std::time::Duration::from_millis(400));
    left(&mut session, Down(L), moved);
    left(&mut session, Up(L), moved);
    assert_eq!(layout(&mut session), want, "{command} one click");
    left(&mut session, Down(L), moved);
    left(&mut session, Up(L), moved);
    assert_eq!(layout(&mut session), equal, "{command} equalized");
  }
  Ok(())
}

#[test]
fn panel_edges_drag_and_reset() -> Result<()> {
  use MouseEventKind::{Down, Drag, Up};
  const L: MouseButton = MouseButton::Left;
  let (_directory, mut session) = session(false)?;
  keys(&mut session, "go")?;
  status(&mut session)?;
  let default = session.app().left;
  let edge = (default - 1, 5);
  // Motion onto the border draws its shape.
  assert!(left(&mut session, MouseEventKind::Moved, (default - 2, 5)));
  assert!(left(&mut session, MouseEventKind::Moved, edge));
  assert!(left(&mut session, MouseEventKind::Moved, (default - 2, 5)));
  let selected = session.app().panel.as_ref().map(|panel| panel.selected);
  // (drag to column) -> panel columns
  let cases = [(30, 31), (2, 8), (115, 109)];
  left(&mut session, Down(L), edge);
  for (column, want) in cases {
    left(&mut session, Drag(L), (column, 9));
    assert_eq!(session.app().left, want, "to {column}");
  }
  left(&mut session, Up(L), (115, 9));
  let panel = session.app().panel.as_ref().map(|panel| panel.selected);
  assert_eq!(panel, selected, "the border selects no row");
  // A double click restores the default width.
  let edge = (session.app().left - 1, 5);
  for _ in 0..2 {
    left(&mut session, Down(L), edge);
    left(&mut session, Up(L), edge);
  }
  assert_eq!(session.app().left, default);
  Ok(())
}

#[test]
fn boundary_presses_never_focus() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("split")?;
  session.settle()?;
  // Focus the lower pane.
  crate::mouse::click(&mut session, (5, 30))?;
  let focused = session.app().id;
  for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
    left(&mut session, MouseEventKind::Down(button), (5, 19));
    left(&mut session, MouseEventKind::Up(button), (5, 19));
    assert_eq!(session.app().id, focused, "{button:?}");
  }
  // A held pan keeps the left button off boundaries.
  let before = layout(&mut session);
  left(&mut session, MouseEventKind::Down(MouseButton::Middle), (5, 30));
  left(&mut session, MouseEventKind::Down(MouseButton::Left), (5, 19));
  left(&mut session, MouseEventKind::Drag(MouseButton::Left), (5, 10));
  left(&mut session, MouseEventKind::Up(MouseButton::Middle), (5, 30));
  assert_eq!(layout(&mut session), before);
  assert!(!session.app().grabbing());
  Ok(())
}

/// The focused pane's screen cell showing `item`.
fn chrome_cell(
  session: &mut Session,
  item: vpdf::chrome::Chrome,
) -> Result<(u16, u16)> {
  let app = session.app();
  for region in [Region::Status, Region::Overlay] {
    for row in 0..app.screen.rows {
      for column in 0..app.screen.columns {
        if app.chrome_at(region, column, row) == Some(item) {
          return Ok((app.at.column + column, app.at.row + row));
        }
      }
    }
  }
  anyhow::bail!("no cell shows {item:?}")
}

/// A settled left click.
fn click(session: &mut Session, at: (u16, u16)) {
  left(session, MouseEventKind::Down(MouseButton::Left), at);
  left(session, MouseEventKind::Up(MouseButton::Left), at);
}

#[test]
fn status_items_act_on_click() -> Result<()> {
  use vpdf::chrome::Chrome;
  let (directory, mut session) = session(false)?;
  for name in ["b.pdf", "c.pdf"] {
    let other = directory.path().join(name);
    crate::common::text_pdf(&other, &["Other words"])?;
    session.open(&other, true, crate::focus::screen())?;
  }
  status(&mut session)?;
  // Item to item motion draws nothing.
  let first = chrome_cell(&mut session, Chrome::Tab(1))?;
  let zero = chrome_cell(&mut session, Chrome::Tab(0))?;
  assert!(left(&mut session, MouseEventKind::Moved, (0, 0)));
  assert!(left(&mut session, MouseEventKind::Moved, zero));
  assert!(!left(&mut session, MouseEventKind::Moved, first));
  // A held drag keeps middle presses off tabs.
  left(&mut session, MouseEventKind::Down(MouseButton::Left), (40, 10));
  left(&mut session, MouseEventKind::Down(MouseButton::Middle), first);
  left(&mut session, MouseEventKind::Up(MouseButton::Left), (40, 10));
  assert_eq!(session.tabs.len(), 3);
  // Presses act on the shown item.
  keys(&mut session, "y")?;
  status(&mut session)?;
  let pending = chrome_cell(&mut session, Chrome::Tab(1))?;
  click(&mut session, pending);
  assert_eq!(session.active, 1);
  status(&mut session)?;
  let id = session.app().id;
  let before = chrome_cell(&mut session, Chrome::Tab(0))?;
  middle(&mut session, before)?;
  assert_eq!((session.tabs.len(), session.app().id), (2, id), "kept active");
  status(&mut session)?;
  let after = chrome_cell(&mut session, Chrome::Tab(1))?;
  middle(&mut session, after)?;
  assert_eq!(session.tabs.len(), 1);
  // Zoom fits; the counter opens pages.
  let (_directory, mut session) = crate::mouse::session(false)?;
  keys(&mut session, "+")?;
  status(&mut session)?;
  let zoom = chrome_cell(&mut session, Chrome::Zoom)?;
  click(&mut session, zoom);
  assert!((session.app().layout.zoom - 1.0).abs() < 0.01);
  status(&mut session)?;
  let pages = chrome_cell(&mut session, Chrome::Pages)?;
  click(&mut session, pages);
  assert!(session.app().overview.is_some());
  status(&mut session)?;
  assert!(chrome_cell(&mut session, Chrome::Pages).is_err(), "inert in gp");
  // Sections open outlines; notes hide them.
  let (_directory, mut session) = crate::mouse::session(false)?;
  session.app().command("2")?;
  status(&mut session)?;
  let section = chrome_cell(&mut session, Chrome::Section)?;
  session.app().reader.message = "A note over the section".into();
  status(&mut session)?;
  click(&mut session, section);
  assert!(session.app().panel.is_none(), "the click clears the note");
  status(&mut session)?;
  let section = chrome_cell(&mut session, Chrome::Section)?;
  click(&mut session, section);
  let app = session.app();
  let panel = app.panel.as_ref().map(|p| (p.focused, Some(p.selected)));
  assert_eq!(panel, Some((true, app.current_section)));
  Ok(())
}

#[test]
fn unfocused_status_rows_only_focus() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("vsplit")?;
  session.settle()?;
  status(&mut session)?;
  // That item in the same-sized pane.
  let pages = chrome_cell(&mut session, vpdf::chrome::Chrome::Pages)?;
  let index = (session.tab().focus + 1) % 2;
  let (here, there) = (session.app().at, session.tab().panes[index].at);
  let at = (pages.0 - here.column + there.column, pages.1);
  let id = session.tab().panes[index].id;
  click(&mut session, at);
  assert_eq!(session.app().id, id);
  assert!(session.app().overview.is_none());
  Ok(())
}

#[test]
fn status_items_beat_the_divider() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("split")?;
  session.settle()?;
  status(&mut session)?;
  let before = layout(&mut session);
  let pages = chrome_cell(&mut session, vpdf::chrome::Chrome::Pages)?;
  assert_eq!(pages.1, 19, "the upper pane's status row");
  assert!(left(&mut session, MouseEventKind::Moved, pages));
  left(&mut session, MouseEventKind::Down(MouseButton::Left), pages);
  left(&mut session, MouseEventKind::Drag(MouseButton::Left), (pages.0, 9));
  left(&mut session, MouseEventKind::Up(MouseButton::Left), (pages.0, 9));
  assert!(session.app().overview.is_some());
  assert_eq!(layout(&mut session), before);
  Ok(())
}

#[test]
fn clues_and_completions_act_on_click() -> Result<()> {
  use crossterm::event::{KeyEvent, KeyModifiers};
  use vpdf::chrome::Chrome;
  let (_directory, mut session) = session(false)?;
  let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
  session.app().key(ctrl_w, &mut vpdf::prompt::History::default())?;
  session.app().awaiting =
    std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1));
  assert!(session.app().clue_due());
  let clues = session.app().clues.as_ref().map(|c| c.items.clone());
  let split = clues.and_then(|items| items.iter().position(|(k, _)| k == "v"));
  let at = chrome_cell(&mut session, Chrome::Clue(split.expect("a v clue")))?;
  click(&mut session, at);
  session.settle()?;
  assert_eq!(session.tab().panes.len(), 2);
  assert!(session.app().clues.is_none());
  // A failing clue still closes.
  let (_text, mut failing) = crate::mouse::session(true)?;
  keys(&mut failing, "g")?;
  failing.app().awaiting =
    std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1));
  assert!(failing.app().clue_due());
  let clues = failing.app().clues.as_ref().map(|c| c.items.clone());
  let pages = clues.and_then(|items| items.iter().position(|(k, _)| k == "p"));
  status(&mut failing)?;
  let at = chrome_cell(&mut failing, Chrome::Clue(pages.expect("a p clue")))?;
  let down = mouse(&mut failing, MouseEventKind::Down(MouseButton::Left), at);
  assert!(down.is_err(), "the session reports it");
  let app = failing.app();
  assert!(app.clues.is_none());
  assert_eq!(app.keys.pending(), "");
  // Ranges name no key.
  keys(&mut session, "m")?;
  session.app().awaiting =
    std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1));
  assert!(session.app().clue_due());
  status(&mut session)?;
  let app = session.app();
  let rows = app.screen.rows;
  let columns = app.screen.columns;
  let items = (0..rows).flat_map(|row| (0..columns).map(move |c| (c, row)));
  let mut any =
    items.filter_map(|(c, row)| app.chrome_at(Region::Overlay, c, row));
  let any = any.next();
  assert_eq!(any, None);
  // Blank clue rows close the clues.
  let app = session.app();
  let blank = (app.at.column + app.screen.columns - 1, app.screen.rows - 2);
  click(&mut session, blank);
  assert!(session.app().clues.is_none());
  // A click picks a completion.
  keys(&mut session, ":tab")?;
  press(&mut session, KeyCode::Tab)?;
  let items = session.app().prompt.as_ref().and_then(|p| p.menu.as_ref());
  let second = items.map(|menu| menu.items[1].clone()).expect("a menu");
  status(&mut session)?;
  let at = chrome_cell(&mut session, Chrome::Completion(1))?;
  click(&mut session, at);
  let text = session.app().prompt.as_ref().map(|p| p.text.clone());
  assert!(text.is_some_and(|text| text.ends_with(&second)), "{second}");
  // Blank completion rows keep the prompt.
  let app = session.app();
  let blank = (app.at.column + app.screen.columns - 1, app.screen.rows - 2);
  click(&mut session, blank);
  assert!(session.app().prompt.is_some());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_clicks_select_their_entry() -> Result<()> {
  let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, crate::focus::screen())?;
  crate::common::outline_loaded(session.app())?;
  session.app().command("4")?;
  status(&mut session)?;
  keys(&mut session, "go")?;
  if let Some(panel) = &mut session.app().panel {
    (panel.selected, panel.focused) = (12, false);
  }
  status(&mut session)?;
  let selected = session.app().panel.as_ref().map(|p| p.selected);
  assert_eq!(selected, Some(12), "a stale selection");
  let section = chrome_cell(&mut session, vpdf::chrome::Chrome::Section)?;
  click(&mut session, section);
  let app = session.app();
  let panel = app.panel.as_ref().map(|p| (p.focused, Some(p.selected)));
  assert_eq!(panel, Some((true, app.current_section)));
  assert_ne!(app.current_section, Some(12));
  Ok(())
}
