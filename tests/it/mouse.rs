use crate::common;
use crate::focus::{action, screen};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::fmt::Write as _;
use std::path::Path;
use vpdf::Result;
use vpdf::app::{Request, TabMove};
use vpdf::decode::{Button, Mouse, MouseKind, Spot};
use vpdf::keys::Action;
use vpdf::pointer::{Hit, Region};
use vpdf::prompt::{Prompt, PromptKind};
use vpdf::session::Session;
use vpdf::text::PageId;
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

fn session(text_mode: bool) -> Result<(tempfile::TempDir, Session)> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("links.pdf");
  link_pdf(&path)?;
  let mut session = Session::new(None);
  session.open(&path, true, screen())?;
  let app = session.app();
  common::outline_loaded(app)?;
  if text_mode {
    app.open_panel(vpdf::keys::PanelKind::Text)?;
    app.close_panel()?;
    app.draw(&mut Vec::new())?;
  }
  assert_eq!(app.text_mode, text_mode);
  Ok((directory, session))
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
