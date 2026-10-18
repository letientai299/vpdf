//! The extracted-text view draws on across page breaks.

use crate::common;
use crossterm::event::{
  KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::{Heading, Target};
use vpdf::layout::Screen;
use vpdf::pointer::{Hit, Region, Route};
use vpdf::reader::SelectionKind;
use vpdf::text::{PageId, Position};

const PAGES: [[&str; 4]; 3] = [
  ["P0 L0 alpha", "P0 L1 beta", "P0 L2 gamma", "P0 L3 delta"],
  ["P1 L0 alpha", "P1 L1 beta", "P1 L2 gamma", "P1 L3 delta"],
  ["P2 L0 alpha", "P2 L1 beta", "P2 L2 gamma", "P2 L3 delta"],
];

const SELECTION: &str = "\x1b[0;7;35m";
const MATCH: &str = "\x1b[0;7;33m";
const CURSORLINE: &str = "\x1b[0;100m";
const LINK: &str = "\x1b[0;4;36m";

const WIDE: &str = "P1 L0 a wide line";

/// Three pages of four lines in an eight-row text view, with web
/// links over `(page, line)`s.
fn app(links: &[(usize, usize)]) -> Result<(tempfile::TempDir, App)> {
  let pages = PAGES.each_ref().map(|lines| &lines[..]);
  app_of(&pages, links)
}

/// `pages` in an eight-row text view.
fn app_of(
  pages: &[&[&str]],
  links: &[(usize, usize)],
) -> Result<(tempfile::TempDir, App)> {
  let dir = tempfile::tempdir()?;
  let path = dir.path().join("pages.pdf");
  common::linked_pages_pdf(&path, pages, links)?;
  let screen = Screen {
    columns: 40,
    rows: 9,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let app = App::with_screen(&path, "", true, screen)?;
  Ok((dir, app))
}

fn on(page: usize, line: usize, glyph: usize) -> Position {
  Position { page: PageId(page), line, glyph }
}

/// The text view's rows as drawn, styles kept.
fn drawn(app: &mut App) -> Result<Vec<String>> {
  let mut out = Vec::new();
  app.text_canvas(&mut out)?;
  let out = String::from_utf8(out)?;
  let mut rows = vec![String::new(); usize::from(app.screen.rows - 1)];
  let mut current = 0;
  for part in out.split("\x1b[").skip(1) {
    let goto = part.split_once('H').and_then(|(at, rest)| {
      let (row, column) = at.split_once(';')?;
      let row = row.parse::<usize>().ok()?.checked_sub(1)?;
      column.parse::<usize>().ok().map(|_| (row, rest))
    });
    match goto {
      Some((row, rest)) => {
        current = row;
        rows[row].push_str(rest);
      }
      None => {
        rows[current].push_str("\x1b[");
        rows[current].push_str(part);
      }
    }
  }
  Ok(rows)
}

/// Rows scrolled down from the top, as the wheel does.
fn scrolled(rows: i16) -> Result<(tempfile::TempDir, App)> {
  let (dir, mut app) = app(&[])?;
  app.scroll(f32::from(rows) * 16.0)?;
  Ok((dir, app))
}

#[test]
fn rows_show_later_pages_after_a_break() -> Result<()> {
  let (_dir, mut app) = app(&[])?;
  let rows = drawn(&mut app)?;
  assert!(rows[3].contains("P0 L3"), "{rows:?}");
  assert!(rows[4].contains("── 2 ─"), "{rows:?}");
  assert!(rows[5].contains("P1 L0"), "{rows:?}");
  for (name, position, cell) in [
    ("first page", on(0, 3, 0), Some((0, 3))),
    ("after the break", on(1, 0, 1), Some((1, 5))),
    ("last row", on(1, 2, 0), Some((0, 7))),
    ("below the view", on(1, 3, 0), None),
  ] {
    assert_eq!(app.text_cell(position)?, cell, "{name}");
  }
  let end = app.doc.text(PageId(0))?.lines[3].glyphs.len() - 1;
  for (name, (column, row), position, hit) in [
    ("next page", (1, 5), on(1, 0, 1), Hit::Text),
    ("the break", (1, 4), on(0, 3, end), Hit::Nothing),
  ] {
    assert_eq!(app.text_position(column, row)?, position, "{name}");
    assert_eq!(app.hovered(Region::Page, column, row, None)?, hit, "{name}");
  }
  Ok(())
}

#[test]
fn scrolling_steps_rows_through_breaks() -> Result<()> {
  for (rows, cell) in
    [(1, Some((0, 4))), (4, Some((0, 1))), (5, Some((0, 0))), (6, None)]
  {
    let (_dir, mut app) = scrolled(rows)?;
    assert_eq!(app.text_cell(on(1, 0, 0))?, cell, "{rows} rows");
    app.scroll(-f32::from(rows) * 16.0)?;
    assert_eq!(app.text_cell(on(1, 0, 0))?, Some((0, 5)), "{rows} back");
  }
  // The last line stays at the bottom.
  let (_dir, mut app) = scrolled(100)?;
  assert_eq!(app.text_cell(on(2, 3, 0))?, Some((0, 7)));
  app.screen.rows = 13;
  app.relayout()?;
  assert_eq!(app.text_cell(on(2, 3, 0))?, Some((0, 11)), "taller");
  Ok(())
}

#[test]
fn marks_and_bands_cross_breaks() -> Result<()> {
  let (_dir, mut app) = app(&[])?;
  // (kind, style past the cursor on its line)
  for (kind, after) in
    [(SelectionKind::Character, CURSORLINE), (SelectionKind::Line, SELECTION)]
  {
    app.reader.cursor = on(0, 2, 0);
    app.reader.visual(kind);
    app.reader.cursor = on(1, 1, 2);
    app.cursorline = true;
    let rows = drawn(&mut app)?;
    assert!(rows[4].starts_with(&format!("{SELECTION}──")), "{rows:?}");
    assert!(rows[6].contains(SELECTION), "{rows:?}");
    assert!(rows[6].starts_with(CURSORLINE), "{rows:?}");
    let tail = format!("{after}L1 beta\x1b[0m");
    assert!(rows[6].ends_with(&tail), "{kind:?}: {rows:?}");
    assert!(!rows[7].contains(SELECTION), "{rows:?}");
    app.reader.end_selection();
  }
  app.start_search("gamma", false, false)?;
  common::search_loaded(&mut app)?;
  let rows = drawn(&mut app)?;
  let marked = rows.iter().filter(|row| row.contains(&format!("{MATCH}gamma")));
  assert_eq!(marked.count(), 2, "{rows:?}");
  Ok(())
}

#[test]
fn hints_and_tints_reach_later_pages() -> Result<()> {
  let (_dir, mut app) = app(&[(0, 0), (1, 1)])?;
  drawn(&mut app)?;
  app.start_hints(false)?;
  let hints = app.hints.take().expect("hints");
  let rows: Vec<_> = hints.items.iter().map(|hint| hint.cell.1).collect();
  assert_eq!(rows, [0, 6]);
  let hit = app.hovered(Region::Page, 2, 6, None)?;
  assert_eq!(hit, Hit::Link(PageId(1), 0));
  app.hover = hit;
  let rows = drawn(&mut app)?;
  assert!(rows[6].contains(&format!("{LINK}P1 L1")), "{rows:?}");
  assert!(!rows[0].contains(LINK), "{rows:?}");
  Ok(())
}

#[test]
fn the_top_row_names_page_and_section() -> Result<()> {
  for (rows, section, position) in
    [(3, 0, on(0, 3, 0)), (4, 0, on(1, 0, 0)), (5, 1, on(1, 0, 0))]
  {
    let (_dir, mut app) = scrolled(rows)?;
    for page in 0..2 {
      let glyph = app.doc.text(PageId(page))?.lines[0].glyphs[0].bounds;
      let (x, y) = glyph.centre();
      app.doc.outline.push(Heading {
        title: format!("Page {page}"),
        depth: 0,
        target: Target::Point { page: PageId(page), x, y },
      });
    }
    assert_eq!(app.section(), Some(section), "{rows} rows");
    assert_eq!(app.reading_position()?, position, "{rows} rows");
  }
  Ok(())
}

#[test]
fn middle_drags_move_text_by_whole_rows() -> Result<()> {
  let (_dir, mut app) = app(&[])?;
  let pane = app.id;
  let press = |kind, row| Route {
    pane,
    region: Region::Page,
    focus: true,
    mouse: MouseEvent { kind, column: 5, row, modifiers: KeyModifiers::NONE },
    clicks: 1,
    pixel: None,
  };
  app.mouse(press(MouseEventKind::Down(MouseButton::Middle), 6))?;
  // (drag to row) -> row of the second page's first line
  for (row, shown) in [(6, 5), (4, 3), (1, 0), (2, 1), (6, 5), (8, 5)] {
    app.mouse(press(MouseEventKind::Drag(MouseButton::Middle), row))?;
    assert_eq!(app.text_cell(on(1, 0, 0))?, Some((0, shown)), "row {row}");
  }
  assert!(app.grabbing());
  app.mouse(press(MouseEventKind::Up(MouseButton::Middle), 8))?;
  assert!(!app.grabbing());
  Ok(())
}

fn key(app: &mut App, ch: char) -> Result<()> {
  let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
  app.key(key, &mut vpdf::prompt::History::default())
}

/// What a case does to the view.
#[derive(Clone, Copy, Debug)]
enum Act {
  Go(Position),
  Land(Position),
  Key(char),
  /// Searches "gamma", then presses `n` this often.
  Search(usize),
  End,
}

fn act(app: &mut App, action: Act) -> Result<()> {
  match action {
    Act::Go(to) => app.go_to(to, false),
    Act::Land(to) => app.land_at(to),
    Act::Key(ch) => key(app, ch),
    Act::Search(next) => {
      app.start_search("gamma", false, false)?;
      common::search_loaded(app)?;
      (0..next).try_for_each(|_| key(app, 'n'))
    }
    Act::End => app.scroll_text(100),
  }
}

/// Motions to drawn lines stay; jumps show the page top.
#[test]
fn jumps_move_least_to_show_targets() -> Result<()> {
  let [p0, p1, p2] = PAGES.each_ref().map(|lines| &lines[..]);
  let full: [&[&str]; 5] = [p0, p1, p2, p0, p1];
  let holes: [&[&str]; 5] = [p0, &[], p1, p2, p0];
  // (pages, top row, Cursor mode at, act, cursor, its row)
  let cases = [
    (&full, 0, None, Act::Go(on(1, 0, 0)), on(1, 0, 0), 0),
    (&full, 0, None, Act::Go(on(2, 0, 0)), on(2, 0, 0), 0),
    (&full, 0, Some(on(0, 3, 0)), Act::Key('j'), on(1, 0, 0), 5),
    (&full, 0, Some(on(1, 0, 0)), Act::Key('k'), on(0, 3, 0), 3),
    (&full, 2, Some(on(1, 3, 0)), Act::Key('j'), on(2, 0, 0), 0),
    (&holes, 0, Some(on(0, 3, 0)), Act::Key('j'), on(1, 0, 0), 5),
    (&holes, 0, Some(on(2, 0, 0)), Act::Key('k'), on(1, 0, 0), 5),
    (&full, 0, None, Act::Search(0), on(0, 2, 6), 2),
    (&full, 0, None, Act::Search(1), on(1, 2, 6), 2),
    (&full, 0, None, Act::Search(2), on(2, 2, 6), 2),
  ];
  for (pages, top, at, action, cursor, row) in cases {
    let (_dir, mut app) = app_of(pages, &[])?;
    app.scroll_text(top)?;
    if let Some(at) = at {
      key(&mut app, 'i')?;
      app.reader.cursor = at;
    }
    act(&mut app, action)?;
    assert_eq!(app.reader.cursor, cursor, "{action:?} from {at:?}");
    let drawn = app.text_row(cursor)?;
    assert_eq!(drawn, Some(row), "{action:?} from {at:?}");
  }
  Ok(())
}

/// Pages without text show a blank row under their rule.
#[test]
fn empty_pages_take_a_blank_row() -> Result<()> {
  let [p0, p1, p2] = PAGES.each_ref().map(|lines| &lines[..]);
  let middle: [&[&str]; 5] = [p0, p1, &[], p2, p0];
  let last: [&[&str]; 5] = [p0, p1, p2, p0, &[]];
  // (pages, act, rows with the third and fourth pages' rules)
  for (pages, action, rules) in [
    (&middle, Act::Go(on(2, 0, 0)), (None, Some(1))),
    (&middle, Act::Land(on(2, 0, 0)), (Some(0), Some(2))),
    (&last, Act::End, (None, Some(1))),
  ] {
    let (_dir, mut app) = app_of(pages, &[])?;
    act(&mut app, action)?;
    let rows = drawn(&mut app)?;
    let rule =
      |page| rows.iter().position(|row| row.contains(&format!("── {page} ")));
    assert_eq!((rule(3), rule(4)), rules, "{action:?}: {rows:?}");
  }
  Ok(())
}

/// One column offset pans every page's rows.
#[test]
fn panning_spans_pages() -> Result<()> {
  let [p0, _, p2] = PAGES.each_ref().map(|lines| &lines[..]);
  let wide = [WIDE, "P1 L1 beta", "P1 L2 gamma", "P1 L3 delta"];
  let (_dir, mut app) = app_of(&[p0, &wide, p2], &[])?;
  app.screen.columns = 12;
  app.relayout()?;
  let pane = app.id;
  let press = |kind, column| Route {
    pane,
    region: Region::Page,
    focus: true,
    mouse: MouseEvent { kind, column, row: 2, modifiers: KeyModifiers::NONE },
    clicks: 1,
    pixel: None,
  };
  app.mouse(press(MouseEventKind::Down(MouseButton::Middle), 10))?;
  app.mouse(press(MouseEventKind::Drag(MouseButton::Middle), 7))?;
  assert_eq!(app.text_cell(on(1, 0, 3))?, Some((0, 5)), "drag");
  assert_eq!(app.text_cell(on(0, 0, 3))?, Some((0, 0)), "drag");
  // The widest drawn line bounds the offset.
  app.mouse(press(MouseEventKind::Drag(MouseButton::Middle), 0))?;
  assert_eq!(app.text_cell(on(1, 0, 5))?, Some((0, 5)), "bound");
  app.mouse(press(MouseEventKind::Up(MouseButton::Middle), 0))?;
  // Scrolling the wide line away resets it.
  app.scroll(6.0 * 16.0)?;
  assert_eq!(app.text_cell(on(1, 3, 0))?, Some((0, 2)), "scrolled");
  Ok(())
}

/// Views too short for rows read nothing past the ends.
#[test]
fn rowless_views_move_safely() -> Result<()> {
  let (_dir, mut app) = app(&[])?;
  app.screen.rows = 1;
  app.relayout()?;
  app.scroll_text(5)?;
  app.go_to(on(2, 3, 0), false)?;
  app.land_at(on(1, 2, 0))?;
  assert_eq!(drawn(&mut app)?.len(), 0);
  assert_eq!(app.text_cell(on(1, 2, 0))?, None);
  assert_eq!(app.reader.cursor, on(1, 2, 0));
  Ok(())
}

/// Presses on a rule miss, as gaps between pages do.
#[test]
fn presses_on_rules_select_nothing() -> Result<()> {
  let (_dir, mut app) = app(&[])?;
  drawn(&mut app)?;
  let pane = app.id;
  let press = |kind, row| Route {
    pane,
    region: Region::Page,
    focus: true,
    mouse: MouseEvent { kind, column: 2, row, modifiers: KeyModifiers::NONE },
    clicks: 1,
    pixel: None,
  };
  app.mouse(press(MouseEventKind::Down(MouseButton::Left), 4))?;
  app.mouse(press(MouseEventKind::Up(MouseButton::Left), 4))?;
  assert_eq!(app.target, None);
  app.mouse(press(MouseEventKind::Down(MouseButton::Left), 3))?;
  assert_eq!(app.target, Some(on(0, 3, 2)));
  Ok(())
}

/// Rules name the next page by its label.
#[test]
fn rules_show_page_labels() -> Result<()> {
  use mupdf::pdf::{PageLabelRule, PageLabelStyle, PdfWriteOptions};
  let (dir, app) = app(&[])?;
  let path = dir.path().join("labeled.pdf");
  let mut doc = vpdf::document::Document::open(&app.path, "")?;
  let rule = PageLabelRule::new(0, PageLabelStyle::LowerRoman);
  doc.native.set_page_label_rule(rule)?;
  let mut file = std::fs::File::create(&path)?;
  doc.native.write_to_with_options(&mut file, PdfWriteOptions::default())?;
  let mut app = App::with_screen(&path, "", true, app.screen)?;
  let rows = drawn(&mut app)?;
  assert!(rows[4].contains("── ii ─"), "{rows:?}");
  Ok(())
}
