use crate::common;
use crate::focus::screen;
use crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::decode::{Event, Parser};
use vpdf::keys::{Action, Input};
use vpdf::motion::Motion;
use vpdf::pointer::Pixel;
use vpdf::session::Session;
use vpdf::text::PageId;

/// SGR wheel flags: 64 up, 65 down, 66 left, 67 right; 4 Shift, 16 Ctrl.
const UP: u16 = 64;
const DOWN: u16 = 65;
const LEFT: u16 = 66;
const RIGHT: u16 = 67;
const SHIFT: u16 = 4;
const CTRL: u16 = 16;
/// Bare motion.
const MOTION: u16 = 35;

/// An SGR report at screen pixel `(x, y)` under mode 1016, arriving at
/// `now`; whether it asked for a frame.
fn wheel_at(
  session: &mut Session,
  flags: u16,
  (x, y): (u16, u16),
  now: Instant,
) -> Result<bool> {
  let mut parser = Parser::default();
  parser.set_pixels(true);
  let mut events = VecDeque::new();
  parser.feed(format!("\x1b[<{flags};{x};{y}M").as_bytes(), &mut events);
  let mut frame = false;
  for event in events {
    let Event::Mouse(mouse) = event else { panic!("mouse bytes") };
    frame |= session.mouse_report_at(mouse, now)?;
  }
  Ok(frame)
}

/// A turn that starts a gesture: the pointer moves there first.
fn wheel(session: &mut Session, flags: u16, at: (u16, u16)) -> Result<bool> {
  let now = Instant::now();
  wheel_at(session, MOTION, at, now)?;
  wheel_at(session, flags, at, now)
}

/// `pages` of Helvetica, each `(size, lines)`, 300 pt wide and as tall
/// as its lines; page 1's `link` line links to page 2.
fn pages_pdf(
  path: &Path,
  pages: &[(u32, usize)],
  link: Option<usize>,
) -> Result<()> {
  let count = pages.len();
  let kids = (0..count).fold(String::new(), |mut kids, page| {
    let _ = write!(kids, "{} 0 R ", 4 + 2 * page);
    kids
  });
  let mut objects = vec![
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    format!("<< /Type /Pages /Kids [{kids}] /Count {count} >>"),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
  ];
  for (page, &(size, lines)) in pages.iter().enumerate() {
    let leading = size + size / 3;
    let height = 20 + leading as usize * lines.max(1);
    let mut content =
      format!("BT /F1 {size} Tf {leading} TL 20 {} Td", height - 20);
    for line in 0..lines {
      write!(content, " (Line {line} of page {}) Tj T*", page + 1)?;
    }
    content.push_str(" ET");
    let annots = if link.is_some() && page == 0 {
      format!("/Annots [{} 0 R]", 4 + 2 * count)
    } else {
      String::new()
    };
    objects.push(format!(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 {height}] \
       /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R {annots} >>",
      5 + 2 * page
    ));
    objects.push(format!(
      "<< /Length {} >>\nstream\n{content}\nendstream",
      content.len()
    ));
  }
  if let Some(line) = link {
    let (size, lines) = pages[0];
    let leading = (size + size / 3) as usize;
    let top = 20 + leading * lines.max(1) - leading * line;
    objects.push(format!(
      "<< /Type /Annot /Subtype /Link /Rect [15 {} 200 {}] \
       /Dest [6 0 R /XYZ 0 2000 0] >>",
      top - 30,
      top
    ));
  }
  common::write_pdf(path, &objects, "")
}

fn session(path: &Path) -> Result<Session> {
  let mut session = Session::new(None);
  session.open(path, true, screen())?;
  Ok(session)
}

/// The page point under a screen pixel of the only pane.
fn under(app: &App, (x, y): (u16, u16)) -> (usize, f32, f32) {
  let pixel = Some(Pixel { x, y, cell: (8, 16) });
  let (page, x, y) = app.page_point(x / 8, y / 16, pixel).expect("a page");
  (page.0, x, y)
}

#[test]
fn ctrl_wheel_zooms_at_the_pointer() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("pages.pdf");
  pages_pdf(&path, &[(18, 6), (18, 6), (18, 6)], None)?;
  // (pointer pixel, turns before, the turn, scroll first) -> zoom grows;
  // horizontal anchoring holds only while the page overflows.
  let cases = [
    ((480, 300), 0, UP, 0.0, true),
    ((40, 100), 0, UP, 0.0, true),
    ((900, 500), 0, UP, 0.0, true),
    ((300, 200), 6, UP, 0.0, true),
    ((300, 200), 6, DOWN, 0.0, false),
    ((700, 450), 4, DOWN, 0.0, false),
    // A later page, across the gap under the first.
    ((480, 300), 0, UP, 900.0, true),
    ((480, 560), 2, UP, 400.0, true),
    // Below fit width the page centres; the row still holds.
    ((480, 300), 0, DOWN, 300.0, false),
  ];
  for (at, before, turn, scroll, grows) in cases {
    let mut session = session(&path)?;
    session.app().reader.scroll = scroll;
    for _ in 0..before {
      wheel(&mut session, UP | CTRL, at)?;
    }
    let app = session.app();
    let (zoom, point) = (app.layout.zoom, under(app, at));
    assert!(wheel(&mut session, turn | CTRL, at)?);
    let app = session.app();
    let label = format!("{at:?} {before} {scroll}");
    assert_eq!(app.layout.zoom > zoom, grows, "{label}");
    let after = under(app, at);
    assert_eq!(after.0, point.0, "{label}");
    let overflows = app.layout.pan_limit(PageId(after.0)) > 0.0;
    // A pixel is under half a point at fit width and above.
    assert!(
      (after.2 - point.2).abs() < 0.5
        && (!overflows || (after.1 - point.1).abs() < 0.5),
      "{label}: {point:?} -> {after:?}"
    );
    assert!(app.animation.is_none());
  }
  Ok(())
}

#[test]
fn shift_and_sideways_wheel_pan_zoomed_pages() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("lines.pdf");
  common::text_pdf(&path, &["Pan a zoomed page sideways"])?;
  let at = (480, 300);
  let notch = f32::from(vpdf::pointer::WHEEL_ROWS) * 8.0;
  // (flags) -> pan change, scrolls
  let cases = [
    (SHIFT | DOWN, notch, false),
    (SHIFT | UP, -notch, false),
    (RIGHT, notch, false),
    (LEFT, -notch, false),
    (SHIFT | RIGHT, notch, false),
    (DOWN, 0.0, true),
    // Ctrl gestures never pan off their anchor.
    (CTRL | RIGHT, 0.0, false),
    (CTRL | SHIFT | DOWN, 0.0, false),
  ];
  for (flags, change, scrolls) in cases {
    let mut session = session(&path)?;
    let moved = wheel(&mut session, flags, at)?;
    assert_eq!(session.app().layout.pan, 0.0, "fit width cannot pan");
    assert_eq!(moved, scrolls, "{flags}: frames only when moved");
    session.app().layout.zoom = 3.0;
    session.app().relayout()?;
    session.app().layout.pan = 400.0;
    let app = session.app();
    let (pan, scroll, zoom) =
      (app.layout.pan, app.reader.scroll, app.layout.zoom);
    wheel(&mut session, flags, at)?;
    let app = session.app();
    assert!((app.layout.pan - pan - change).abs() < 0.01, "{flags}");
    let target = app.animation.as_ref().map_or(scroll, |a| a.target);
    assert_eq!((target - scroll).abs() > 1.0, scrolls, "{flags}");
    assert!((app.layout.zoom - zoom).abs() < f32::EPSILON, "{flags}");
  }
  Ok(())
}

#[test]
fn sideways_drift_in_a_gesture_is_ignored() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("lines.pdf");
  common::text_pdf(&path, &["Rails for a zoomed page"])?;
  let mut session = session(&path)?;
  session.app().layout.zoom = 3.0;
  session.app().relayout()?;
  session.app().layout.pan = 400.0;
  let (at, start) = ((480, 300), Instant::now());
  assert!(wheel_at(&mut session, DOWN, at, start)?);
  let later = start + Duration::from_millis(50);
  assert!(!wheel_at(&mut session, RIGHT, at, later)?, "drift");
  assert!(!wheel_at(&mut session, SHIFT | DOWN, at, later)?, "drift");
  assert!((session.app().layout.pan - 400.0).abs() < 0.01);
  assert!(wheel(&mut session, RIGHT, at)?, "a sideways gesture pans");
  assert!(!wheel_at(&mut session, DOWN, at, Instant::now())?, "drift");
  Ok(())
}

#[test]
fn text_wheel_pans_columns_and_ignores_ctrl() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("lines.pdf");
  common::text_pdf(&path, &["A line far wider than the narrow view"])?;
  let mut session = Session::new(None);
  let narrow = vpdf::layout::Screen { columns: 16, ..screen() };
  session.open(&path, true, narrow)?;
  session.app().command("text")?;
  let first = session.app().text_position(0, 0)?;
  let zoom = session.app().layout.zoom;
  assert!(!wheel(&mut session, UP | CTRL, (36, 8))?);
  let app = session.app();
  assert!((app.layout.zoom - zoom).abs() < f32::EPSILON, "text shows no zoom");
  assert_eq!(app.text_position(0, 0)?, first);
  assert!(wheel(&mut session, RIGHT, (36, 8))?);
  let panned = session.app().text_position(0, 0)?;
  assert_eq!(panned.glyph, first.glyph + 3, "a turn pans three columns");
  Ok(())
}

#[test]
fn wheel_frames_only_when_something_moves() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("lines.pdf");
  common::text_pdf(&path, &["Frames"])?;
  let mut session = session(&path)?;
  let at = (480, 300);
  assert!(wheel(&mut session, DOWN, at)?);
  assert!(!wheel(&mut session, RIGHT, at)?, "fit width cannot pan");
  session.app().layout.zoom = 8.0;
  session.app().relayout()?;
  assert!(!wheel(&mut session, UP | CTRL, at)?, "zoom at its limit");
  assert!(wheel(&mut session, DOWN | CTRL, at)?);
  // A pending count clears, which the status line shows.
  session.app().layout.zoom = 1.0;
  session.app().relayout()?;
  let mut history = vpdf::prompt::History::default();
  let two = crossterm::event::KeyEvent::new(
    crossterm::event::KeyCode::Char('2'),
    KeyModifiers::NONE,
  );
  session.app().key(two, &mut history)?;
  assert!(wheel(&mut session, RIGHT, at)?);
  assert!(!wheel(&mut session, RIGHT, at)?);
  Ok(())
}

#[test]
fn panel_preview_and_text_turns_frame_until_an_end() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("linked.pdf");
  pages_pdf(&path, &[(18, 3), (10, 60), (18, 3)], Some(0))?;
  // A panel list: the help rows scroll down, not above the first.
  let mut panel = session(&path)?;
  panel.app().open_panel(vpdf::keys::PanelKind::Help)?;
  panel.app().draw(&mut Vec::new())?;
  assert!(!wheel(&mut panel, UP, pixel((2, 5)))?);
  assert!(wheel(&mut panel, DOWN, pixel((2, 5)))?);
  // The preview scrolls down, not above its top.
  let mut preview = linked(&path, false)?;
  let frame = drawn(&mut preview)?;
  let inside = pixel((frame.column + 2, frame.row + 2));
  assert!(!wheel(&mut preview, UP, inside)?);
  assert!(wheel(&mut preview, DOWN, inside)?);
  // Text scrolls rows; short lines do not pan.
  let mut text = session(&path)?;
  text.app().command("text")?;
  assert!(!wheel(&mut text, UP, (480, 300))?);
  assert!(!wheel(&mut text, RIGHT, (480, 300))?);
  assert!(wheel(&mut text, DOWN, (480, 300))?);
  Ok(())
}

#[test]
fn overview_wheel_scrolls_rows_and_keeps_selection() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("pages.pdf");
  pages_pdf(&path, &[(18, 6); 40], None)?;
  let mut session = session(&path)?;
  let app = session.app();
  app.open_overview()?;
  app.draw(&mut Vec::new())?;
  // (selected page, top row)
  let state = |session: &mut Session| {
    let overview = session.app().overview.as_ref().expect("an overview");
    (overview.selected, overview.top())
  };
  assert_eq!(state(&mut session), (0, 0));
  for (flags, row) in [(DOWN, 1), (DOWN, 2), (UP, 1), (DOWN | CTRL, 2)] {
    assert!(wheel(&mut session, flags, (480, 300))?);
    session.app().draw(&mut Vec::new())?;
    assert_eq!(state(&mut session), (0, row), "{flags}");
  }
  for _ in 0..40 {
    wheel(&mut session, DOWN, (480, 300))?;
  }
  session.app().draw(&mut Vec::new())?;
  let last = state(&mut session);
  assert!(!wheel(&mut session, DOWN, (480, 300))?, "the last row");
  assert_eq!(state(&mut session), last);
  // Keys reveal the selection again.
  session.app().overview_input(Input::plain(Action::Motion(Motion::Down)))?;
  // `j` moves a row of thumbnails: the selection is the column count.
  let (columns, row) = state(&mut session);
  assert_eq!(row, 1);
  // A new grid shape reveals the selection too.
  for _ in 0..5 {
    wheel(&mut session, DOWN, (480, 300))?;
  }
  assert!(state(&mut session).1 > 1);
  session.app().screen.rows = 30;
  session.app().relayout()?;
  session.app().draw(&mut Vec::new())?;
  assert_eq!(state(&mut session), (columns, 1));
  // A click opens the page under it after the wheel scrolled.
  wheel(&mut session, DOWN, (480, 300))?;
  assert_eq!(state(&mut session).1, 2);
  session.app().overview_mouse(MouseEvent {
    kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
    column: 3,
    row: 2,
    modifiers: KeyModifiers::NONE,
  })?;
  assert!(session.app().overview.is_none());
  assert_eq!(session.app().reader.cursor.page, PageId(2 * columns));
  Ok(())
}

/// A session on page 1 of `path`, which links to page 2, with that
/// link's `K` preview open.
fn linked(path: &Path, text: bool) -> Result<Session> {
  let mut session = session(path)?;
  let app = session.app();
  if text {
    app.command("text")?;
  }
  let link = app.doc.links(PageId(0))?.first().cloned().expect("a link");
  let position = app.doc.text(PageId(0))?.nearest(
    PageId(0),
    f32::midpoint(link.first().x0, link.first().x1),
    f32::midpoint(link.first().y0, link.first().y1),
  );
  app.go_to(position, true)?;
  app.preview_link()?;
  assert!(app.peek.is_some());
  Ok(session)
}

/// Draws a frame; its preview's cells.
fn drawn(session: &mut Session) -> Result<vpdf::split::Rect> {
  use vpdf::trace::Drawable;
  session.draw_frame(&mut Vec::new())?;
  session.app().preview_cells().ok_or_else(|| anyhow::anyhow!("a preview"))
}

/// The pixel at the centre of a cell.
fn pixel((column, row): (u16, u16)) -> (u16, u16) {
  (column * 8 + 4, row * 16 + 8)
}

fn settle(app: &mut App) {
  if let Some(animation) = app.animation.take() {
    app.reader.scroll = animation.target;
  }
}

/// Where the page is scrolled, in PDF or text presentation.
fn page(app: &mut App) -> Result<(f32, vpdf::text::Position)> {
  let scroll = app.animation.as_ref().map_or(app.reader.scroll, |a| a.target);
  Ok((scroll, app.text_position(0, 0)?))
}

#[test]
fn preview_takes_the_wheel_under_the_pointer() -> Result<()> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("linked.pdf");
  // The link stays shown while scrolling (U17).
  pages_pdf(&path, &[(10, 60), (10, 60), (18, 3)], Some(30))?;
  let start = Instant::now();
  let at = |ms| start + Duration::from_millis(ms);
  for text in [false, true] {
    let mut session = linked(&path, text)?;
    // (page, preview) scrolled, as drawn.
    let view = |session: &mut Session| -> Result<_> {
      drawn(session)?;
      let app = session.app();
      Ok((page(app)?, app.preview_top()))
    };
    let frame = drawn(&mut session)?;
    let (page0, preview0) = view(&mut session)?;
    let inside = pixel((frame.column + 2, frame.row + 2));
    assert!(wheel_at(&mut session, DOWN, inside, at(0))?);
    let (page1, preview1) = view(&mut session)?;
    assert_eq!(page1, page0, "text={text}: the page stays");
    assert_ne!(preview1, preview0, "text={text}: the preview scrolls");
    // The page scrolls under a pointer resting above the preview, which
    // slides under it; the gesture keeps scrolling the page.
    let above = pixel((frame.column + 2, frame.row - 1));
    assert!(wheel_at(&mut session, DOWN, above, at(1000))?);
    settle(session.app());
    let moved = drawn(&mut session)?;
    let cell = (above.0 / 8, above.1 / 16);
    assert!(moved.contains(cell.0, cell.1), "text={text}: {moved:?}");
    let (page2, preview2) = view(&mut session)?;
    assert_eq!(preview2, preview1, "text={text}");
    assert!(wheel_at(&mut session, DOWN, above, at(1100))?);
    let (page3, preview3) = view(&mut session)?;
    assert_ne!(page3, page2, "text={text}: the page scrolls on");
    assert_eq!(preview3, preview1, "text={text}");
    settle(session.app());
    // A pause ends the gesture; the preview under the pointer takes over.
    let (page4, _) = view(&mut session)?;
    assert!(wheel_at(&mut session, DOWN, above, at(2000))?);
    let (page5, preview5) = view(&mut session)?;
    assert_eq!(page5, page4, "text={text}");
    assert_ne!(preview5, preview1, "text={text}");
    // A zoom over the preview keeps the page for its gesture.
    if !text {
      assert!(wheel_at(&mut session, UP | CTRL, above, at(3000))?);
      let (page6, preview6) = view(&mut session)?;
      wheel_at(&mut session, DOWN, above, at(3100))?;
      let (page7, preview7) = view(&mut session)?;
      assert_ne!(page7, page6, "a zoom gesture keeps the page");
      assert_eq!(preview7, preview6);
    }
  }
  Ok(())
}
