use crate::common;
use crate::focus::{action, screen};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::fmt::Write as _;
use std::path::Path;
use vpdf::Result;
use vpdf::app::{Request, TabMove};
use vpdf::decode::{Button, Mouse, MouseKind, Spot};
use vpdf::figures::Source;
use vpdf::keys::Action;
use vpdf::pointer::{Hit, Region};
use vpdf::prompt::{Prompt, PromptKind};
use vpdf::reader::SelectionKind;
use vpdf::session::Session;
use vpdf::text::{Bounds, PageId, Position};
use vpdf::trace::Drawable;

/// Page 1 links: page 2, web, itself.
fn link_pdf(path: &Path) -> Result<()> {
  let content = |lines: &[&str]| {
    let mut content = String::from("BT /F1 18 Tf 24 TL 20 170 Td");
    for line in lines {
      write!(content, " ({line}) Tj T*").expect("string write");
    }
    content.push_str(" ET");
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len())
  };
  let page = |contents, annots| {
    format!(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
       /Resources << /Font << /F1 4 0 R >> >> /Contents {contents} 0 R \
       {annots} >>"
    )
  };
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R /Outlines 8 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R 6 0 R] /Count 2 >>".to_owned(),
    page(5, "/Annots [10 0 R 11 0 R 12 0 R]"),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    content(&[
      "See the results",
      "Visit the site",
      "Plain words here",
      "Back to the top",
    ]),
    page(7, ""),
    content(&["Results heading", "Second page text"]),
    "<< /Type /Outlines /First 9 0 R /Last 9 0 R /Count 1 >>".to_owned(),
    "<< /Title (Results) /Parent 8 0 R /Dest [6 0 R /XYZ 0 200 0] >>"
      .to_owned(),
    "<< /Type /Annot /Subtype /Link /Rect [15 162 285 192] \
     /Dest [6 0 R /XYZ 0 190 0] >>"
      .to_owned(),
    "<< /Type /Annot /Subtype /Link /Rect [15 138 285 160] \
     /A << /S /URI /URI (https://example.com/) >> >>"
      .to_owned(),
    "<< /Type /Annot /Subtype /Link /Rect [15 92 285 112] \
     /Dest [3 0 R /XYZ 0 150 0] >>"
      .to_owned(),
  ];
  common::write_pdf(path, &objects, "")
}

/// A session on a PDF `write` makes.
fn open(
  write: impl FnOnce(&Path) -> Result<()>,
  text_mode: bool,
) -> Result<(tempfile::TempDir, Session)> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("test.pdf");
  write(&path)?;
  let mut session = Session::new(None);
  session.open(&path, true, screen())?;
  let app = session.app();
  common::outline_loaded(app)?;
  if text_mode {
    app.open_panel(vpdf::keys::PanelKind::Text)?;
    app.close_panel()?;
  }
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text_mode, text_mode);
  Ok((directory, session))
}

fn session(text_mode: bool) -> Result<(tempfile::TempDir, Session)> {
  open(link_pdf, text_mode)
}

/// Screen cell of pane `index` hitting `want`.
fn cell_in(
  session: &mut Session,
  index: usize,
  want: Hit,
) -> Result<(u16, u16)> {
  let app = &mut session.tab().panes[index];
  let (columns, rows) = (app.screen.columns, app.screen.rows - 1);
  for row in 0..rows {
    for column in 0..columns {
      if app.hovered(Region::Page, column, row, None)? == want {
        return Ok((app.at.column + column, app.at.row + row));
      }
    }
  }
  anyhow::bail!("no cell hits {want:?}")
}

/// Focused pane cell hitting `want`.
fn cell(session: &mut Session, want: Hit) -> Result<(u16, u16)> {
  let index = session.tab().focus;
  cell_in(session, index, want)
}

fn mouse(
  session: &mut Session,
  kind: MouseEventKind,
  (column, row): (u16, u16),
) -> Result<bool> {
  let modifiers = KeyModifiers::NONE;
  session.mouse(MouseEvent { kind, column, row, modifiers })
}

fn click(session: &mut Session, at: (u16, u16)) -> Result<()> {
  mouse(session, MouseEventKind::Down(MouseButton::Left), at)?;
  mouse(session, MouseEventKind::Up(MouseButton::Left), at)?;
  Ok(())
}

/// A side button press at a cell.
fn side(button: Button, (column, row): (u16, u16)) -> Mouse {
  Mouse {
    kind: MouseKind::Press(button),
    at: Spot::Cell { column, row },
    modifiers: KeyModifiers::NONE,
  }
}

/// An Alt mouse event at a cell.
fn alt(kind: MouseEventKind, (column, row): (u16, u16)) -> MouseEvent {
  MouseEvent { kind, column, row, modifiers: KeyModifiers::ALT }
}

fn alt_click(session: &mut Session, at: (u16, u16)) -> Result<()> {
  session.mouse(alt(MouseEventKind::Down(MouseButton::Left), at))?;
  session.mouse(alt(MouseEventKind::Up(MouseButton::Left), at))?;
  Ok(())
}

/// The cell over a page rectangle's centre.
fn centre_cell(
  app: &vpdf::app::App,
  page: PageId,
  bounds: Bounds,
) -> (u16, u16) {
  let rect = app.layout.project(page, bounds, app.reader.scroll);
  let (x, y) = rect.centre();
  let (width, height) =
    (f32::from(app.screen.cell_width), f32::from(app.screen.cell_height));
  (app.left + (x / width) as u16, (y / height) as u16)
}

fn status(session: &mut Session) -> Result<String> {
  let app = session.app();
  let mut out = Vec::new();
  app.status(&mut out, app.screen.rows - 1)?;
  Ok(String::from_utf8(out)?)
}

#[test]
fn hovered_links_show_their_destination() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let web = cell(&mut session, Hit::Link(PageId(0), 1))?;
    let plain = cell(&mut session, Hit::Text)?;
    // cell -> status label
    let cases = [
      (internal, Some("→ p. 2 · Results")),
      (web, Some("→ https://example.com/")),
      (plain, None),
    ];
    for (at, want) in cases {
      assert!(mouse(&mut session, MouseEventKind::Moved, at)?, "{at:?}");
      let app = session.app();
      let label = app.hover_link.as_ref().map(|link| link.label.as_str());
      assert_eq!(label, want, "text={text_mode} {at:?}");
      let line = status(&mut session)?;
      if let Some(want) = want {
        assert!(line.contains(want), "{line:?}");
      }
    }
    // Errors and messages outrank the label.
    mouse(&mut session, MouseEventKind::Moved, internal)?;
    session.app().reader.message = "Yanked 4 characters".into();
    let line = status(&mut session)?;
    assert!(line.contains("Yanked") && !line.contains("→"), "{line:?}");
    session.app().reader.message.clear();
    session.app().error = Some("Broken".into());
    let line = status(&mut session)?;
    assert!(line.contains("Broken") && !line.contains("→"), "{line:?}");
    session.app().error = None;
    assert!(status(&mut session)?.contains("→ p. 2"));
    // Reloads drop it; the next frame restores.
    session.app().reload_with("")?;
    assert!(session.app().hover_link.is_none());
    session.draw_frame(&mut Vec::new())?;
    assert!(session.app().hover_link.is_some(), "text={text_mode}");
    assert!(session.pointer_left());
    assert!(session.app().hover_link.is_none(), "leaving clears the tint");
  }
  Ok(())
}

#[test]
fn hovered_links_draw_a_tint() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let mut plain = Vec::new();
    session.app().draw(&mut plain)?;
    mouse(&mut session, MouseEventKind::Moved, internal)?;
    let mut tinted = Vec::new();
    session.app().draw(&mut tinted)?;
    let (plain, tinted) =
      (String::from_utf8_lossy(&plain), String::from_utf8_lossy(&tinted));
    if text_mode {
      // Underlined cyan.
      let style = "\x1b[0;4;36m";
      assert!(!plain.contains(style) && tinted.contains(style));
    } else {
      let placements = |frame: &str| frame.matches("a=p,").count();
      assert!(placements(&tinted) > placements(&plain), "a tint placed");
    }
  }
  Ok(())
}

#[test]
fn clicks_follow_links_and_drags_select() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    // Drags from a link select.
    let beyond = (internal.0 + 6, internal.1);
    mouse(&mut session, MouseEventKind::Down(MouseButton::Left), internal)?;
    mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), beyond)?;
    mouse(&mut session, MouseEventKind::Up(MouseButton::Left), beyond)?;
    let app = session.app();
    assert_eq!(app.reader.cursor.page, PageId(0), "text={text_mode}");
    assert!(app.reader.selection.is_some(), "text={text_mode}");
    app.reader.cancel();
    click(&mut session, internal)?;
    let app = session.app();
    assert_eq!(app.reader.cursor.page, PageId(1), "text={text_mode}");
    assert_eq!(app.reader.jumps().count(), 1, "the jump list grew");
    // Double and triple clicks follow once.
    for link in [0, 2] {
      action(session.app(), Action::History(false))?;
      session.draw_frame(&mut Vec::new())?;
      let at = cell(&mut session, Hit::Link(PageId(0), link))?;
      click(&mut session, at)?;
      let landing = session.app().reader.cursor;
      let jumps = session.app().reader.jumps().count();
      for _ in 0..2 {
        click(&mut session, at)?;
      }
      let app = session.app();
      assert_eq!(app.reader.cursor, landing, "text={text_mode} {at:?}");
      assert_eq!(app.reader.jumps().count(), jumps, "{at:?}");
      assert!(app.reader.selection.is_none(), "text={text_mode} {at:?}");
    }
    // Presses that close a prompt follow nothing.
    let app = session.app();
    app.prompt = Some(Prompt::new(PromptKind::Command));
    let before = app.reader.cursor;
    click(&mut session, internal)?;
    assert_eq!(session.app().reader.cursor, before, "text={text_mode}");
  }
  Ok(())
}

/// Text clicks follow drawn glyphs only.
#[test]
fn text_clicks_need_a_drawn_link() -> Result<()> {
  let (_directory, mut session) = session(true)?;
  let (column, row) = cell(&mut session, Hit::Link(PageId(0), 0))?;
  let blank = (session.app().screen.columns - 2, row);
  assert_eq!(
    session.app().hovered(Region::Page, blank.0, row, None)?,
    Hit::Nothing
  );
  click(&mut session, blank)?;
  assert_eq!(session.app().reader.cursor.page, PageId(0));
  click(&mut session, (column, row))?;
  assert_eq!(session.app().reader.cursor.page, PageId(1));
  Ok(())
}

#[test]
fn back_and_forward_buttons_walk_history() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  click(&mut session, internal)?;
  let report = |button| side(button, (40, 5));
  // (button, page after)
  let cases = [
    (Button::BACK, 0),
    (Button::FORWARD, 1),
    (Button::BACK, 0),
    (Button::BACK, 0),
  ];
  for (step, (button, page)) in cases.into_iter().enumerate() {
    assert!(session.mouse_report(report(button))?, "step {step}");
    assert_eq!(session.app().reader.cursor.page, PageId(page), "step {step}");
  }
  let release =
    Mouse { kind: MouseKind::Release(Button::BACK), ..report(Button::BACK) };
  assert!(!session.mouse_report(release)?, "releases do nothing");
  Ok(())
}

/// Hover and side buttons follow the pointer.
#[test]
fn split_panes_hover_and_focus_apart() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  session.app().command("vsplit")?;
  session.settle()?;
  let focused = session.app().id;
  let tab = session.tab();
  let index = (tab.focus + 1) % tab.panes.len();
  let id = tab.panes[index].id;
  let link = cell_in(&mut session, index, Hit::Link(PageId(0), 0))?;
  mouse(&mut session, MouseEventKind::Moved, link)?;
  for app in &session.tab().panes {
    assert_eq!(app.hover_link.is_some(), app.id == id, "pane {}", app.id);
  }
  assert_eq!(session.app().id, focused, "hover keeps focus");
  click(&mut session, link)?;
  assert_eq!(session.app().id, id, "a press focuses");
  assert_eq!(session.app().reader.cursor.page, PageId(1));
  // A prompt eats one back press.
  let back = side(Button::BACK, link);
  session.app().prompt = Some(Prompt::new(PromptKind::Command));
  assert!(session.mouse_report(back)?);
  assert!(session.app().prompt.is_none());
  assert_eq!(session.app().reader.cursor.page, PageId(1));
  assert!(session.mouse_report(back)?);
  assert_eq!(session.app().reader.cursor.page, PageId(0));
  // A held press keeps its capture.
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), link)?;
  assert!(!session.mouse_report(back)?, "one button at a time");
  mouse(&mut session, MouseEventKind::Up(MouseButton::Left), link)?;
  // Tab switches retest the stale hover.
  mouse(&mut session, MouseEventKind::Moved, link)?;
  let copy = session.app().path.with_file_name("copy.pdf");
  std::fs::copy(&session.app().path, &copy)?;
  session.open(&copy, true, screen())?;
  mouse(&mut session, MouseEventKind::Moved, (0, 0))?;
  session.app().request = Some(Request::Tab(TabMove::Previous(1)));
  session.settle()?;
  assert!(session.tab().panes.iter().any(|app| app.hover_link.is_some()));
  session.draw_frame(&mut Vec::new())?;
  assert!(session.tab().panes.iter().all(|app| app.hover_link.is_none()));
  Ok(())
}

fn words(text_mode: bool) -> Result<(tempfile::TempDir, Session)> {
  let lines = ["alpha beta gamma", "delta epsilon"];
  open(|path| common::text_pdf(path, &lines), text_mode)
}

/// Cell whose press lands on `line`, `glyph`.
fn glyph_cell(
  session: &mut Session,
  line: usize,
  glyph: usize,
) -> Result<(u16, u16)> {
  let app = session.app();
  let page = PageId(0);
  let want = Position { page, line, glyph };
  let (columns, rows) = (app.screen.columns, app.screen.rows - 1);
  for (column, row) in (0..rows).flat_map(|r| (0..columns).map(move |c| (c, r)))
  {
    let got = if app.text_mode {
      Some(app.text_position(column, row)?)
    } else {
      app
        .page_point(column, row, None)
        .map(|(page, x, y)| {
          app.doc.text(page).map(|text| text.nearest(page, x, y))
        })
        .transpose()?
    };
    if got == Some(want)
      && app.hovered(Region::Page, column, row, None)? == Hit::Text
    {
      return Ok((column, row));
    }
  }
  anyhow::bail!("no cell presses {want:?}")
}

fn at(line: usize, glyph: usize) -> Position {
  Position { page: PageId(0), line, glyph }
}

/// Selection ends and kind, if any.
fn selected(
  session: &mut Session,
) -> Option<(Position, Position, SelectionKind)> {
  let reader = &session.app().reader;
  reader.selection.map(|s| (s.anchor, reader.cursor, s.kind))
}

#[test]
fn click_runs_select_words_lines_and_paragraphs() -> Result<()> {
  use SelectionKind::{Character, Line};
  for text_mode in [false, true] {
    let (_directory, mut session) = words(text_mode)?;
    let beta = glyph_cell(&mut session, 0, 7)?;
    // clicks -> selection
    let cases = [
      (1, None),
      (2, Some((at(0, 6), at(0, 9), Character))),
      (3, Some((at(0, 7), at(0, 7), Line))),
    ];
    // Another cell ends each run.
    let delta = glyph_cell(&mut session, 1, 2)?;
    for (clicks, want) in cases {
      click(&mut session, delta)?;
      for _ in 0..clicks {
        click(&mut session, beta)?;
      }
      assert_eq!(selected(&mut session), want, "text={text_mode} {clicks}");
    }
    click(&mut session, delta)?;
    for _ in 0..4 {
      click(&mut session, beta)?;
    }
    let want = Some((at(0, 0), at(1, 12), Line));
    assert_eq!(selected(&mut session), want, "text={text_mode}");
  }
  Ok(())
}

#[test]
fn run_drags_extend_by_word_and_line() -> Result<()> {
  use MouseEventKind::{Down, Drag, Up};
  use SelectionKind::{Character, Line};
  for text_mode in [false, true] {
    let (_directory, mut session) = words(text_mode)?;
    let beta = glyph_cell(&mut session, 0, 7)?;
    let gamma = glyph_cell(&mut session, 0, 12)?;
    let alpha = glyph_cell(&mut session, 0, 2)?;
    let delta = glyph_cell(&mut session, 1, 2)?;
    // (clicks, drag to) -> selection
    let cases = [
      (2, gamma, (at(0, 6), at(0, 15), Character)),
      (2, alpha, (at(0, 9), at(0, 0), Character)),
      (3, delta, (at(0, 7), at(1, 2), Line)),
      (4, gamma, (at(0, 0), at(1, 12), Line)),
    ];
    for (clicks, to, want) in cases {
      click(&mut session, delta)?;
      for _ in 1..clicks {
        click(&mut session, beta)?;
      }
      mouse(&mut session, Down(MouseButton::Left), beta)?;
      mouse(&mut session, Drag(MouseButton::Left), to)?;
      mouse(&mut session, Up(MouseButton::Left), to)?;
      assert_eq!(selected(&mut session), Some(want), "text={text_mode}");
    }
  }
  Ok(())
}

/// Wheel scrolls keep both selection ends.
#[test]
fn wheel_keeps_the_selection() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let text = cell(&mut session, Hit::Text)?;
  let beyond = (text.0 + 6, text.1);
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), text)?;
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), beyond)?;
  mouse(&mut session, MouseEventKind::Up(MouseButton::Left), beyond)?;
  let before = (selected(&mut session), session.app().reader.scroll);
  mouse(&mut session, MouseEventKind::ScrollDown, text)?;
  let started = std::time::Instant::now();
  while session.app().animation.is_some() {
    session.app().animate();
    anyhow::ensure!(started.elapsed().as_secs() < 2, "scroll animation");
    std::thread::sleep(std::time::Duration::from_millis(2));
  }
  assert!(session.app().reader.scroll > before.1, "the view moved");
  assert_eq!(selected(&mut session), before.0);
  Ok(())
}

/// An off-screen Cursor glyph does not anchor.
#[test]
fn alt_clicks_skip_unseen_cursors() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let text = cell(&mut session, Hit::Text)?;
  action(session.app(), Action::Cursor)?;
  let start = session.app().reader.cursor;
  session.app().reader.cursor = Position { page: PageId(1), line: 1, glyph: 0 };
  alt_click(&mut session, text)?;
  let anchor = selected(&mut session).map(|(anchor, ..)| anchor);
  assert_eq!(anchor, Some(start));
  Ok(())
}

/// Drags enter Visual; Esc returns.
#[test]
fn drags_return_to_their_entry_mode() -> Result<()> {
  use crossterm::event::{KeyCode, KeyEvent};
  use vpdf::reader::Mode;
  let (_directory, mut session) = words(false)?;
  let alpha = glyph_cell(&mut session, 0, 0)?;
  let gamma = glyph_cell(&mut session, 0, 12)?;
  let key = |session: &mut Session, code| {
    let event = KeyEvent::new(code, KeyModifiers::NONE);
    session.app().key(event, &mut vpdf::prompt::History::default())
  };
  // (enter Cursor first) -> mode after Esc
  for (cursor, entry) in [(false, Mode::Normal), (true, Mode::Cursor)] {
    if cursor {
      key(&mut session, KeyCode::Char('i'))?;
    }
    click(&mut session, gamma)?;
    assert_eq!(session.app().reader.mode(), entry, "a click keeps it");
    mouse(&mut session, MouseEventKind::Down(MouseButton::Left), alpha)?;
    mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), gamma)?;
    mouse(&mut session, MouseEventKind::Up(MouseButton::Left), gamma)?;
    let visual = Mode::Visual(SelectionKind::Character);
    assert_eq!(session.app().reader.mode(), visual);
    key(&mut session, KeyCode::Esc)?;
    assert_eq!(session.app().reader.mode(), entry);
  }
  Ok(())
}

#[test]
fn alt_clicks_extend_the_selection() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = words(text_mode)?;
    let gamma = glyph_cell(&mut session, 0, 12)?;
    let delta = glyph_cell(&mut session, 1, 2)?;
    // (cell) -> selection, from first visible text
    for (to, want) in [(gamma, at(0, 12)), (delta, at(1, 2))] {
      alt_click(&mut session, to)?;
      let want = Some((at(0, 0), want, SelectionKind::Character));
      assert_eq!(selected(&mut session), want, "text={text_mode}");
    }
    // A target or Cursor glyph anchors.
    session.app().reader.cancel();
    let beta = glyph_cell(&mut session, 0, 7)?;
    click(&mut session, beta)?;
    alt_click(&mut session, delta)?;
    let want = Some((at(0, 7), at(1, 2), SelectionKind::Character));
    assert_eq!(selected(&mut session), want, "text={text_mode}");
    session.app().reader.cancel();
    action(session.app(), Action::Cursor)?;
    session.app().reader.cursor = at(1, 0);
    alt_click(&mut session, gamma)?;
    let want = Some((at(1, 0), at(0, 12), SelectionKind::Character));
    assert_eq!(selected(&mut session), want, "text={text_mode}");
  }
  Ok(())
}

#[test]
fn drags_past_edges_scroll_faster_with_distance() -> Result<()> {
  use vpdf::decode::Parser;
  let (_directory, mut session) = session(false)?;
  let text = cell(&mut session, Hit::Text)?;
  let rows = session.app().screen.rows;
  let x = text.0 * 8 + 4;
  let report = |bytes: String| {
    let mut events = std::collections::VecDeque::new();
    let mut parser = Parser::default();
    parser.set_pixels(true);
    parser.feed(bytes.as_bytes(), &mut events);
    match events.pop_front() {
      Some(vpdf::decode::Event::Mouse(mouse)) => mouse,
      other => panic!("a mouse report: {other:?}"),
    }
  };
  let last = i32::from(rows - 1) * 16;
  // (scroll, drag pixel y) -> rows scrolled
  let cases = [
    (0.0, last - 8, 0.0),
    (0.0, last + 8, 1.0),
    (0.0, last + 64, 5.0),
    (64.0, 0, -1.0),
    (64.0, -40, -1.0),
  ];
  for (start, y, want) in cases {
    session.app().reader.scroll = start;
    let press = format!("\x1b[<0;{x};{}M", text.1 * 16 + 8);
    session.mouse_report(report(press))?;
    let before = session.app().reader.scroll;
    session.mouse_report(report(format!("\x1b[<32;{x};{y}M")))?;
    let scrolled = (session.app().reader.scroll - before) / 16.0;
    assert!((scrolled - want).abs() < 0.01, "{y}: {scrolled}");
    assert!(session.app().reader.selection.is_some(), "the drag selects");
    session.mouse_report(report(format!("\x1b[<0;{x};{y}m")))?;
  }
  // Cells: the status row scrolls one row.
  session.app().reader.scroll = 0.0;
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), text)?;
  let status = (text.0, rows - 1);
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), status)?;
  assert!((session.app().reader.scroll - 16.0).abs() < 0.01);
  mouse(&mut session, MouseEventKind::Up(MouseButton::Left), status)?;
  Ok(())
}

/// Text drags land on the scrolled view.
#[test]
fn text_edge_drags_track_the_pointer() -> Result<()> {
  let (_directory, mut session) = session(true)?;
  let text = cell(&mut session, Hit::Text)?;
  let edge = (text.0, session.app().screen.rows - 2);
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), text)?;
  for _ in 0..3 {
    mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), edge)?;
  }
  let app = session.app();
  app.draw(&mut Vec::new())?;
  let under = app.text_position(edge.0, edge.1)?;
  assert_eq!(app.reader.cursor.line, under.line);
  Ok(())
}

fn right(session: &mut Session, at: (u16, u16)) -> Result<()> {
  mouse(session, MouseEventKind::Down(MouseButton::Right), at)?;
  mouse(session, MouseEventKind::Up(MouseButton::Right), at)?;
  Ok(())
}

fn labels(session: &mut Session) -> Vec<&'static str> {
  let menu = session.app().context_menu.as_ref();
  menu.map_or_default(|menu| menu.items.iter().map(|item| item.label).collect())
}

fn press(session: &mut Session, code: crossterm::event::KeyCode) -> Result<()> {
  let key = crossterm::event::KeyEvent::new(code, KeyModifiers::NONE);
  session.app().key(key, &mut vpdf::prompt::History::default())
}

const ANYWHERE: [&str; 6] =
  ["Back", "Forward", "Fit width", "Fit page", "Outline", "Overview"];

#[test]
fn right_clicks_open_menus_by_context() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let web = cell(&mut session, Hit::Link(PageId(0), 1))?;
    let plain = cell(&mut session, Hit::Text)?;
    let link = ["Follow", "Preview"];
    let address = ["Follow", "Copy address"];
    // (cell) -> items before the common ones
    let cases: [(_, &[&str]); 3] = [
      (internal, &link),
      (web, &address),
      (plain, &["Select word", "Select line", "Search word"]),
    ];
    let text = ["Select word", "Select line", "Search word"];
    for (at, want) in cases {
      right(&mut session, at)?;
      let got = labels(&mut session);
      // Text items may follow link items.
      let middle = &got[want.len()..got.len() - ANYWHERE.len()];
      assert!(got.starts_with(want), "text={text_mode} {got:?}");
      assert!(middle.is_empty() || middle == text, "{got:?}");
      assert!(got.ends_with(&ANYWHERE), "{got:?}");
      assert!(status(&mut session)?.contains("MENU"));
      // A press outside only closes it.
      let cursor = session.app().reader.cursor;
      let screen = session.app().screen;
      click(&mut session, (screen.columns - 1, screen.rows - 2))?;
      assert!(session.app().context_menu.is_none());
      assert_eq!(session.app().reader.cursor, cursor, "text={text_mode}");
    }
    // A selection offers copying and search.
    click(&mut session, plain)?;
    for _ in 0..2 {
      click(&mut session, plain)?;
    }
    right(&mut session, plain)?;
    let want = ["Copy", "Copy as image", "Search for selection"];
    let want: Vec<_> = want.iter().chain(&ANYWHERE).copied().collect();
    assert_eq!(labels(&mut session), want, "text={text_mode}");
    // Closing keeps the selection.
    let screen = session.app().screen;
    click(&mut session, (screen.columns - 1, screen.rows - 2))?;
    assert!(session.app().reader.selection.is_some(), "text={text_mode}");
    right(&mut session, plain)?;
    press(&mut session, crossterm::event::KeyCode::Char('j'))?;
    press(&mut session, crossterm::event::KeyCode::Char('j'))?;
    press(&mut session, crossterm::event::KeyCode::Enter)?;
    let app = session.app();
    assert!(app.context_menu.is_none() && app.reader.selection.is_none());
    assert_eq!(app.search.query.text, "Plain words here", "text={text_mode}");
  }
  Ok(())
}

#[test]
fn menu_items_run_by_key_and_click() -> Result<()> {
  use crossterm::event::KeyCode;
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  let web = cell(&mut session, Hit::Link(PageId(0), 1))?;
  let plain = cell(&mut session, Hit::Text)?;
  // Enter runs the first item: follow.
  right(&mut session, internal)?;
  press(&mut session, KeyCode::Enter)?;
  assert_eq!(session.app().reader.cursor.page, PageId(1));
  // A click on "Back" goes back.
  right(&mut session, plain)?;
  let at =
    session.app().context_menu.as_ref().map(|menu| menu.at).expect("a menu");
  let back = labels(&mut session).iter().position(|label| *label == "Back");
  let row = at.1 + u16::try_from(back.expect("Back"))?;
  click(&mut session, (at.0 + 1, row))?;
  assert!(session.app().context_menu.is_none());
  assert_eq!(session.app().reader.cursor.page, PageId(0));
  // Copy address fills the register.
  right(&mut session, web)?;
  press(&mut session, KeyCode::Char('j'))?;
  press(&mut session, KeyCode::Enter)?;
  let register =
    session.app().reader.registers.get(&'"').map(ToString::to_string);
  assert_eq!(register.as_deref(), Some("https://example.com/"));
  // Select line enters line Visual.
  right(&mut session, plain)?;
  press(&mut session, KeyCode::Char('j'))?;
  press(&mut session, KeyCode::Enter)?;
  let visual = vpdf::reader::Mode::Visual(SelectionKind::Line);
  assert_eq!(session.app().reader.mode(), visual);
  // Esc closes; `:` closes and prompts.
  right(&mut session, plain)?;
  press(&mut session, KeyCode::Esc)?;
  assert!(session.app().context_menu.is_none());
  assert_eq!(session.app().reader.mode(), visual, "Esc only closes");
  right(&mut session, plain)?;
  press(&mut session, KeyCode::Char(':'))?;
  let app = session.app();
  assert!(app.context_menu.is_none() && app.prompt.is_some());
  press(&mut session, KeyCode::Esc)?;
  // Preview shows the pressed link.
  session.app().reader.cancel();
  right(&mut session, internal)?;
  press(&mut session, KeyCode::Char('j'))?;
  press(&mut session, KeyCode::Enter)?;
  let target = session.app().peek.as_ref().map(|peek| peek.target.clone());
  let page = match target {
    Some(vpdf::document::Target::Point { page, .. }) => Some(page),
    _ => None,
  };
  assert_eq!(page, Some(PageId(1)), "a preview of page 2");
  // Search word searches the pressed word.
  session.app().peek = None;
  right(&mut session, plain)?;
  let at =
    labels(&mut session).iter().position(|label| *label == "Search word");
  for _ in 0..at.expect("Search word") {
    press(&mut session, KeyCode::Char('j'))?;
  }
  press(&mut session, KeyCode::Enter)?;
  assert_ne!(session.app().search.query.text, "");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn figures_offer_copy_and_save() -> Result<()> {
  let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen())?;
  let app = session.app();
  app.move_figure(true, 1)?;
  let snapshot = app.region.as_ref().expect("a figure").snapshot().clone();
  app.region = None;
  let cell = centre_cell(app, snapshot.page, snapshot.bounds);
  right(&mut session, cell)?;
  let labels = labels(&mut session);
  assert!(labels.contains(&"Copy image") && labels.contains(&"Save image"));
  Ok(())
}

/// Menus stay inside short, narrow panes.
#[test]
fn menus_fit_small_panes() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  for command in ["split", "split", "vsplit", "vsplit", "vsplit"] {
    session.app().command(command)?;
    session.settle()?;
  }
  let app = session.app();
  let (at, screen) = (app.at, app.screen);
  let corner = (at.column + screen.columns - 1, at.row + screen.rows - 2);
  right(&mut session, corner)?;
  let app = session.app();
  let menu = app.context_menu.as_ref().expect("a menu");
  let rows = u16::try_from(menu.items.len())?;
  assert!(menu.at.1 + rows < app.screen.rows, "{:?} {rows}", menu.at);
  assert!(menu.at.0 + menu.width <= app.screen.columns, "{:?}", menu.at);
  assert!(menu.at.0 >= app.left);
  Ok(())
}

/// Search joins hyphen breaks like its index.
#[test]
fn selection_search_finds_hyphenated_words() -> Result<()> {
  use crossterm::event::KeyCode;
  let lines = ["an exam-", "ple here"];
  let (_directory, mut session) =
    open(|path| common::text_pdf(path, &lines), false)?;
  let from = glyph_cell(&mut session, 0, 3)?;
  let to = glyph_cell(&mut session, 1, 2)?;
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), from)?;
  mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), to)?;
  mouse(&mut session, MouseEventKind::Up(MouseButton::Left), to)?;
  right(&mut session, to)?;
  let at =
    labels(&mut session).iter().position(|l| *l == "Search for selection");
  for _ in 0..at.expect("the search item") {
    press(&mut session, KeyCode::Char('j'))?;
  }
  press(&mut session, KeyCode::Enter)?;
  common::search_loaded(session.app())?;
  let app = session.app();
  assert_eq!(app.search.query.text, "example");
  assert_eq!(app.search.hits.len(), 1);
  Ok(())
}

/// Tab closes the menu and moves focus.
#[test]
fn tab_passes_through_menus() -> Result<()> {
  use crossterm::event::KeyCode;
  let (_directory, mut session) = session(false)?;
  action(session.app(), Action::Panel(vpdf::keys::PanelKind::Outline))?;
  press(&mut session, KeyCode::Tab)?;
  assert!(!session.app().panel_focused());
  let plain = cell(&mut session, Hit::Text)?;
  right(&mut session, plain)?;
  press(&mut session, KeyCode::Tab)?;
  let app = session.app();
  assert!(app.context_menu.is_none() && app.panel_focused());
  Ok(())
}

/// Alt-drags sweep; release opens the bar.
#[test]
fn alt_drags_capture_regions() -> Result<()> {
  use crossterm::event::KeyCode;
  let (_directory, mut session) = words(false)?;
  let from = glyph_cell(&mut session, 0, 2)?;
  let to = glyph_cell(&mut session, 1, 9)?;
  let backdrops = |session: &mut Session| -> Result<usize> {
    let mut frame = Vec::new();
    session.app().draw(&mut frame)?;
    Ok(String::from_utf8_lossy(&frame).matches("z=-1").count())
  };
  let before = backdrops(&mut session)?;
  session.mouse(alt(MouseEventKind::Down(MouseButton::Left), from))?;
  assert!(session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), to))?);
  assert!(backdrops(&mut session)? > before, "the page dims");
  // Alt released mid-drag still captures.
  mouse(&mut session, MouseEventKind::Up(MouseButton::Left), to)?;
  let app = session.app();
  let Some(region) = &app.region else { panic!("a capture") };
  assert!(region.is_bar());
  let bounds = region.snapshot().bounds;
  // Corners sit under press and release.
  for (column, row) in [from, to] {
    let (_, x, y) = app.page_point(column, row, None).expect("on the page");
    let corner =
      |a: f32, b: f32, c: f32| (a - b).abs() < 0.01 || (a - c).abs() < 0.01;
    assert!(corner(x, bounds.x0, bounds.x1) && corner(y, bounds.y0, bounds.y1));
  }
  press(&mut session, KeyCode::Esc)?;
  // Cuts make the drag inert.
  let cuts: [Cut; 2] = [
    |session, _| press(session, KeyCode::Esc),
    |session, at| mouse(session, MouseEventKind::Moved, at).map(drop),
  ];
  for (step, cut) in cuts.into_iter().enumerate() {
    session.app().reader.cancel();
    session.mouse(alt(MouseEventKind::Down(MouseButton::Left), from))?;
    session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), to))?;
    cut(&mut session, to)?;
    mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), from)?;
    mouse(&mut session, MouseEventKind::Up(MouseButton::Left), from)?;
    assert_eq!(backdrops(&mut session)?, before, "step {step}: undimmed");
    let app = session.app();
    assert!(app.region.is_none(), "step {step}: no capture");
    assert!(app.reader.selection.is_none(), "step {step}: inert drag");
    // A plain drag after it selects.
    mouse(&mut session, MouseEventKind::Down(MouseButton::Left), from)?;
    mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), to)?;
    mouse(&mut session, MouseEventKind::Up(MouseButton::Left), to)?;
    let app = session.app();
    assert!(app.region.is_none() && app.reader.selection.is_some(), "{step}");
  }
  Ok(())
}

/// Ends a sweep mid-drag.
type Cut = fn(&mut Session, (u16, u16)) -> Result<()>;

/// A filled box and a lone caption.
fn figure_pdf(path: &Path) -> Result<()> {
  let content = "0.2 0.4 0.8 rg 60 20 180 50 re f \
    BT /F1 12 Tf 20 180 Td (Table 1 alone here) Tj ET";
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
     /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
      .to_owned(),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
  ];
  common::write_pdf(path, &objects, "")
}

/// Alt-click picks graphics; captions extend.
#[test]
fn alt_clicks_pick_graphics() -> Result<()> {
  let (_directory, mut session) = open(figure_pdf, false)?;
  let app = session.app();
  let figures = app.doc.figures(PageId(0))?;
  let graphic = figures.iter().find(|f| f.source == Source::Graphic);
  let graphic = graphic.expect("a box");
  assert!(figures.iter().any(|f| f.source == Source::Caption));
  let inside = centre_cell(app, PageId(0), graphic.bounds);
  let want = graphic.bounds;
  let caption = glyph_cell(&mut session, 0, 2)?;
  alt_click(&mut session, inside)?;
  let region = session.app().region.as_ref().expect("the box");
  let got = region.snapshot().bounds;
  assert_eq!(
    (got.x0, got.y0, got.x1, got.y1),
    (want.x0, want.y0, want.x1, want.y1)
  );
  session.app().region = None;
  alt_click(&mut session, caption)?;
  let app = session.app();
  assert!(app.region.is_none(), "captions extend");
  assert!(app.reader.selection.is_some());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn alt_clicks_pick_figures() -> Result<()> {
  let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut session = Session::new(None);
  session.open(&root.join("paper.pdf"), true, screen())?;
  let app = session.app();
  app.move_figure(true, 1)?;
  let figure = app.region.take().expect("a figure").snapshot().clone();
  let cell = centre_cell(app, figure.page, figure.bounds);
  alt_click(&mut session, cell)?;
  let app = session.app();
  let region = app.region.as_ref().expect("the picked figure");
  assert!(region.is_bar());
  let picked = region.snapshot();
  assert_eq!(
    (picked.page, picked.bounds.x0, picked.bounds.y0),
    (figure.page, figure.bounds.x0, figure.bounds.y0)
  );
  Ok(())
}

/// Sweeps stay inside the cropped page.
#[test]
fn sweeps_stay_on_page() -> Result<()> {
  let (_directory, mut session) = words(false)?;
  action(session.app(), Action::Crop)?;
  // Zoomed out, the pane edge is off the page.
  for _ in 0..4 {
    action(session.app(), Action::Zoom(-1))?;
  }
  session.app().draw(&mut Vec::new())?;
  let from = glyph_cell(&mut session, 0, 2)?;
  let edge = (session.app().screen.columns - 1, from.1 + 2);
  let app = session.app();
  let shown = app.layout.crop.apply(app.doc.pages[0].bounds);
  let off = app.page_point(edge.0, edge.1, None).expect("a point");
  assert!(off.1 > shown.x1, "the edge lies past the page");
  session.mouse(alt(MouseEventKind::Down(MouseButton::Left), from))?;
  session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), edge))?;
  session.mouse(alt(MouseEventKind::Up(MouseButton::Left), edge))?;
  let app = session.app();
  let got = app.region.as_ref().expect("a capture").snapshot().bounds;
  assert!(got.x1 <= shown.x1 && got.y1 <= shown.y1, "{got:?} in {shown:?}");
  assert!(got.x1 > got.x0 + 10.0, "a real sweep");
  Ok(())
}
