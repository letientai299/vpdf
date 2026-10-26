//! Middle button, chrome, and dividers.
use super::common::SeedReader;

use crate::harness::{
  cell_in, corpus, draw as status, mouse, pane, press, screen,
};
use crate::mouse::{cell, choose, labels, session};
use vpdf::Result;
use vpdf::session::Session;
use vpdf_model::input::{MouseButton, MouseEventKind};
use vpdf_model::text::PageId;
use vpdf_ui::pointer::{Hit, Region};

/// A middle click, settled after each event.
fn middle(session: &mut Session, at: (u16, u16)) -> Result<()> {
  mouse(session, MouseEventKind::Down(MouseButton::Middle), at)?;
  session.settle()?;
  mouse(session, MouseEventKind::Up(MouseButton::Middle), at)?;
  session.settle()
}

fn keys(session: &mut Session, keys: &str) -> Result<()> {
  press(session, keys.as_bytes())?;
  session.settle()?;
  if let Some(kind) = session.app().panel.as_ref().map(|p| p.view.kind()) {
    crate::common::sidebar_loaded(session.app(), kind)?;
  }
  Ok(())
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
      let got = app.reader.register('"').map(ToString::to_string);
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
  app.layout.request_zoom(vpdf_ui::layout::Zoom::To(3.0));
  app.relayout()?;
  app.reader.seed_reader(|reader| reader.scroll = 64.0);
  let (scroll, pan) = (app.reader.scroll, app.layout.pan());
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
    assert_eq!(app.layout.pan(), pan + right, "{at:?}");
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
  choose(&mut session, "Open in split")?;
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
        Some(vpdf::app::Request::Panel(vpdf_ui::keys::PanelKind::Marks));
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
  let rows = session.app().panel.as_ref().map(|p| p.view.rows().len());
  let web = (0..rows.unwrap_or(0)).find(|&index| {
    let panel = session.tab().panes[0].panel.as_ref();
    panel.is_some_and(|panel| {
      matches!(
        panel.view.rows()[index].destination,
        vpdf_ui::panel::Destination::Link(vpdf_model::document::Target::Uri(_))
      )
    })
  });
  let at = cell(&mut session, Hit::Row(web.expect("a web row")))?;
  middle(&mut session, at)?;
  let app = session.app();
  let got = app.reader.register('"').map(ToString::to_string);
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
  assert!(session.app().grabbing(), "text drags grab");
  // The top left cannot move further.
  assert_eq!(session.app().navigation_position(), view);
  mouse(&mut session, MouseEventKind::Up(MouseButton::Middle), to)?;
  session.settle()?;
  assert_eq!(session.tab().panes.len(), 1);
  Ok(())
}

/// The menu's labels and shown keys.
fn menu(session: &mut Session) -> Vec<(&str, String)> {
  let menu = session.app().overlays.menu();
  let items = menu.map(|menu| menu.entries().iter());
  items
    .into_iter()
    .flatten()
    .map(|i| (i.label.as_ref(), i.key.clone()))
    .collect()
}

/// U13: panels and the overview open menus of their own keys.
#[test]
fn panel_and_overview_menus_run_their_items() -> Result<()> {
  type Case<'a> = (&'a str, Hit, &'a [(&'a str, &'a str)], &'a str);
  let row = [
    ("Follow", "<CR>"),
    ("Follow, keep panel", "o"),
    ("Open in split", ""),
    ("Copy row", "yy"),
    ("Close panel", "q"),
    ("Expand all", "zR"),
    ("Collapse all", "zM"),
    ("Auto-expand", "za"),
  ];
  let thumbnail = [
    ("Open", "<CR>"),
    ("Open in split", ""),
    ("Back", "<C-o>"),
    ("Forward", "<C-i>"),
    ("Close overview", "q"),
  ];
  // (keys, hit, items, item run) -> (panes, other page, cursor page)
  let cases: [(Case, _); 4] = [
    (("go", Hit::Row(0), &row, "Open in split"), (2, Some(PageId(1)), 0)),
    (("go", Hit::Row(0), &row, "Follow"), (1, None, 1)),
    (("gp", Hit::Thumbnail(PageId(1)), &thumbnail, "Open"), (1, None, 1)),
    (
      ("gp", Hit::Thumbnail(PageId(1)), &thumbnail, "Open in split"),
      (2, Some(PageId(1)), 0),
    ),
  ];
  for ((typed, hit, items, run), (count, page, cursor)) in cases {
    let (_directory, mut session) = session(false)?;
    let id = session.app().id;
    keys(&mut session, typed)?;
    status(&mut session)?;
    let at = cell(&mut session, hit)?;
    crate::mouse::right(&mut session, at)?;
    let got = menu(&mut session);
    let want: Vec<_> =
      items.iter().map(|(l, k)| (*l, (*k).to_owned())).collect();
    assert_eq!(got, want, "{typed} {run}");
    // Drawn over the panel and page.
    let frame = status(&mut session)?;
    assert!(frame.contains(run), "{typed}: {run} drawn");
    // A click on the item runs it.
    let index = items.iter().position(|(label, _)| *label == run);
    let area =
      session.app().overlays.menu().map(vpdf_ui::context::ContextMenu::area);
    let area = area.expect("a menu");
    let row = area.row + u16::try_from(index.expect(run))?;
    crate::mouse::click(&mut session, (area.column, row))?;
    session.settle()?;
    let cursor_page = session.app().reader.cursor.page;
    assert!(session.app().overlays.menu().is_none(), "{typed} {run}: closed");
    assert_eq!(panes(&mut session), (count, id, page), "{typed} {run}");
    assert_eq!(cursor_page, PageId(cursor), "{typed} {run}");
  }
  Ok(())
}

/// Address rows copy instead of splitting; right drags run row items.
#[test]
fn row_menus_copy_and_select() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  keys(&mut session, "gl")?;
  status(&mut session)?;
  let web = cell(&mut session, Hit::Row(1))?;
  crate::mouse::right(&mut session, web)?;
  let app = session.app();
  assert_eq!(app.panel.as_ref().map(|panel| panel.view.selected()), Some(1));
  assert!(app.panel_focused());
  let want = ["Follow", "Follow, keep panel", "Copy address", "Close panel"];
  assert_eq!(labels(&mut session), want);
  // A right drag onto the item and its release run it.
  let area =
    session.app().overlays.menu().map(vpdf_ui::context::ContextMenu::area);
  let area = area.expect("a menu");
  session.app().overlays.close_menu();
  mouse(&mut session, MouseEventKind::Down(MouseButton::Right), web)?;
  let item = (area.column + 1, area.row + 2);
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Right), item)?;
  mouse(&mut session, MouseEventKind::Up(MouseButton::Right), item)?;
  let app = session.app();
  assert!(app.overlays.menu().is_none(), "the release ran it");
  let got = app.reader.register('"').map(ToString::to_string);
  assert_eq!(got.as_deref(), Some("https://example.com/"));
  assert_eq!(app.reader.message, "Copied link");
  Ok(())
}

/// Outline generations invalidate row menus and selection.
#[test]
fn outline_refills_invalidate_targets() -> Result<()> {
  use vpdf_model::document::{Heading, Target};
  let opening = || Heading {
    title: "Opening".into(),
    depth: 0,
    target: Target::Page(PageId(0)),
  };
  for shifted in [true, false] {
    let (_directory, mut session) = session(false)?;
    let id = session.app().id;
    keys(&mut session, "go")?;
    status(&mut session)?;
    let row = cell(&mut session, Hit::Row(0))?;
    crate::mouse::right(&mut session, row)?;
    let app = session.app();
    if shifted {
      app.doc.outline.insert(0, opening());
    } else {
      app.doc.outline = vec![opening()];
    }
    app.refill_panel(vpdf_ui::keys::PanelKind::Outline)?;
    assert!(app.overlays.menu().is_none(), "the generation changed");
    assert_eq!(app.panel.as_ref().unwrap().view.selected(), 0);
    assert_eq!(panes(&mut session), (1, id, None));
  }
  Ok(())
}

/// The covered recent view opens with Enter alone; its menu stays left
/// of the card.
#[test]
fn recent_menus_open_rows() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  let entry = vpdf::state::Entry {
    path: app.path.clone(),
    cursor: vpdf_model::text::Position::default(),
    zoom: 1.0,
    rotation: 0,
    used: 0,
    marks: std::collections::BTreeMap::new(),
    crop: vpdf_ui::layout::Crop::default(),
    outline: None,
  };
  app.show_recent(&[entry])?;
  assert!(app.covered());
  // Before the card first draws, near the list's right end: 121 columns
  // leave the card 48.
  let row = cell(&mut session, Hit::Row(0))?;
  crate::mouse::right(&mut session, (121 - 48 - 3, row.1))?;
  let want = [
    "Open",
    "Open in split",
    "Copy row",
    "Toggle checkbox",
    "Remove from recent",
    "Close panel",
  ];
  assert_eq!(labels(&mut session), want);
  status(&mut session)?;
  let app = session.app();
  let area = app.overlays.menu().expect("a menu").area();
  let card = app.panel.as_ref().and_then(|panel| panel.view.shelf());
  let start = card.and_then(vpdf_ui::panel::recent::Shelf::start);
  assert!(area.column + area.columns <= start.expect("a drawn card"));
  // Turns over the menu leave the hidden page.
  let scroll = session.app().reader.scroll;
  mouse(&mut session, MouseEventKind::ScrollDown, (area.column, area.row))?;
  let app = session.app();
  assert_eq!(app.reader.scroll.to_bits(), scroll.to_bits());
  assert!(!app.reader.animation().active() && app.overlays.menu().is_some());
  Ok(())
}

/// A right drag from a thumbnail onto an item runs it on release.
#[test]
fn overview_menus_take_right_drags() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  keys(&mut session, "gp")?;
  status(&mut session)?;
  let at = cell(&mut session, Hit::Thumbnail(PageId(1)))?;
  mouse(&mut session, MouseEventKind::Down(MouseButton::Right), at)?;
  let area =
    session.app().overlays.menu().map(vpdf_ui::context::ContextMenu::area);
  let area = area.expect("a menu");
  // Open, Open in split, Back, ...
  let item = (area.column + 2, area.row + 1);
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Right), item)?;
  mouse(&mut session, MouseEventKind::Up(MouseButton::Right), item)?;
  session.settle()?;
  let id = session.app().id;
  assert_eq!(panes(&mut session), (2, id, Some(PageId(1))));
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
    let open =
      app.overlays.menu().is_some() || app.overlays.preview().is_some();
    assert!(!open && app.overlays.prompt().is_none(), "{overlay}");
    assert_eq!(session.tab().panes.len(), 1, "{overlay}");
  }
  // Middle presses focus and split their pane.
  session.app().command("vsplit")?;
  session.settle()?;
  let index = (session.tab().focus + 1) % 2;
  let id = pane(&mut session, index).id;
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
  session.open(&other, false, screen())?;
  let id = session.app().id;
  session.app().request =
    Some(vpdf::app::Request::Panel(vpdf_ui::keys::PanelKind::Marks));
  session.settle()?;
  status(&mut session)?;
  let at = cell(&mut session, Hit::Row(0))?;
  middle(&mut session, at)?;
  // A double click splits once; its second press is dropped.
  let again =
    mouse(&mut session, MouseEventKind::Down(MouseButton::Middle), at)?;
  assert!(!again, "the repeat is dropped");
  mouse(&mut session, MouseEventKind::Up(MouseButton::Middle), at)?;
  session.settle()?;
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
    let hover = session.app().hover.hit();
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
  let edge = (default - 2, 5);
  // Motion onto the border draws its shape.
  assert!(left(&mut session, MouseEventKind::Moved, (default - 3, 5)));
  for column in [default - 2, default - 1] {
    left(&mut session, MouseEventKind::Moved, (column, 5));
  }
  let selected =
    session.app().panel.as_ref().map(|panel| panel.view.selected());
  // (drag to column) -> panel columns
  let cases = [(30, 31), (2, 8), (115, 109)];
  left(&mut session, Down(L), edge);
  for (column, want) in cases {
    left(&mut session, Drag(L), (column, 9));
    assert_eq!(session.app().left, want, "to {column}");
  }
  left(&mut session, Up(L), (115, 9));
  let panel = session.app().panel.as_ref().map(|panel| panel.view.selected());
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
  left(&mut session, MouseEventKind::Drag(MouseButton::Middle), (12, 34));
  assert!(session.app().grabbing(), "the pan holds");
  left(&mut session, MouseEventKind::Down(MouseButton::Left), (5, 19));
  left(&mut session, MouseEventKind::Drag(MouseButton::Left), (5, 10));
  left(&mut session, MouseEventKind::Up(MouseButton::Middle), (5, 30));
  assert_eq!(layout(&mut session), before);
  assert!(!session.app().grabbing());
  Ok(())
}

/// Focused pane cell where `region` hits `want`.
fn find_cell(
  session: &mut Session,
  region: Region,
  want: Hit,
) -> Result<(u16, u16)> {
  let app = session.app();
  let rows = 0..app.screen.rows;
  crate::harness::cell_where(app, region, rows, |hit| hit == want)
}

/// The focused pane's screen cell showing `item`.
fn chrome_cell(
  session: &mut Session,
  item: vpdf_ui::chrome::Item,
) -> Result<(u16, u16)> {
  let app = session.app();
  let row = app.screen.rows - 1;
  let column =
    (0..app.screen.columns).find(|c| app.chrome_at(*c, row) == Some(item));
  let column =
    column.ok_or_else(|| anyhow::anyhow!("no cell shows {item:?}"))?;
  Ok((app.at.column + column, app.at.row + row))
}

/// A settled left click.
fn click(session: &mut Session, at: (u16, u16)) {
  left(session, MouseEventKind::Down(MouseButton::Left), at);
  left(session, MouseEventKind::Up(MouseButton::Left), at);
}

#[test]
fn status_items_act_on_click() -> Result<()> {
  use vpdf_ui::chrome::Item as Chrome;
  let (directory, mut session) = session(false)?;
  for name in ["b.pdf", "c.pdf"] {
    let other = directory.path().join(name);
    crate::common::text_pdf(&other, &["Other words"])?;
    session.open(&other, true, screen())?;
  }
  status(&mut session)?;
  // Item to item motion draws nothing.
  let first = chrome_cell(&mut session, Chrome::Tab(1))?;
  let zero = chrome_cell(&mut session, Chrome::Tab(0))?;
  assert!(left(&mut session, MouseEventKind::Moved, (0, 0)));
  assert!(left(&mut session, MouseEventKind::Moved, zero));
  assert!(!left(&mut session, MouseEventKind::Moved, first));
  // Held drags keep middle presses off tabs.
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
  assert!((session.app().layout.zoom() - 1.0).abs() < 0.01);
  status(&mut session)?;
  let pages = chrome_cell(&mut session, Chrome::Pages)?;
  click(&mut session, pages);
  assert!(session.app().overlays.overview().is_some());
  status(&mut session)?;
  assert!(chrome_cell(&mut session, Chrome::Pages).is_err(), "inert in gp");
  // Sections open outlines; notes hide them.
  let (_directory, mut session) = crate::mouse::session(false)?;
  session.app().command("2")?;
  status(&mut session)?;
  let section = chrome_cell(&mut session, Chrome::Section)?;
  session.app().reader.notice("A note over the section");
  status(&mut session)?;
  click(&mut session, section);
  assert!(session.app().panel.is_none(), "the click clears the note");
  status(&mut session)?;
  let section = chrome_cell(&mut session, Chrome::Section)?;
  click(&mut session, section);
  let app = session.app();
  let panel =
    app.panel.as_ref().map(|p| (p.view.focused(), Some(p.view.selected())));
  assert_eq!(panel, Some((true, app.outline.current())));
  Ok(())
}

/// U32: tab names yield to a long detail and return on the status row.
#[test]
fn tab_names_yield_to_hover_details() -> Result<()> {
  // Fits beside the file name, not beside all three tabs.
  let uri = format!("https://example.com/{}", "a".repeat(60));
  let target = format!("/A << /S /URI /URI ({uri}) >>");
  let write = |path: &std::path::Path| {
    let links = [(crate::mouse::LINES[0], target.as_str())];
    crate::mouse::lines_pdf(path, links.into_iter())
  };
  let (directory, mut session) = crate::mouse::open(write, false)?;
  for name in ["b.pdf", "c.pdf"] {
    let other = directory.path().join(name);
    crate::common::text_pdf(&other, &["Other words"])?;
    session.open(&other, true, screen())?;
  }
  session.activate(0)?;
  status(&mut session)?;
  let link = cell(&mut session, Hit::Link(PageId(0), 0))?;
  assert!(left(&mut session, MouseEventKind::Moved, link));
  let drawn = status(&mut session)?;
  assert!(drawn.contains(&format!("→ {uri}")), "{drawn:?}");
  assert!(!drawn.contains("2:b.pdf"), "names yield: {drawn:?}");
  // On the status row the names return and take clicks.
  let tab = chrome_cell(&mut session, vpdf_ui::chrome::Item::Tab(1))?;
  assert!(left(&mut session, MouseEventKind::Moved, tab));
  assert_eq!(session.app().hover.hit(), Hit::Chrome);
  assert!(status(&mut session)?.contains("2:b.pdf"));
  click(&mut session, tab);
  assert_eq!(session.active, 1);
  Ok(())
}

#[test]
fn unfocused_status_items_activate() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("vsplit")?;
  session.settle()?;
  status(&mut session)?;
  // That item in the same-sized pane.
  let pages = chrome_cell(&mut session, vpdf_ui::chrome::Item::Pages)?;
  let index = (session.tab().focus + 1) % 2;
  let (here, there) = (session.app().at, pane(&mut session, index).at);
  let at = (pages.0 - here.column + there.column, pages.1);
  let id = pane(&mut session, index).id;
  click(&mut session, at);
  assert_eq!(session.app().id, id);
  assert!(session.app().overlays.overview().is_some());
  Ok(())
}

#[test]
fn status_items_beat_the_divider() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("split")?;
  session.settle()?;
  status(&mut session)?;
  let before = layout(&mut session);
  let pages = chrome_cell(&mut session, vpdf_ui::chrome::Item::Pages)?;
  assert_eq!(pages.1, 19, "the upper pane's status row");
  assert!(left(&mut session, MouseEventKind::Moved, pages));
  left(&mut session, MouseEventKind::Down(MouseButton::Left), pages);
  left(&mut session, MouseEventKind::Drag(MouseButton::Left), (pages.0, 9));
  left(&mut session, MouseEventKind::Up(MouseButton::Left), (pages.0, 9));
  assert!(session.app().overlays.overview().is_some());
  assert_eq!(layout(&mut session), before);
  Ok(())
}

/// The screen cell of an overlay item.
fn item_cell(
  session: &mut Session,
  list: vpdf_ui::pointer::List,
  index: usize,
) -> Result<(u16, u16)> {
  find_cell(session, Region::Overlay, Hit::Item(list, Some(index)))
}

#[test]
fn clues_and_completions_act_on_click() -> Result<()> {
  use vpdf_ui::pointer::List;
  let (_directory, mut session) = session(false)?;
  // Clues shown during a held pan.
  let page = cell(&mut session, Hit::Text)?;
  let pan = (page.0 + 2, page.1);
  left(&mut session, MouseEventKind::Down(MouseButton::Middle), page);
  left(&mut session, MouseEventKind::Drag(MouseButton::Middle), pan);
  press(&mut session, b"\x17")?;
  crate::common::age_clues(session.app());
  assert!(session.app().clue_due(std::time::Instant::now()));
  let clues =
    session.app().overlays.clues().shown().map(|c| c.items().to_vec());
  let split = clues.and_then(|items| items.iter().position(|(k, _)| k == "v"));
  let app = session.app();
  app.overlays.clues_mut().scroll(isize::MAX, app.screen);
  let at = item_cell(&mut session, List::Clues, split.expect("a v clue"))?;
  // The held pan keeps clues off.
  left(&mut session, MouseEventKind::Down(MouseButton::Left), at);
  assert!(
    session.app().overlays.clues().shown().is_some()
      && session.app().grabbing()
  );
  left(&mut session, MouseEventKind::Up(MouseButton::Middle), pan);
  // The press splits, before its release.
  left(&mut session, MouseEventKind::Down(MouseButton::Left), at);
  assert_eq!(session.tab().panes.len(), 2);
  left(&mut session, MouseEventKind::Up(MouseButton::Left), at);
  assert!(session.app().overlays.clues().shown().is_none());
  // A failing clue still closes.
  let (_text, mut failing) = crate::mouse::session(true)?;
  keys(&mut failing, "g")?;
  crate::common::age_clues(failing.app());
  assert!(failing.app().clue_due(std::time::Instant::now()));
  let clues =
    failing.app().overlays.clues().shown().map(|c| c.items().to_vec());
  let pages = clues.and_then(|items| items.iter().position(|(k, _)| k == "p"));
  status(&mut failing)?;
  let at = item_cell(&mut failing, List::Clues, pages.expect("a p clue"))?;
  mouse(&mut failing, MouseEventKind::Down(MouseButton::Left), at)?;
  assert!(failing.settle().is_err(), "the session reports it");
  let app = failing.app();
  assert!(app.overlays.clues().shown().is_none());
  assert_eq!(app.keys.pending(), "");
  // Ranges name no key.
  keys(&mut session, "m")?;
  crate::common::age_clues(session.app());
  assert!(session.app().clue_due(std::time::Instant::now()));
  status(&mut session)?;
  let app = session.app();
  let rows = app.screen.rows;
  let columns = app.screen.columns;
  let items = (0..rows).flat_map(|row| (0..columns).map(move |c| (c, row)));
  let mut hits =
    items.map(|(c, row)| app.hovered(Region::Overlay, c, row, None));
  let any = hits.find(|hit| matches!(hit, Ok(Hit::Item(_, Some(_)))));
  assert!(any.is_none(), "{any:?}");
  // Blank clue rows close the clues.
  let app = session.app();
  let blank = (app.at.column + app.screen.columns - 1, app.screen.rows - 2);
  click(&mut session, blank);
  assert!(session.app().overlays.clues().shown().is_none());
  // A click picks a completion.
  keys(&mut session, ":tab")?;
  press(&mut session, b"\t")?;
  let items = session.app().overlays.prompt().and_then(|p| p.menu());
  let second = items.map(|menu| menu.items()[1].clone()).expect("a menu");
  status(&mut session)?;
  let at = item_cell(&mut session, List::Completions, 1)?;
  click(&mut session, at);
  let text = session.app().overlays.prompt().map(|p| p.text().to_owned());
  assert!(text.is_some_and(|text| text.ends_with(&second)), "{second}");
  // Blank completion rows keep the prompt.
  let app = session.app();
  let blank = (app.at.column + app.screen.columns - 1, app.screen.rows - 2);
  click(&mut session, blank);
  assert!(session.app().overlays.prompt().is_some());
  Ok(())
}

/// U9: hover highlights clues and completions as menu rows.
#[test]
fn clues_and_completions_highlight_on_hover() -> Result<()> {
  use vpdf_ui::pointer::List;
  let badge = |label: &str| format!("\x1b[0;1;7;34m {label}");
  let pointed = |label: &str| format!(" \x1b[0;4;100m{label}");
  let (_directory, mut session) = session(false)?;
  let page = cell(&mut session, Hit::Text)?;
  keys(&mut session, "g")?;
  crate::common::age_clues(session.app());
  assert!(session.app().clue_due(std::time::Instant::now()));
  status(&mut session)?;
  let app = session.app();
  let clues = app.overlays.clues().shown().expect("clues");
  let index = clues.items().iter().position(|(k, _)| k == "p").expect("p");
  let label = clues.labels()[index].clone();
  let count = clues.items().len();
  let at = item_cell(&mut session, List::Clues, index)?;
  let other = (0..count)
    .filter(|other| *other != index)
    .find_map(|other| item_cell(&mut session, List::Clues, other).ok());
  mouse(&mut session, MouseEventKind::Moved, other.expect("another clue"))?;
  assert!(!drawn(&mut session)?.contains(&badge(&label)));
  // Same shape, another item.
  assert!(mouse(&mut session, MouseEventKind::Moved, at)?, "a frame");
  assert!(drawn(&mut session)?.contains(&badge(&label)));
  assert!(mouse(&mut session, MouseEventKind::Moved, page)?, "a frame");
  assert!(!drawn(&mut session)?.contains(&badge(&label)));
  // Hover marks a completion; the chosen one and the line stay.
  press(&mut session, b"\x1b")?;
  keys(&mut session, ":tab")?;
  // Lists, then chooses the first.
  press(&mut session, b"\t\t")?;
  status(&mut session)?;
  let prompt = |session: &mut Session| {
    let prompt = session.app().overlays.prompt().expect("a prompt");
    let menu = prompt.menu().expect("a menu");
    let chosen = menu.selected().map(|index| menu.labels()[index].clone());
    (prompt.text().to_owned(), menu.labels()[1].clone(), chosen)
  };
  let (text, label, chosen) = prompt(&mut session);
  assert!(chosen.is_some(), "a chosen completion");
  let first = item_cell(&mut session, List::Completions, 0)?;
  let at = item_cell(&mut session, List::Completions, 1)?;
  mouse(&mut session, MouseEventKind::Moved, first)?;
  assert!(!drawn(&mut session)?.contains(&pointed(&label)));
  // Same shape, another item.
  assert!(mouse(&mut session, MouseEventKind::Moved, at)?, "a frame");
  let shown = drawn(&mut session)?;
  assert!(shown.contains(&pointed(&label)), "{shown:?}");
  let marked = chosen.as_ref().is_some_and(|c| shown.contains(&badge(c)));
  assert!(marked, "{shown:?}");
  assert_eq!(prompt(&mut session), (text, label, chosen));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn section_clicks_select_their_entry() -> Result<()> {
  let mut session = crate::harness::session(&corpus("paper.pdf")?, screen())?;
  crate::common::outline_loaded(session.app())?;
  session.app().command("4")?;
  status(&mut session)?;
  keys(&mut session, "go")?;
  if let Some(panel) = &mut session.app().panel {
    panel.view.choose(12);
    panel.view.focus(false);
  }
  status(&mut session)?;
  let selected = session.app().panel.as_ref().map(|p| p.view.selected());
  assert_eq!(selected, Some(12), "a stale selection");
  let section = chrome_cell(&mut session, vpdf_ui::chrome::Item::Section)?;
  click(&mut session, section);
  let app = session.app();
  let panel =
    app.panel.as_ref().map(|p| (p.view.focused(), Some(p.view.selected())));
  assert_eq!(panel, Some((true, app.outline.current())));
  assert_ne!(app.outline.current(), Some(12));
  Ok(())
}

/// The draw of the focused pane alone.
fn drawn(session: &mut Session) -> Result<String> {
  crate::common::focused_frame(session)
}

#[test]
fn scrollbars_show_near_the_edge_and_after_scrolls() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let app = session.app();
    let edge = app.screen.columns - 1;
    let hit = app.hovered(Region::Page, edge, 5, None)?;
    if text_mode {
      assert_ne!(hit, Hit::Scrollbar, "text shows no bar");
      continue;
    }
    assert_eq!(hit, Hit::Scrollbar);
    // Resting, hovering, and scrolling frames.
    // The track places in the last column.
    let track = format!(";{}H\x1b_Ga=p", edge + 1);
    assert!(!drawn(&mut session)?.contains(&track), "hidden at rest");
    session.app().hover.point(Hit::Scrollbar, None, std::time::Instant::now);
    assert!(drawn(&mut session)?.contains(&track), "shown near the edge");
    session.app().hover.point(Hit::Nothing, None, std::time::Instant::now);
    session.app().reader.seed_reader(|reader| reader.scroll += 32.0);
    assert!(drawn(&mut session)?.contains(&track), "shown after a scroll");
    // Relayouts are no scrolls.
    session.app().bar_expired(
      std::time::Instant::now() + std::time::Duration::from_secs(9),
    );
    keys(&mut session, "go")?;
    drawn(&mut session)?;
    assert_eq!(session.app().bar_until(), None, "panels show no bar");
    keys(&mut session, "q")?;
    session.app().reader.seed_reader(|reader| reader.scroll += 16.0);
    drawn(&mut session)?;
    let until = session.app().bar_until();
    let later =
      until.expect("a deadline") + std::time::Duration::from_millis(1);
    assert!(session.app().bar_expired(later));
    assert!(!session.app().bar_expired(later), "expires once");
  }
  Ok(())
}

#[test]
fn track_presses_jump_and_drags_scrub() -> Result<()> {
  use MouseEventKind::{Down, Drag, Up};
  const L: MouseButton = MouseButton::Left;
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  let edge = app.screen.columns - 1;
  let rows = app.screen.rows - 1;
  let room = app.layout.total() - app.layout.screen().height();
  // Halfway presses jump halfway, recorded.
  left(&mut session, Down(L), (edge, rows / 2));
  let scroll = session.app().reader.scroll;
  assert!((scroll / room - 0.5).abs() < 0.05, "{scroll} of {room}");
  let label = drawn(&mut session)?;
  assert!(label.contains(" p. "), "scrubs show a page label");
  // Thumb presses grip; drags move it.
  let (_directory, mut grabbed) = crate::mouse::session(false)?;
  left(&mut grabbed, Down(L), (edge, 2));
  assert_eq!(grabbed.app().reader.scroll, 0.0);
  left(&mut grabbed, Drag(L), (edge, 4));
  let total = grabbed.app().layout.total();
  let moved = 32.0 / grabbed.app().layout.screen().height() * total;
  assert!((grabbed.app().reader.scroll - moved).abs() < 0.5);
  // Moved thumbs record a jump on release.
  left(&mut grabbed, Up(L), (edge, 4));
  press(&mut grabbed, b"\x0f")?;
  assert_eq!(grabbed.app().reader.scroll, 0.0);
  // A lost release ends the scrub.
  left(&mut grabbed, Down(L), (edge, 2));
  assert!(left(&mut grabbed, MouseEventKind::Moved, (10, 10)));
  assert!(!drawn(&mut grabbed)?.contains(" p. "));
  // Drags reach the end; releases end.
  left(&mut session, Drag(L), (edge, rows));
  assert_eq!(session.app().reader.scroll, room);
  left(&mut session, Drag(L), (0, 0));
  assert_eq!(session.app().reader.scroll, 0.0, "drags off the track scrub");
  left(&mut session, Drag(L), (edge, rows));
  left(&mut session, Up(L), (edge, rows));
  assert!(!drawn(&mut session)?.contains(" p. "));
  assert_eq!(session.app().reader.selection, None, "no text selects");
  // Ctrl-o returns from the jump.
  press(&mut session, b"\x0f")?;
  assert_eq!(session.app().reader.scroll, 0.0);
  Ok(())
}

#[test]
fn long_documents_reach_both_ends() -> Result<()> {
  use MouseEventKind::{Down, Drag, Up};
  const L: MouseButton = MouseButton::Left;
  let (_directory, mut session) = crate::mouse::open(
    |path| crate::common::pages_pdf(path, &[&["Page"][..]; 60]),
    false,
  )?;
  let app = session.app();
  let (edge, rows) = (app.screen.columns - 1, app.screen.rows - 2);
  let room = app.layout.total() - app.layout.screen().height();
  let cell = f32::from(app.screen.cell_height);
  let size = app.layout.screen().height() / app.layout.total();
  assert!(size * app.layout.screen().height() < cell, "a tiny thumb");
  // (press, drag to) -> end scroll
  let cases =
    [((edge, rows), None, room), ((edge, 0), Some((edge, rows)), room)];
  for (press, drag, want) in cases {
    session.app().reader.seed_reader(|reader| reader.scroll = 0.0);
    left(&mut session, Down(L), press);
    if let Some(to) = drag {
      left(&mut session, Drag(L), to);
    }
    left(&mut session, Up(L), drag.unwrap_or(press));
    assert_eq!(session.app().reader.scroll, want, "{press:?} {drag:?}");
  }
  // Cell presses touching a straddling thumb grip.
  let height = session.app().layout.screen().height();
  let top = 18.5 * cell;
  session
    .app()
    .reader
    .seed_reader(|reader| reader.scroll = top / (height - cell) * room);
  let partial = 18;
  for row in [partial, partial + 1] {
    let before = session.app().reader.scroll;
    left(&mut session, Down(L), (edge, row));
    left(&mut session, Up(L), (edge, row));
    assert_eq!(session.app().reader.scroll, before, "row {row}");
  }
  Ok(())
}

#[test]
fn scrollbars_track_searches_follows_and_panes() -> Result<()> {
  use MouseEventKind::{Down, Drag, Moved};
  const L: MouseButton = MouseButton::Left;
  let (_directory, mut session) = session(false)?;
  let edge = session.app().screen.columns - 1;
  // New searches redraw same-count ticks.
  let hit = |page| {
    let at =
      vpdf_model::text::Position { page: PageId(page), line: 0, glyph: 0 };
    vpdf_search::search::Hit { first: at, last: at, excerpt: String::new() }
  };
  // Last-column placements: rows and crops.
  let column = format!(";{}H\x1b_Ga=p", edge + 1);
  let track = |frame: String| -> Vec<String> {
    let parts = frame.split("\x1b[").filter(|part| part.contains(&column));
    parts
      .map(|part| part.split(",i=").next().unwrap_or("").to_owned())
      .collect()
  };
  session.app().hover.point(Hit::Scrollbar, None, std::time::Instant::now);
  let query = vpdf_search::query::Query::default();
  session.app().search.show(query.clone(), vec![hit(0)]);
  let first = track(drawn(&mut session)?);
  session.app().search.show(query, vec![hit(1)]);
  assert_ne!(track(drawn(&mut session)?), first, "ticks follow the search");
  session.app().hover.point(Hit::Nothing, None, std::time::Instant::now);
  // Following an outline entry shows the bar.
  session
    .app()
    .bar_expired(std::time::Instant::now() + std::time::Duration::from_secs(9));
  keys(&mut session, "+++")?;
  drawn(&mut session)?;
  keys(&mut session, "go")?;
  drawn(&mut session)?;
  assert_eq!(session.app().bar_until(), None, "panels show no bar");
  keys(&mut session, "q=")?;
  // Pinned thumbs' labels follow the pointer.
  session.app().reader.seed_reader(|reader| reader.scroll = 0.0);
  left(&mut session, Down(L), (edge, 3));
  assert!(left(&mut session, Drag(L), (edge, 0)), "the label moves");
  // Motionless presses end lost scrubs.
  left(&mut session, Down(L), (5, 5));
  assert!(!drawn(&mut session)?.contains(" p. "));
  // Presses elsewhere end this pane's scrub.
  session.app().command("vsplit")?;
  session.settle()?;
  let app = session.app();
  let edge = app.at.column + app.screen.columns - 1;
  let id = app.id;
  left(&mut session, Down(L), (edge, 3));
  left(&mut session, Moved, (edge, 3));
  left(&mut session, Down(L), (edge + 10, 5));
  let tab = session.tab();
  tab.focus = tab.panes.iter().position(|app| app.id == id).unwrap();
  let label = drawn(&mut session)?;
  assert!(!label.contains(" p. "), "blurred panes end scrubs");
  Ok(())
}

#[test]
fn outline_follows_show_the_scrollbar() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  app.layout.request_zoom(vpdf_ui::layout::Zoom::To(3.0));
  app.relayout()?;
  keys(&mut session, "go")?;
  drawn(&mut session)?;
  let start = session.app().reader.scroll;
  let later = std::time::Instant::now() + std::time::Duration::from_secs(9);
  session.app().bar_expired(later);
  press(&mut session, b"\r")?;
  session.settle()?;
  drawn(&mut session)?;
  let app = session.app();
  assert_ne!(app.reader.scroll.to_bits(), start.to_bits(), "the row moves");
  assert!(app.bar_until().is_some(), "followed rows show the bar");
  Ok(())
}

#[test]
fn match_count_opens_results() -> Result<()> {
  for text in [false, true] {
    let (_directory, mut session) = session(text)?;
    session.app().start_search("results", false, false)?;
    crate::common::search_loaded(session.app())?;
    let at = chrome_cell(&mut session, vpdf_ui::chrome::Item::Matches)?;
    mouse(&mut session, MouseEventKind::Moved, at)?;
    assert_eq!(session.app().hover.hit(), Hit::Chrome);
    click(&mut session, at);
    let panel = session.app().panel.as_ref().unwrap();
    assert_eq!(panel.view.kind(), vpdf_ui::keys::PanelKind::Results);
    assert!(panel.view.focused());
  }
  Ok(())
}

#[test]
fn refocusing_links_refreshes_page() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  app.open_panel(vpdf_ui::keys::PanelKind::Links)?;
  crate::common::sidebar_loaded(app, vpdf_ui::keys::PanelKind::Links)?;
  app.panel.as_mut().unwrap().view.choose(1);
  app.open_panel(vpdf_ui::keys::PanelKind::Links)?;
  crate::common::sidebar_loaded(app, vpdf_ui::keys::PanelKind::Links)?;
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), 1);
  crate::common::keys(app, b"\ti")?;
  app.command("2")?;
  app.open_panel(vpdf_ui::keys::PanelKind::Links)?;
  crate::common::sidebar_loaded(app, vpdf_ui::keys::PanelKind::Links)?;
  assert!(app.panel.as_ref().unwrap().view.rows().is_empty());
  Ok(())
}

#[test]
fn unfocused_matches_keep_identity() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("vsplit")?;
  session.settle()?;
  let index = (session.tab().focus + 1) % 2;
  let app = pane(&mut session, index);
  app.start_search("results", false, false)?;
  crate::common::search_loaded(app)?;
  let row = app.screen.rows - 1;
  let column = (0..app.screen.columns)
    .find(|&column| {
      app.chrome_at(column, row) == Some(vpdf_ui::chrome::Item::Matches)
    })
    .unwrap();
  let at = (app.at.column + column, app.at.row + row);
  let id = app.id;
  click(&mut session, at);
  assert_eq!(session.app().id, id);
  assert_eq!(
    session.app().panel.as_ref().unwrap().view.kind(),
    vpdf_ui::keys::PanelKind::Results
  );
  Ok(())
}

#[test]
fn split_mirrors_text_preview() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("text")?;
  session.app().command("vsplit")?;
  session.settle()?;
  for app in &mut session.tab().panes {
    assert!(app.text_mode);
    assert_eq!(app.preview, Some(false));
    crate::common::keys(app, b"q")?;
    assert!(!app.text_mode && !app.quit);
    assert!(app.preview.is_none());
  }
  Ok(())
}

#[test]
fn gx_reuses_url_scheme_checks() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  crate::common::keys(app, b"igx")?;
  assert_eq!(app.reader.cursor.page, PageId(0));
  assert!(!app.text_mode);
  app.show_panel(
    vpdf_ui::keys::PanelKind::Links,
    vec![vpdf_ui::panel::Row::new(
      "unsafe".into(),
      String::new(),
      vpdf_ui::panel::Destination::Link(vpdf_model::document::Target::Uri(
        "file:///etc/passwd".into(),
      )),
    )],
  )?;
  assert!(
    crate::common::keys(app, b"gx")
      .unwrap_err()
      .to_string()
      .contains("Unsupported link scheme")
  );
  app.command("text")?;
  assert!(app.text_mode);
  Ok(())
}

#[test]
fn panel_yank_keeps_named_register() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  crate::common::keys(app, b"go\"ay")?;
  assert!(app.reader.pending_yank.is_none());
  crate::common::keys(app, b"y")?;
  let yank = app.reader.take_yank().unwrap();
  assert_eq!(yank.register, 'a');
  assert_eq!(yank.text, "Results\n");
  assert!(app.reader.operator.is_none());
  assert_eq!(app.keys.pending(), "");
  Ok(())
}
