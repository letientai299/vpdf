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
use vpdf::pointer::{Hit, List, Region};
use vpdf::prompt::{Prompt, PromptKind};
use vpdf::reader::SelectionKind;
use vpdf::session::Session;
use vpdf::text::{Bounds, PageId, Position};
use vpdf::trace::Drawable;

/// Page 1 links: page 2, web, itself.
fn link_pdf(path: &Path) -> Result<()> {
  let page2 = "/Dest [6 0 R /XYZ 0 190 0]";
  let web = "/A << /S /URI /URI (https://example.com/) >>";
  let targets = [page2, web, "/Dest [3 0 R /XYZ 0 150 0]"];
  lines_pdf(path, LINES.into_iter().zip(targets))
}

/// Link rectangles over lines 1, 2, and 4.
pub(super) const LINES: [&str; 3] =
  ["15 162 285 192", "15 138 285 160", "15 92 285 112"];

/// Page 1 links: (rectangle, target).
pub(super) fn lines_pdf<'a>(
  path: &Path,
  links: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<()> {
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
  ];
  let annots = links.map(|(rect, target)| {
    format!("<< /Type /Annot /Subtype /Link /Rect [{rect}] {target} >>")
  });
  let objects: Vec<_> = objects.into_iter().chain(annots).collect();
  common::write_pdf(path, &objects, "")
}

/// A session on a PDF `write` makes.
pub(super) fn open(
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

pub(super) fn session(text_mode: bool) -> Result<(tempfile::TempDir, Session)> {
  open(link_pdf, text_mode)
}

/// Screen cell of pane `index` hitting `want`.
pub(super) fn cell_in(
  session: &mut Session,
  index: usize,
  want: Hit,
) -> Result<(u16, u16)> {
  let region = match want {
    Hit::Row(_) => Region::Panel,
    Hit::Thumbnail(_) => Region::Overview,
    _ => Region::Page,
  };
  let app = &mut session.tab().panes[index];
  let (columns, rows) = (app.screen.columns, app.screen.rows - 1);
  for row in 0..rows {
    for column in 0..columns {
      if app.hovered(region, column, row, None)? == want {
        return Ok((app.at.column + column, app.at.row + row));
      }
    }
  }
  anyhow::bail!("no cell hits {want:?}")
}

/// Focused pane cell hitting `want`.
pub(super) fn cell(session: &mut Session, want: Hit) -> Result<(u16, u16)> {
  let index = session.tab().focus;
  cell_in(session, index, want)
}

pub(super) fn mouse(
  session: &mut Session,
  kind: MouseEventKind,
  (column, row): (u16, u16),
) -> Result<bool> {
  let modifiers = KeyModifiers::NONE;
  session.mouse(MouseEvent { kind, column, row, modifiers })
}

/// A press and release of `button`.
pub(super) fn tap(
  session: &mut Session,
  button: MouseButton,
  at: (u16, u16),
) -> Result<()> {
  mouse(session, MouseEventKind::Down(button), at)?;
  mouse(session, MouseEventKind::Up(button), at)?;
  Ok(())
}

pub(super) fn click(session: &mut Session, at: (u16, u16)) -> Result<()> {
  tap(session, MouseButton::Left, at)
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

/// A frame, whose status line shows the hover detail.
pub(super) fn status(session: &mut Session) -> Result<String> {
  use vpdf::trace::Drawable;
  let mut out = Vec::new();
  session.draw_frame(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
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
      let label = session.app().hover_label();
      assert_eq!(label.as_deref(), want, "text={text_mode} {at:?}");
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
    assert!(session.app().hover_link().is_none());
    assert!(session.app().hover_due().is_none(), "reloads end the rest");
    session.draw_frame(&mut Vec::new())?;
    assert!(session.app().hover_link().is_some(), "text={text_mode}");
    assert!(session.pointer_left());
    assert!(session.app().hover_link().is_none(), "leaving clears the tint");
  }
  Ok(())
}

/// U16: notes yield to the detail once the pointer moves past them.
#[test]
fn hover_details_show_over_passed_messages() -> Result<()> {
  let label = "→ p. 2 · Results";
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let link = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let near = (link.0 + 1, link.1);
    let plain = cell(&mut session, Hit::Text)?;
    // (message set before the step, error, motion to) -> frame, shown
    let steps = [
      (Some("20 matches"), None, link, true, label),
      (Some("Yanked 4 characters"), None, link, true, label),
      (None, None, near, false, label),
      (None, None, plain, true, "Yanked 4 characters"),
      (Some("Copied link"), Some("Broken"), link, true, "Broken"),
    ];
    for (step, (message, error, to, frame, shown)) in steps.iter().enumerate() {
      let app = session.app();
      app.error = error.map(str::to_owned);
      if let Some(message) = message {
        app.reader.message = (*message).to_owned();
        // A note made at rest shows over the detail; errors over both.
        let drawn = status(&mut session)?;
        let note = error.unwrap_or(message);
        assert!(drawn.contains(note), "step {step}: {drawn:?}");
      }
      let framed = mouse(&mut session, MouseEventKind::Moved, *to)?;
      assert_eq!(framed, *frame, "text={text_mode} step {step}");
      let drawn = status(&mut session)?;
      let at = format!("text={text_mode} step {step}: {drawn:?}");
      assert!(drawn.contains(shown), "{at}");
    }
  }
  Ok(())
}

/// Wrapped lines of one address tint together.
#[test]
fn wrapped_links_tint_whole() -> Result<()> {
  let uri = "/A << /S /URI /URI (https://example.com/long) >>";
  // Line 1's segment ends it; line 2's starts the next.
  let rects = ["120 162 285 192", "15 138 150 160", LINES[2]];
  for text_mode in [false, true] {
    let wrapped =
      |path: &Path| lines_pdf(path, rects.into_iter().zip([uri; 3]));
    let (_directory, mut session) = open(wrapped, text_mode)?;
    let mut plain = Vec::new();
    session.app().draw(&mut plain)?;
    // Tints drawn per segment.
    let mut each = std::collections::BTreeSet::new();
    // (hovered link) -> tinted segments
    for (index, want) in [(0, 2), (1, 2), (2, 1)] {
      let at = cell(&mut session, Hit::Link(PageId(0), index))?;
      mouse(&mut session, MouseEventKind::Moved, at)?;
      let segments = session.app().hover_segments().map(|(_, s)| s.len());
      assert_eq!(segments, Some(want), "text={text_mode} link {index}");
      let mut tinted = Vec::new();
      session.app().draw(&mut tinted)?;
      let tinted = String::from_utf8_lossy(&tinted);
      let drawn = if text_mode {
        // One underlined run per line.
        tinted.matches("\x1b[0;4;36m").count()
      } else {
        let placements = |frame: &str| frame.matches("a=p,").count();
        placements(&tinted) - placements(&String::from_utf8_lossy(&plain))
      };
      assert_eq!(drawn % want, 0, "text={text_mode} link {index} drawn");
      each.insert(drawn / want);
    }
    assert_eq!(each.len(), 1, "text={text_mode}: {each:?}");
    assert!(!each.contains(&0), "text={text_mode}: tinted");
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
  /// U18: only a jump closes the grid.
  #[derive(Clone, Copy, PartialEq)]
  enum Origin {
    Page,
    Grid,
    /// The grid over a focused panel.
    Panel,
    /// The grid without history.
    Empty,
  }
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  click(&mut session, internal)?;
  let report = |button| side(button, (40, 5));
  // (button, from, page after)
  let cases = [
    (Button::BACK, Origin::Page, 0),
    (Button::FORWARD, Origin::Page, 1),
    (Button::BACK, Origin::Grid, 0),
    (Button::FORWARD, Origin::Grid, 1),
    (Button::BACK, Origin::Page, 0),
    (Button::BACK, Origin::Page, 0),
    (Button::FORWARD, Origin::Panel, 1),
    (Button::BACK, Origin::Empty, 1),
  ];
  for (step, (button, from, page)) in cases.into_iter().enumerate() {
    let app = session.app();
    if from == Origin::Panel {
      action(app, Action::Panel(vpdf::keys::PanelKind::Outline))?;
    }
    if from == Origin::Empty {
      app.reader.clear_history();
    }
    if from != Origin::Page {
      app.open_overview()?;
    }
    if let Some(panel) = app.panel.as_mut() {
      panel.focused = from == Origin::Panel;
    }
    assert!(session.mouse_report(report(button))?, "step {step}");
    let app = session.app();
    assert_eq!(app.reader.cursor.page, PageId(page), "step {step}");
    assert_eq!(app.overview.is_some(), from == Origin::Empty, "step {step}");
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
    assert_eq!(
      matches!(app.hover, Hit::Link(..)),
      app.id == id,
      "pane {}",
      app.id
    );
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
  assert!(
    session.tab().panes.iter().any(|app| matches!(app.hover, Hit::Link(..)))
  );
  session.draw_frame(&mut Vec::new())?;
  assert!(
    session.tab().panes.iter().all(|app| !matches!(app.hover, Hit::Link(..)))
  );
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

/// U30: a Normal click marks its glyph.
#[test]
fn clicks_mark_their_target() -> Result<()> {
  #[derive(Clone, Copy, Debug)]
  enum Then {
    Nothing,
    Double,
    Drag,
    Cursor,
  }
  // (text view, after the click, marked)
  for (text_mode, then, marked) in [
    (false, Then::Nothing, true),
    (true, Then::Nothing, true),
    (false, Then::Double, false),
    (true, Then::Drag, false),
    (false, Then::Drag, false),
    (true, Then::Cursor, false),
  ] {
    let case = format!("text {text_mode} {then:?}");
    let (_directory, mut session) = words(text_mode)?;
    let cell = glyph_cell(&mut session, 0, 7)?;
    click(&mut session, cell)?;
    match then {
      Then::Nothing => {}
      Then::Double => click(&mut session, cell)?,
      Then::Drag => {
        let end = glyph_cell(&mut session, 0, 12)?;
        mouse(&mut session, MouseEventKind::Down(MouseButton::Left), end)?;
        mouse(&mut session, MouseEventKind::Drag(MouseButton::Left), cell)?;
        mouse(&mut session, MouseEventKind::Up(MouseButton::Left), cell)?;
      }
      Then::Cursor => action(session.app(), Action::Cursor)?,
    }
    // Frames diff placements, so compare steady frames.
    let app = session.app();
    let mut frame = Vec::new();
    for _ in 0..2 {
      frame.clear();
      app.draw(&mut frame)?;
    }
    app.landing = None;
    let mut plain = Vec::new();
    app.draw(&mut plain)?;
    assert_eq!(frame != plain, marked, "{case}");
  }
  Ok(())
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
    // A quick plain click runs anew.
    click(&mut session, delta)?;
    assert_eq!(selected(&mut session), None, "text={text_mode}");
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

pub(super) fn right(session: &mut Session, at: (u16, u16)) -> Result<()> {
  tap(session, MouseButton::Right, at)
}

pub(super) fn labels(session: &mut Session) -> Vec<&'static str> {
  let menu = session.app().context_menu.as_ref();
  menu.map_or_default(|menu| menu.items.iter().map(|item| item.label).collect())
}

pub(super) fn press(
  session: &mut Session,
  code: crossterm::event::KeyCode,
) -> Result<()> {
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
    let link = ["Follow", "Preview", "Open in split"];
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

/// Rows without a target keep focus.
#[test]
fn inert_rows_keep_focus() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  action(session.app(), Action::Panel(vpdf::keys::PanelKind::Help))?;
  for _ in 0..2 {
    click(&mut session, (2, 2))?;
  }
  assert!(session.app().panel_focused());
  Ok(())
}

/// U31: a row double click is Enter.
#[test]
fn row_double_clicks_focus() -> Result<()> {
  use vpdf::document::{Heading, Target};
  // (rows, page target, page, focused, jumps)
  for (rows, page_target, page, focused, jumps) in [
    (&[1][..], false, 1, true, 1),
    (&[1, 1], false, 1, false, 1),
    (&[1, 1], true, 1, false, 1),
    (&[1, 2], false, 0, true, 2),
  ] {
    let case = format!("{rows:?} page {page_target}");
    let (_directory, mut session) = session(false)?;
    let outline = &mut session.app().doc.outline;
    if page_target {
      outline[0].target = Target::Page(PageId(1));
    }
    let top = Target::Point { page: PageId(0), x: 0.0, y: 0.0 };
    outline.push(Heading { title: "Top".into(), depth: 0, target: top });
    action(session.app(), Action::Panel(vpdf::keys::PanelKind::Outline))?;
    for &row in rows {
      click(&mut session, (2, row))?;
    }
    let app = session.app();
    assert_eq!(app.reader.cursor.page, PageId(page), "{case}");
    assert_eq!(app.panel_focused(), focused, "{case}");
    assert_eq!(app.reader.jumps().count(), jumps, "{case}");
  }
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
  let hint = "Release captures; Esc cancels";
  session.mouse(alt(MouseEventKind::Down(MouseButton::Left), from))?;
  // Along one row, the sweep has no area yet.
  let along = (to.0, from.1);
  session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), along))?;
  assert!(!status(&mut session)?.contains(hint), "no area, no hint");
  assert!(session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), to))?);
  assert!(status(&mut session)?.contains(hint), "the drag hints");
  // A note mid-sweep shows until the pointer moves past it.
  session.app().reader.message = "Set tmux allow-passthrough all".into();
  assert!(!status(&mut session)?.contains(hint), "a new note shows");
  // The same spot moves nothing else, so the pass alone draws.
  assert!(session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), to))?);
  assert!(status(&mut session)?.contains(hint), "motion passes the note");
  assert!(backdrops(&mut session)? > before, "the page dims");
  // Alt released mid-drag still captures.
  mouse(&mut session, MouseEventKind::Up(MouseButton::Left), to)?;
  let line = status(&mut session)?;
  assert!(!line.contains(hint) && line.contains("h/l choose"), "{line:?}");
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
    assert!(!status(&mut session)?.contains(hint), "step {step}: hint ends");
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

/// Rests on `link` and draws its preview.
fn hover_open(
  session: &mut Session,
  link: (u16, u16),
) -> Result<vpdf::split::Rect> {
  let screen = session.app().screen;
  mouse(session, MouseEventKind::Moved, (screen.columns - 1, screen.rows - 2))?;
  mouse(session, MouseEventKind::Moved, link)?;
  let later = std::time::Instant::now() + vpdf::links::REST;
  anyhow::ensure!(session.hover_preview(later), "a due preview");
  session.draw_frame(&mut Vec::new())?;
  drawn_preview(session)
}

/// Screen cells of the drawn preview.
fn drawn_preview(session: &mut Session) -> Result<vpdf::split::Rect> {
  let shown = session.tab().panes.iter().find_map(|app| {
    let area = app.peek.as_ref()?.placement?.area;
    Some(vpdf::split::Rect {
      column: area.column + app.at.column,
      row: area.row + app.at.row,
      ..area
    })
  });
  shown.ok_or_else(|| anyhow::anyhow!("a drawn preview"))
}

/// Opens `K` on page 1's first link.
fn key_open(session: &mut Session) -> Result<vpdf::split::Rect> {
  let app = session.app();
  let link = app.doc.links(PageId(0))?[0].bounds;
  let text = app.doc.text(PageId(0))?;
  let (x, y) = link.centre();
  app.go_to(text.nearest(PageId(0), x, y), true)?;
  app.preview_link()?;
  session.draw_frame(&mut Vec::new())?;
  drawn_preview(session)
}

/// Rests open passive previews.
#[test]
fn resting_on_links_previews_them() -> Result<()> {
  use vpdf::preview::Origin;
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let web = cell(&mut session, Hit::Link(PageId(0), 1))?;
    // Addresses never preview.
    mouse(&mut session, MouseEventKind::Moved, web)?;
    assert!(session.app().hover_due().is_none());
    mouse(&mut session, MouseEventKind::Moved, internal)?;
    assert!(!session.app().hover_preview(std::time::Instant::now()));
    let area = hover_open(&mut session, internal)?;
    let origin = session.app().peek.as_ref().map(|peek| peek.origin);
    assert_eq!(origin, Some(Origin::Hover(PageId(0), 0)));
    assert!(status(&mut session)?.contains("NORMAL"), "passive");
    // The row past the link keeps it.
    let below = area.row >= internal.1;
    let gap = if below { internal.1 + 1 } else { internal.1 - 1 };
    // In text, column 0 below is the next link.
    let column = internal.0.max(area.column);
    mouse(&mut session, MouseEventKind::Moved, (column, gap))?;
    assert!(session.app().peek.is_some(), "text={text_mode} gap");
    let inside = (area.column + area.columns / 2, area.row + area.rows / 2);
    mouse(&mut session, MouseEventKind::Moved, inside)?;
    assert!(session.app().peek.is_some(), "text={text_mode}");
    // Off both, it closes.
    let screen = session.app().screen;
    let away = (screen.columns - 1, screen.rows - 2);
    assert!(mouse(&mut session, MouseEventKind::Moved, away)?);
    assert!(session.app().peek.is_none(), "text={text_mode}");
  }
  Ok(())
}

/// Popups hide links; closing keeps them calm.
#[test]
fn popups_hide_links_beneath() -> Result<()> {
  use crossterm::event::KeyCode;
  use vpdf::terminal::Shape;
  // (text mode, popup) -> its hit's shape
  let cases = [
    (false, "menu", Shape::Pointer),
    (true, "menu", Shape::Pointer),
    (false, "hover", Shape::Pointer),
    (true, "hover", Shape::Pointer),
    // Clicks follow in both previews (U14).
    (false, "key", Shape::Pointer),
    (true, "key", Shape::Pointer),
  ];
  for (text_mode, popup, shape) in cases {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let area = match popup {
      "menu" => {
        right(&mut session, internal)?;
        session.app().context_menu.as_ref().expect("a menu").area()
      }
      "hover" => hover_open(&mut session, internal)?,
      _ => key_open(&mut session)?,
    };
    let app = session.app();
    let mut cells = (area.row..area.row + area.rows).flat_map(|row| {
      (area.column..area.column + area.columns).map(move |c| (c, row))
    });
    let under = cells.find(|&(column, row)| {
      let hit = app.hovered(Region::Page, column, row, None);
      matches!(hit, Ok(Hit::Link(..)))
    });
    let under = under.expect("a link under the popup");
    let at = format!("text={text_mode} {popup}");
    // Leaving would close a hover preview.
    if popup != "hover" {
      let screen = session.app().screen;
      let away = (screen.columns - 1, screen.rows - 2);
      mouse(&mut session, MouseEventKind::Moved, away)?;
    }
    let drew = mouse(&mut session, MouseEventKind::Moved, under)?;
    // Only a new shape shows.
    let shows = popup == "hover" || shape != Shape::Default;
    assert_eq!(drew, shows, "{at}: frame");
    let hover = session.app().hover;
    let link = matches!(hover, Hit::Link(..));
    assert!(!link && hover.shape() == shape, "{at}: {hover:?}");
    assert!(session.app().hover_link().is_none(), "{at}: no tint");
    // Previews name their destination (U24).
    let bar = status(&mut session)?;
    let label = (popup != "menu").then_some("→ p. 2 · Results");
    assert_eq!(bar.contains('→'), label.is_some(), "{at}: label");
    assert!(label.is_none_or(|label| bar.contains(label)), "{at}: {bar}");
    press(&mut session, KeyCode::Esc)?;
    session.draw_frame(&mut Vec::new())?;
    let app = session.app();
    assert!(app.context_menu.is_none() && app.peek.is_none(), "{at}");
    assert!(matches!(app.hover, Hit::Link(..)), "{at}: uncovered");
    assert!(app.hover_due().is_none(), "{at}: no rest");
  }
  Ok(())
}

/// Hits with the first cell showing each.
type Found = Vec<(Hit, (u16, u16))>;

/// First cell of each hit in `area`.
fn items(
  session: &mut Session,
  region: Region,
  area: vpdf::split::Rect,
) -> Result<Found> {
  let app = session.app();
  let mut found = Found::new();
  for row in area.row..area.row + area.rows {
    for column in area.column..area.column + area.columns {
      let hit = app.hovered(region, column, row, None)?;
      if hit != Hit::Nothing && found.iter().all(|(seen, _)| *seen != hit) {
        found.push((hit, (column, row)));
      }
    }
  }
  Ok(found)
}

/// Menu rows follow pointer and keys.
#[test]
fn menu_rows_follow_the_pointer() -> Result<()> {
  use crossterm::event::KeyCode;
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let plain = cell(&mut session, Hit::Text)?;
    right(&mut session, plain)?;
    let menu = session.app().context_menu.as_ref().expect("a menu").area();
    let rows = items(&mut session, Region::Menu, menu)?;
    let at = |index: usize| rows[index].1;
    let screen = session.app().screen;
    let away = (screen.columns - 1, screen.rows - 2);
    let selected = |session: &mut Session| {
      session.app().context_menu.as_ref().map(|m| m.selected)
    };
    // (move to, frame, selected)
    let steps = [
      (away, true, 0),
      // The shape changes on the selected row.
      (at(0), true, 0),
      (at(2), true, 2),
      ((at(2).0 + 3, at(2).1), false, 2),
      (at(1), true, 1),
    ];
    for (step, (to, frame, want)) in steps.into_iter().enumerate() {
      let drew = mouse(&mut session, MouseEventKind::Moved, to)?;
      let got = (drew, selected(&mut session));
      assert_eq!(got, (frame, Some(want)), "text={text_mode} {step}");
    }
    press(&mut session, KeyCode::Char('j'))?;
    assert_eq!(selected(&mut session), Some(2), "keys share it");
    // Nudges along the row, across links beneath, keep it.
    for column in menu.column..menu.column + menu.columns {
      let near = (column, at(1).1);
      assert!(!mouse(&mut session, MouseEventKind::Moved, near)?);
      assert_eq!(selected(&mut session), Some(2), "text={text_mode}");
    }
  }
  Ok(())
}

/// A bar item's first cell, freshly drawn.
fn bar_item(session: &mut Session, index: usize) -> Result<(u16, u16)> {
  status(session)?;
  let bar = session.app().bar_cells().expect("a drawn bar");
  let found = items(session, Region::Actions, bar)?;
  let item =
    found.iter().find(|(hit, _)| *hit == Hit::Item(List::Actions, Some(index)));
  Ok(item.expect("a shown item").1)
}

/// Bar items follow the pointer; clicks run.
#[test]
fn bar_items_follow_the_pointer() -> Result<()> {
  use crossterm::event::KeyCode;
  use vpdf::region::Operation;
  let (_directory, mut session) = words(false)?;
  let from = glyph_cell(&mut session, 0, 2)?;
  let to = glyph_cell(&mut session, 1, 9)?;
  let capture = |session: &mut Session| -> Result<()> {
    session.mouse(alt(MouseEventKind::Down(MouseButton::Left), from))?;
    session.mouse(alt(MouseEventKind::Drag(MouseButton::Left), to))?;
    session.mouse(alt(MouseEventKind::Up(MouseButton::Left), to))?;
    status(session).map(drop)
  };
  capture(&mut session)?;
  let action = |session: &mut Session| match &session.app().region {
    Some(vpdf::region::Region::Bar { action, .. }) => Some(*action),
    _ => None,
  };
  // Prefix and separators show nothing.
  let bar = session.app().bar_cells().expect("a drawn bar");
  let screen = session.app().screen;
  mouse(&mut session, MouseEventKind::Moved, (screen.columns - 1, 0))?;
  let prefix = (bar.column, bar.row);
  assert!(!mouse(&mut session, MouseEventKind::Moved, prefix)?, "a gap");
  // (item, nudge, frame, action)
  let steps = [
    (2, 0, true, Operation::Save),
    (2, 2, false, Operation::Save),
    (1, 0, true, Operation::Text),
  ];
  for (step, (item, nudge, frame, want)) in steps.into_iter().enumerate() {
    let (column, row) = bar_item(&mut session, item)?;
    let drew =
      mouse(&mut session, MouseEventKind::Moved, (column + nudge, row))?;
    assert_eq!((drew, action(&mut session)), (frame, Some(want)), "{step}");
  }
  press(&mut session, KeyCode::Char('l'))?;
  assert_eq!(action(&mut session), Some(Operation::Save), "keys share it");
  let text = bar_item(&mut session, 1)?;
  // A prompt owns the pointer first.
  press(&mut session, KeyCode::Char(':'))?;
  mouse(&mut session, MouseEventKind::Moved, (0, 0))?;
  mouse(&mut session, MouseEventKind::Moved, text)?;
  assert_eq!(action(&mut session), Some(Operation::Save), "prompt open");
  click(&mut session, text)?;
  let app = session.app();
  assert!(app.prompt.is_none() && app.reader.pending_yank.is_none());
  click(&mut session, text)?;
  let app = session.app();
  assert!(app.reader.pending_yank.is_some(), "Copy text ran");
  assert!(app.region.as_ref().is_some_and(vpdf::region::Region::is_bar));
  // Clicking off clears the bar hint.
  click(&mut session, (screen.columns - 1, 0))?;
  capture(&mut session)?;
  let hint = "h/l choose; Enter runs; Esc closes";
  assert_eq!(session.app().reader.message, hint);
  click(&mut session, (screen.columns - 1, 0))?;
  let app = session.app();
  assert!(app.region.is_none() && app.reader.message.is_empty());
  Ok(())
}

/// Rests follow the page link beneath popups.
#[test]
fn rests_follow_the_link_beneath() -> Result<()> {
  use vpdf::pointer::Hover;
  let (_directory, mut session) = session(false)?;
  let app = session.app();
  let on = |hit, beneath| {
    let region = Region::Page;
    Some(Hover { pane: app.id, region, hit, beneath })
  };
  let (first, third) = ((PageId(0), 0), (PageId(0), 2));
  let link = |(page, index)| Hit::Link(page, index);
  // (hover, calm first, preview due)
  let steps = [
    (on(link(first), Some(first)), false, true),
    (on(link(first), Some(first)), true, false),
    // A menu covers the calmed link, closes.
    (on(Hit::Item(List::Menu, Some(0)), Some(first)), false, false),
    (on(link(first), Some(first)), false, false),
    // The view moved another link under it.
    (on(Hit::Item(List::Menu, Some(0)), Some(first)), false, false),
    (on(link(third), Some(third)), false, true),
  ];
  for (step, (hover, calm, due)) in steps.into_iter().enumerate() {
    if calm {
      session.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
      })?;
    }
    let app = session.app();
    app.point(hover);
    assert_eq!(app.hover_due().is_some(), due, "step {step}");
  }
  Ok(())
}

/// Right drags pick; release runs.
#[test]
fn right_drags_choose_menu_items() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let plain = cell(&mut session, Hit::Text)?;
    let press = MouseEventKind::Down(MouseButton::Right);
    mouse(&mut session, press, plain)?;
    let menu = session.app().context_menu.as_ref().expect("a menu").area();
    let outline = labels(&mut session).iter().position(|l| *l == "Outline");
    let row = menu.row + u16::try_from(outline.expect("Outline"))?;
    let on = (menu.column + 2, row);
    let drag = MouseEventKind::Drag(MouseButton::Right);
    assert!(mouse(&mut session, drag, on)?, "text={text_mode}: picks");
    mouse(&mut session, MouseEventKind::Up(MouseButton::Right), on)?;
    let app = session.app();
    assert!(app.context_menu.is_none(), "text={text_mode}: ran");
    assert!(app.panel.is_some(), "text={text_mode}: outline open");
    app.close_panel()?;
    // Slips and releases off items run nothing.
    let up = MouseEventKind::Up(MouseButton::Right);
    let screen = session.app().screen;
    let off = if menu.column >= 3 { 0 } else { menu.column + menu.columns + 2 };
    let bottom = (plain.0, screen.rows - 2);
    // (press, drag path, release runs)
    let cases = [
      (plain, vec![(plain.0 + 1, plain.1)], false),
      (plain, vec![(plain.0, plain.1 + 1)], false),
      (plain, vec![(off, plain.1)], false),
      // Flipped above a bottom press.
      (bottom, vec![(bottom.0, bottom.1 - 1)], false),
      // Armed, then back on the first item.
      (plain, vec![(off, plain.1), plain], true),
    ];
    for (step, (from, path, runs)) in cases.into_iter().enumerate() {
      mouse(&mut session, press, from)?;
      for to in &path {
        mouse(&mut session, drag, *to)?;
      }
      let last = *path.last().expect("a path");
      mouse(&mut session, up, last)?;
      let open = session.app().context_menu.is_some();
      assert_eq!(open, !runs, "text={text_mode} {step}");
      session.app().context_menu = None;
      session.app().reader.cancel();
    }
  }
  Ok(())
}

/// The page scrolls under an open menu.
#[test]
fn wheel_scrolls_under_menus() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let plain = cell(&mut session, Hit::Text)?;
  right(&mut session, plain)?;
  let menu = session.app().context_menu.as_ref().expect("a menu").area();
  let before = session.app().reader.scroll;
  // (beside, over) the menu
  let spots =
    [(menu.column.saturating_sub(2), menu.row), (menu.column, menu.row)];
  let mut last = before;
  for at in spots {
    assert!(mouse(&mut session, MouseEventKind::ScrollDown, at)?, "{at:?}");
    let app = session.app();
    assert!(app.context_menu.is_some(), "the menu stays");
    let scroll = app.animation.as_ref().map_or(app.reader.scroll, |a| a.target);
    assert!(scroll > last, "the page scrolls at {at:?}");
    last = scroll;
  }
  Ok(())
}

/// Hint labels follow the pointer; clicks act.
#[test]
fn hint_labels_follow_the_pointer() -> Result<()> {
  use crossterm::event::KeyCode;
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    // Label cells; `copy` starts `F`.
    let start = |session: &mut Session, copy| -> Result<Found> {
      session.app().start_hints(copy)?;
      status(session)?;
      let screen = session.app().screen;
      let rows = screen.rows - 1;
      let page =
        vpdf::split::Rect { column: 0, row: 0, columns: screen.columns, rows };
      items(session, Region::Page, page)
    };
    let at = |labels: &Found, index: usize| {
      let hit = Hit::Item(List::Hints, Some(index));
      let label = labels.iter().find(|(seen, _)| *seen == hit);
      label.map(|(_, cell)| *cell).expect("a label")
    };
    let labels = start(&mut session, false)?;
    let internal = at(&labels, 0);
    assert!(mouse(&mut session, MouseEventKind::Moved, internal)?);
    assert!(!mouse(&mut session, MouseEventKind::Moved, internal)?, "B5");
    let picked = session.app().hints.as_ref().map(|h| h.selected);
    assert_eq!(picked, Some(Some(0)), "text={text_mode}");
    assert!(status(&mut session)?.contains("\x1b[0;97;44m"), "drawn picked");
    // Enter follows the picked label.
    press(&mut session, KeyCode::Enter)?;
    let app = session.app();
    assert!(app.hints.is_none(), "text={text_mode}");
    assert_eq!(app.reader.cursor.page, PageId(1), "text={text_mode}");
    action(app, Action::History(false))?;
    // `F` clicks copy addresses.
    let labels = start(&mut session, true)?;
    click(&mut session, at(&labels, 1))?;
    let app = session.app();
    assert!(app.hints.is_none() && app.reader.message == "Copied link");
    assert_eq!(app.reader.cursor.page, PageId(0), "text={text_mode}");
    let labels = start(&mut session, false)?;
    click(&mut session, at(&labels, 0))?;
    let app = session.app();
    assert!(app.hints.is_none(), "text={text_mode}");
    assert_eq!(app.reader.cursor.page, PageId(1), "text={text_mode}");
  }
  Ok(())
}

/// Typing drops the picked label.
#[test]
fn typing_drops_picked_labels() -> Result<()> {
  use vpdf::document::Target;
  use vpdf::hints::{Hint, Hints};
  let (_directory, mut session) = session(false)?;
  let hint = |label: &str| Hint {
    label: label.into(),
    target: Target::Page(PageId(1)),
    cell: (10, 2),
  };
  let app = session.app();
  app.hints = Some(Hints {
    items: vec![hint("aa"), hint("sa")],
    typed: String::new(),
    copy: false,
    selected: Some(1),
  });
  press(&mut session, crossterm::event::KeyCode::Char('a'))?;
  let hints = session.app().hints.as_ref().expect("hints stay");
  assert_eq!((hints.typed.as_str(), hints.selected), ("a", None));
  Ok(())
}

/// Clicks follow; keys and other presses close.
#[test]
fn hover_previews_answer_input() -> Result<()> {
  for text_mode in [false, true] {
    let (_directory, mut session) = session(text_mode)?;
    let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let area = hover_open(&mut session, internal)?;
    click(&mut session, (area.column + 1, area.row + 1))?;
    let app = session.app();
    assert!(app.peek.is_none());
    assert_eq!(app.reader.cursor.page, PageId(1), "text={text_mode}");
    action(app, Action::History(false))?;
    session.draw_frame(&mut Vec::new())?;
    hover_open(&mut session, internal)?;
    press(&mut session, crossterm::event::KeyCode::Esc)?;
    assert!(session.app().peek.is_none(), "text={text_mode}");
    assert!(session.app().hover_due().is_none(), "keys end the rest");
    let area = hover_open(&mut session, internal)?;
    let on = (area.column + 1, area.row + 1);
    mouse(&mut session, MouseEventKind::Down(MouseButton::Right), on)?;
    let app = session.app();
    assert!(app.peek.is_none() && app.context_menu.is_none());
    assert_eq!(app.reader.cursor.page, PageId(0), "text={text_mode}");
  }
  Ok(())
}

/// A middle click inside splits to the target (C2).
#[test]
fn middle_clicks_split_previews() -> Result<()> {
  for popup in ["hover", "key"] {
    let (_directory, mut session) = session(false)?;
    let link = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let area = match popup {
      "hover" => hover_open(&mut session, link)?,
      _ => key_open(&mut session)?,
    };
    let at = (area.column + 1, area.row + 1);
    mouse(&mut session, MouseEventKind::Down(MouseButton::Middle), at)?;
    mouse(&mut session, MouseEventKind::Up(MouseButton::Middle), at)?;
    session.settle()?;
    let panes = &session.tab().panes;
    assert_eq!(panes.len(), 2, "{popup}");
    assert!(panes.iter().all(|app| app.peek.is_none()), "{popup}");
    let shown: Vec<_> =
      panes.iter().map(|app| app.reader.cursor.page).collect();
    assert!(shown.contains(&PageId(1)), "{popup}: {shown:?}");
    assert_eq!(session.app().reader.cursor.page, PageId(0), "{popup}");
  }
  Ok(())
}

/// A click inside cancels pending keys (r15 #8).
#[test]
fn preview_clicks_cancel_keys() -> Result<()> {
  use crossterm::event::KeyCode;
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  press(&mut session, KeyCode::Char('g'))?;
  let area = hover_open(&mut session, internal)?;
  assert!(!session.app().keys.pending().is_empty(), "g waits");
  click(&mut session, (area.column + 1, area.row + 1))?;
  let app = session.app();
  assert_eq!(app.reader.cursor.page, PageId(1), "followed");
  assert!(app.keys.pending().is_empty(), "g cancelled");
  Ok(())
}

/// A click beside the link closes it.
#[test]
fn clicks_beside_links_close_previews() -> Result<()> {
  let (_directory, mut session) = session(true)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  hover_open(&mut session, internal)?;
  click(&mut session, (internal.0 + 40, internal.1))?;
  let app = session.app();
  assert!(app.peek.is_none());
  assert_eq!(app.reader.cursor.page, PageId(0), "nothing followed");
  Ok(())
}

/// Moving the view away closes it.
#[test]
fn scrolls_close_hover_previews() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  hover_open(&mut session, internal)?;
  session.app().reader.scroll += 300.0;
  session.draw_frame(&mut Vec::new())?;
  assert!(session.app().peek.is_none());
  Ok(())
}

/// Link boxes above the glyphs' descent.
fn tight_pdf(path: &Path) -> Result<()> {
  let page2 = "/Dest [6 0 R /XYZ 0 190 0]";
  lines_pdf(path, [("15 170 285 183", page2)].into_iter())
}

/// Gap pixel reports keep the preview (U2).
#[test]
fn pixel_gaps_keep_previews() -> Result<()> {
  let (_directory, mut session) = open(tight_pdf, false)?;
  let link = cell(&mut session, Hit::Link(PageId(0), 0))?;
  let area = hover_open(&mut session, link)?;
  let app = session.app();
  let bounds = app.doc.links(PageId(0))?[0].bounds;
  let rect = app.layout.project(PageId(0), bounds, app.reader.scroll);
  let height = app.screen.cell_height;
  let x = f32::midpoint(rect.x0, rect.x1) as u16;
  let moved = |y| Mouse {
    kind: MouseKind::Moved,
    at: Spot::Pixel { x, y },
    modifiers: KeyModifiers::NONE,
  };
  let gap = rect.y1 as u16 + 1..area.row * height;
  assert!(!gap.is_empty(), "the box stops above the frame");
  for y in gap {
    session.mouse_report(moved(y))?;
    let app = session.app();
    assert!(!matches!(app.hover, Hit::Link(..)), "{y}: off the box");
    assert!(app.peek.is_some(), "{y}: the gap keeps it");
  }
  session.mouse_report(moved((area.row + 2) * height))?;
  assert!(session.app().peek.is_some(), "inside");
  Ok(())
}

/// A link rectangle and its target.
type Annot<'a> = (&'a str, &'a str);

/// Sideways moves close the preview (r9 #1).
#[test]
fn sideways_moves_close_previews() -> Result<()> {
  let page2 = "/Dest [6 0 R /XYZ 0 190 0]";
  let home = "/Dest [3 0 R /XYZ 0 150 0]";
  let (left, right) = ("15 162 70 192", "75 162 285 192");
  // (links, what lies beside the first)
  let cases: [(&[Annot], Hit); 2] = [
    (&[(left, page2), (right, home)], Hit::Link(PageId(0), 1)),
    (&[(left, page2)], Hit::Text),
  ];
  for (links, beside) in cases {
    let write = |path: &Path| lines_pdf(path, links.iter().copied());
    let (_directory, mut session) = open(write, false)?;
    let link = cell(&mut session, Hit::Link(PageId(0), 0))?;
    let area = hover_open(&mut session, link)?;
    let app = session.app();
    let link_hit = Hit::Link(PageId(0), 0);
    let row = (0..area.row).rev().find(|&row| {
      app.hovered(Region::Page, link.0, row, None).ok() == Some(link_hit)
    });
    let row = row.expect("a link row above the frame");
    let on = |app: &mut vpdf::app::App, column| {
      app.hovered(Region::Page, column, row, None).ok()
    };
    let columns = app.screen.columns;
    let past = (link.0..columns).find(|&c| on(app, c) != Some(link_hit));
    // The bridge keeps the edge cell.
    let past = past.expect("the link's end") + 1;
    let column = (past..columns).find(|&c| on(app, c) == Some(beside));
    let to = (column.expect("a cell beside the link"), row);
    mouse(&mut session, MouseEventKind::Moved, to)?;
    assert!(session.app().peek.is_none(), "{beside:?}");
    let later = std::time::Instant::now() + vpdf::links::REST;
    let next = matches!(beside, Hit::Link(..));
    assert_eq!(session.hover_preview(later), next, "{beside:?}");
  }
  Ok(())
}

/// `K` previews follow clicks too (U14).
#[test]
fn key_previews_follow_clicks() -> Result<()> {
  let (left, right) = (MouseButton::Left, MouseButton::Right);
  // (button, inside) -> followed
  let cases = [(left, true, true), (right, true, false), (left, false, false)];
  for text_mode in [false, true] {
    for (button, inside, followed) in cases {
      let (_directory, mut session) = session(text_mode)?;
      let area = key_open(&mut session)?;
      let target = session.app().target;
      let screen = session.app().screen;
      let at = if inside {
        (area.column + 1, area.row + 1)
      } else {
        (screen.columns - 1, screen.rows - 2)
      };
      mouse(&mut session, MouseEventKind::Down(button), at)?;
      mouse(&mut session, MouseEventKind::Up(button), at)?;
      let app = session.app();
      let case = format!("text={text_mode} {button:?} inside={inside}");
      assert!(app.peek.is_none() && app.context_menu.is_none(), "{case}");
      let page = PageId(usize::from(followed));
      assert_eq!(app.reader.cursor.page, page, "{case}");
      if !followed {
        assert_eq!(app.target, target, "{case}: only dismissed");
      }
    }
  }
  Ok(())
}

/// Previews move, then close, with links (U17).
#[test]
fn previews_follow_links() -> Result<()> {
  let (_directory, mut session) = session(false)?;
  let first = key_open(&mut session)?;
  let screen = session.app().screen;
  let away = (screen.columns - 1, screen.rows - 2);
  let mut rows = vec![first.row];
  for _ in 0..10 {
    mouse(&mut session, MouseEventKind::ScrollDown, away)?;
    let app = session.app();
    if let Some(scroll) = app.animation.take() {
      app.reader.scroll = scroll.target;
    }
    session.draw_frame(&mut Vec::new())?;
    match session.app().preview_cells() {
      Some(area) => rows.push(area.row),
      None => break,
    }
  }
  assert!(session.app().peek.is_none(), "closed: {rows:?}");
  assert!(rows.windows(2).all(|pair| pair[1] < pair[0]), "moved: {rows:?}");
  assert!(rows.len() > 1, "shown while the link was: {rows:?}");
  // A scroll before its first frame still closes it.
  session.app().reader.scroll = 0.0;
  key_open(&mut session)?;
  session.app().peek.as_mut().expect("a preview").placement = None;
  session.app().reader.scroll += 300.0;
  session.draw_frame(&mut Vec::new())?;
  assert!(session.app().peek.is_none(), "closed before a frame");
  Ok(())
}

/// A split session, other pane, link cell.
type Split = (tempfile::TempDir, Session, usize, (u16, u16));

/// A split; the other pane and its link.
fn split() -> Result<Split> {
  let (directory, mut session) = session(false)?;
  session.app().command("vsplit")?;
  session.settle()?;
  let index = 1 - session.tab().focus;
  let link = cell_in(&mut session, index, Hit::Link(PageId(0), 0))?;
  Ok((directory, session, index, link))
}

/// The pane under the pointer previews (U15).
#[test]
fn unfocused_panes_preview_too() -> Result<()> {
  let (left, right) = (MouseButton::Left, MouseButton::Right);
  // (button, inside) -> focus moves, followed
  let cases = [
    (left, true, true, true),
    (right, true, true, false),
    (left, false, false, false),
  ];
  for (button, inside, moves, followed) in cases {
    let (_directory, mut session, index, link) = split()?;
    let focused = session.app().id;
    let other = session.tab().panes[index].id;
    let area = hover_open(&mut session, link)?;
    assert_eq!(session.app().id, focused, "rests keep focus");
    let showing = session.tab().panes.iter().find(|app| app.peek.is_some());
    assert_eq!(showing.map(|app| app.id), Some(other));
    let app = session.app();
    let blank = (app.at.column + 1, app.at.row);
    let at = if inside { (area.column + 1, area.row + 1) } else { blank };
    mouse(&mut session, MouseEventKind::Down(button), at)?;
    mouse(&mut session, MouseEventKind::Up(button), at)?;
    let case = format!("{button:?} inside={inside}");
    let app = session.app();
    assert_eq!(app.id, if moves { other } else { focused }, "{case}");
    assert!(session.tab().panes.iter().all(|app| app.peek.is_none()));
    let page = session.app().reader.cursor.page;
    assert_eq!(page, PageId(usize::from(followed)), "{case}");
  }
  Ok(())
}

/// Focused overlays hold rests in other panes.
#[test]
fn focused_overlays_hold_rests() -> Result<()> {
  for hold in ["menu", "prompt", "recent", "key"] {
    let (_directory, mut session, _, link) = split()?;
    match hold {
      "menu" => {
        let plain = cell(&mut session, Hit::Text)?;
        right(&mut session, plain)?;
      }
      "prompt" => {
        session.app().prompt = Some(Prompt::new(PromptKind::Command));
      }
      "recent" => {
        session.app().command("recent")?;
        session.settle()?;
        assert!(session.app().covered(), "the recent view");
      }
      _ => {
        key_open(&mut session)?;
      }
    }
    mouse(&mut session, MouseEventKind::Moved, link)?;
    let later = std::time::Instant::now() + vpdf::links::REST;
    assert!(!session.hover_preview(later), "{hold}");
  }
  Ok(())
}

/// Owned overlays and presses hold previews back.
#[test]
fn rests_wait_for_free_input() -> Result<()> {
  use crossterm::event::KeyCode;
  let (_directory, mut session) = session(false)?;
  let internal = cell(&mut session, Hit::Link(PageId(0), 0))?;
  let plain = cell(&mut session, Hit::Text)?;
  // An open menu blocks the rest.
  right(&mut session, plain)?;
  mouse(&mut session, MouseEventKind::Moved, internal)?;
  assert!(session.app().hover_due().is_none(), "menu open");
  press(&mut session, KeyCode::Esc)?;
  // A press ends the rest.
  mouse(&mut session, MouseEventKind::Moved, plain)?;
  mouse(&mut session, MouseEventKind::Moved, internal)?;
  assert!(session.app().hover_due().is_some());
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), internal)?;
  assert!(session.app().hover_due().is_none(), "a press");
  // Unfocused panes rest by the same rules (r15 #1).
  let (_directory, mut session, index, link) = split()?;
  mouse(&mut session, MouseEventKind::Moved, link)?;
  let first = session.tab().panes[index].hover_due().expect("a rest");
  std::thread::sleep(std::time::Duration::from_millis(5));
  mouse(&mut session, MouseEventKind::Moved, link)?;
  let again = session.tab().panes[index].hover_due().expect("a rest");
  assert!(again > first, "motion restarts it");
  let plain = cell(&mut session, Hit::Text)?;
  right(&mut session, plain)?;
  mouse(&mut session, MouseEventKind::Moved, link)?;
  mouse(&mut session, MouseEventKind::Down(MouseButton::Left), link)?;
  assert!(session.app().context_menu.is_none(), "the press dismissed");
  assert!(session.tab().panes[index].hover_due().is_none(), "a press");
  Ok(())
}
