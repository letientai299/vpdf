use super::common::keys;
use super::focus::{screen, send};
use crate::harness::{corpus, solo, temp_copy};
use vpdf::Result;
use vpdf::solo::Solo;
use vpdf_model::document::Target;
use vpdf_model::text::PageId;
use vpdf_reader::reader::{Mode, SelectionKind};
use vpdf_ui::input::InputOwner;

pub(super) fn open(text: bool) -> Result<Solo> {
  let mut app = solo("paper.pdf", text, screen())?;
  let link = app
    .doc
    .links(PageId(1))?
    .iter()
    .find(|link| !matches!(link.target, Target::Uri(_)))
    .cloned()
    .unwrap();
  let position = app.doc.text(PageId(1))?.nearest(
    PageId(1),
    f32::midpoint(link.first().x0, link.first().x1),
    f32::midpoint(link.first().y0, link.first().y1),
  );
  app.go_to(position, true)?;
  Ok(app)
}

fn target_page(target: &Target) -> PageId {
  match *target {
    Target::Page(page) | Target::Point { page, .. } => page,
    Target::Uri(_) => unreachable!(),
  }
}

fn drawn(app: &mut Solo) -> Result<String> {
  app.graphics.begin(&mut std::io::sink())?;
  {
    let (app, graphics) = app.parts();
    vpdf::presentation::try_draw(&mut std::io::sink(), Some(graphics), |s| {
      app.scene_preview(s)
    })?;
  }
  app.graphics.end(&mut std::io::sink())?;
  super::common::preview_frame(app)
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_scroll_clamps_without_moving_target() -> Result<()> {
  for text in [false, true] {
    for entry in [b"".as_slice(), b"i", b"iv"] {
      let mut app = open(text)?;
      keys(&mut app, entry)?;
      keys(&mut app, b"K")?;
      let target = format!("{:?}", app.overlays.preview().unwrap().target());
      let page = target_page(app.overlays.preview().unwrap().target());
      let label = app.doc.label(page).to_owned();
      let cursor = app.reader.cursor;
      let range = app.reader.selection.map(|s| (s.anchor, s.kind));
      let view = (app.reader.scroll, app.text().position(0, 0)?);
      keys(&mut app, b"9999k")?;
      let first = drawn(&mut app)?;
      assert!(first.contains(&format!("╭ {label} ")));
      assert!(!first.contains('↑'));
      assert!(first.contains('↓'));
      keys(&mut app, b"3j")?;
      let advanced = drawn(&mut app)?;
      assert!(advanced.contains('↑'));
      assert_ne!(advanced, first);
      keys(&mut app, b"3k")?;
      assert_eq!(drawn(&mut app)?, first);
      keys(&mut app, b"2\x1b[100;5u")?;
      assert!(drawn(&mut app)?.contains('↑'));
      keys(&mut app, b"2\x1b[117;5u")?;
      assert_eq!(drawn(&mut app)?, first);
      keys(&mut app, b"9999j")?;
      let last = drawn(&mut app)?;
      assert!(!last.contains('↓'));
      // It runs to the last page (U12).
      assert!(!last.contains(&format!("╭ {label} ")), "text={text}");
      keys(&mut app, b"9999j")?;
      assert_eq!(drawn(&mut app)?, last);
      assert_eq!(
        format!("{:?}", app.overlays.preview().unwrap().target()),
        target
      );
      assert_eq!(app.reader.cursor, cursor);
      assert_eq!(app.reader.selection.map(|s| (s.anchor, s.kind)), range);
      assert_eq!((app.reader.scroll, app.text().position(0, 0)?), view);
      keys(&mut app, b"\r")?;
      assert!(app.overlays.preview().is_none());
      let destination = target_page(&app.link_target(cursor)?.unwrap());
      assert_eq!(app.reader.cursor.page, destination);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_prefix_prompt_and_escape_keep_range() -> Result<()> {
  for text in [false, true] {
    let mut app = open(text)?;
    keys(&mut app, b"ivK")?;
    let cursor = app.reader.cursor;
    let anchor = app.reader.selection.unwrap().anchor;
    let target = format!("{:?}", app.overlays.preview().unwrap().target());
    keys(&mut app, b"g")?;
    assert_eq!(app.keys.awaiting(), Some("g"));
    assert_eq!(app.input_owner(), InputOwner::Peek);
    keys(&mut app, b"\x1b[27u")?;
    assert!(app.overlays.preview().is_some() && app.keys.awaiting().is_none());
    keys(&mut app, b"gglwi")?;
    assert_eq!(app.reader.cursor, cursor);
    for prompt in [b":\x1b[27u".as_slice(), b":\r", b":nohlsearch\r"] {
      keys(&mut app, prompt)?;
      assert_eq!(app.input_owner(), InputOwner::Peek);
      assert_eq!(
        format!("{:?}", app.overlays.preview().unwrap().target()),
        target
      );
      assert_eq!(app.reader.cursor, cursor);
      assert_eq!(app.reader.selection.unwrap().anchor, anchor);
    }
    assert!(keys(&mut app, b":unknown\r").is_err());
    assert_eq!(app.input_owner(), InputOwner::Peek);
    assert_eq!(
      format!("{:?}", app.overlays.preview().unwrap().target()),
      target
    );
    keys(&mut app, b"\x1b[27u")?;
    assert!(app.overlays.preview().is_none());
    assert_eq!(app.reader.mode(), Mode::Visual(SelectionKind::Character));
    keys(&mut app, b"\x1b[27u")?;
    assert_eq!(app.reader.mode(), Mode::Cursor);
    keys(&mut app, b"\x1b[27u")?;
    assert_eq!(app.reader.mode(), Mode::Normal);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_pane_chord_dispatches_once() -> Result<()> {
  let app = open(false)?;
  let cursor = app.reader.cursor;
  let mut session = crate::harness::session(&app.path, screen())?;
  session.app().go_to(cursor, true)?;
  send(&mut session, b"K3g")?;
  assert_eq!(session.app().input_owner(), InputOwner::Peek);
  send(&mut session, b"t")?;
  assert!(session.app().overlays.preview().is_none());
  session.app().go_to(cursor, true)?;
  send(&mut session, b"K\x1b[119;5u")?;
  assert!(session.app().overlays.preview().is_none());
  assert_eq!(session.app().keys.awaiting(), Some("C-w"));
  send(&mut session, b"v")?;
  assert_eq!(session.tabs[session.active].panes.len(), 2);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_border_stays_inside_resized_pane() -> Result<()> {
  use unicode_width::UnicodeWidthStr;
  use vpdf_ui::keys::PanelKind;
  use vpdf_ui::layout::Crop;
  for text in [false, true] {
    let mut app = open(text)?;
    let cursor = app.reader.cursor;
    app.open_panel(PanelKind::Help)?;
    app.panel.as_mut().unwrap().view.focus(false);
    app.go_to(cursor, true)?;
    keys(&mut app, b"K")?;
    let target = format!("{:?}", app.overlays.preview().unwrap().target());
    app.at = vpdf_model::geometry::At { column: 7, row: 3 };
    app.screen.origin = (7, 3);
    for rotation in [0, 90, 180, 270] {
      app.screen.columns = 78;
      app.screen.rows = 19;
      app.layout.restore(1.5, rotation, Crop([50, 50, 50, 50]));
      app.relayout()?;
      let out = drawn(&mut app)?;
      assert_eq!(out.matches('╭').count(), 1);
      assert_eq!(out.matches('╰').count(), 1);
      let label = out.split_once('╭').unwrap().1.split('╮').next().unwrap();
      assert!(label.width() + 2 <= usize::from(app.layout.screen().columns));
      for part in out.split("\x1b[") {
        let Some((position, _)) = part.split_once('H') else { continue };
        let Some((row, column)) = position.split_once(';') else { continue };
        let (Ok(row), Ok(column)) = (row.parse::<u16>(), column.parse::<u16>())
        else {
          continue;
        };
        assert!(row > app.at.row && row < app.at.row + app.screen.rows);
        assert!(column > app.at.column + app.left);
        assert!(column <= app.at.column + app.screen.columns);
      }
      assert_eq!(
        format!("{:?}", app.overlays.preview().unwrap().target()),
        target
      );
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_reload_discards_target() -> Result<()> {
  for text in [false, true] {
    let mut app = open(text)?;
    let (_directory, path) = temp_copy("paper.pdf")?;
    app.path.clone_from(&path);
    keys(&mut app, b"ivK")?;
    let target = format!("{:?}", app.overlays.preview().unwrap().target());
    let cursor = app.reader.cursor;
    let range = app.reader.selection.unwrap();
    std::fs::remove_file(&path)?;
    assert!(app.command("reload").is_err());
    assert_eq!(
      format!("{:?}", app.overlays.preview().unwrap().target()),
      target
    );
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, range.anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(range.kind));
    std::fs::copy(corpus("form.pdf")?, &path)?;
    let shorter = vpdf_pdf::document::Document::open(&path, "")?.pages.len();
    assert!(shorter <= target_page(app.overlays.preview().unwrap().target()).0);
    app.command("reload")?;
    assert!(app.overlays.preview().is_none());
    assert_eq!(app.doc.pages.len(), shorter);
    assert_eq!(app.reader.cursor.page, PageId(cursor.page.0.min(shorter - 1)));
    assert_eq!(app.reader.mode(), Mode::Cursor);
    assert_eq!(drawn(&mut app)?, "");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_invalid_keeps_reader() -> Result<()> {
  for text in [false, true] {
    let mut app = open(text)?;
    keys(&mut app, b"ivK")?;
    let cursor = app.reader.cursor;
    let range = app.reader.selection.unwrap();
    app.doc = vpdf_pdf::document::Document::open(&corpus("form.pdf")?, "")?;
    {
      let (app, _) = app.parts();
      app.layout.reflow(app.doc.pages.iter().map(|page| page.bounds));
    }
    assert_eq!(drawn(&mut app)?, "");
    assert!(app.overlays.preview().is_none());
    assert_eq!(app.reader.cursor, cursor);
    assert_eq!(app.reader.selection.unwrap().anchor, range.anchor);
    assert_eq!(app.reader.mode(), Mode::Visual(range.kind));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_label_keeps_arrow() -> Result<()> {
  use unicode_width::UnicodeWidthStr;
  for text in [false, true] {
    let mut app = open(text)?;
    keys(&mut app, b"K9999k3j")?;
    let page = target_page(app.overlays.preview().unwrap().target());
    let wide = format!("界{}", app.doc.label(page)).repeat(200);
    app.doc.pages[page.0].label = wide.into();
    let out = drawn(&mut app)?;
    let border = out.split_once('╭').unwrap().1.split('╮').next().unwrap();
    assert!(border.contains('↑'));
    assert!(border.width() + 2 <= usize::from(app.layout.screen().columns));
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_previews_load_in_steps() -> Result<()> {
  let mut app = open(true)?;
  let links = app.doc.links(PageId(1))?;
  let early = links.iter().find(|link| {
    matches!(link.target, Target::Page(page) | Target::Point { page, .. }
      if page.0 < 5)
  });
  let link = early.cloned().expect("a link to an early page");
  let (x, y) = link.first().centre();
  let position = app.doc.text(PageId(1))?.nearest(PageId(1), x, y);
  app.go_to(position, true)?;
  keys(&mut app, b"K9999j")?;
  // Four pages per scroll; more remain.
  assert!(drawn(&mut app)?.contains('↓'), "pages remain");
  for _ in 0..4 {
    keys(&mut app, b"9999j")?;
  }
  assert!(!drawn(&mut app)?.contains('↓'), "the last page");
  Ok(())
}
