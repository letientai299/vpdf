use super::common::SeedReader;
use super::common::keys;
use crate::harness::solo;
use vpdf::Result;
use vpdf::presentation::try_draw;
use vpdf::solo::Solo;
use vpdf_model::text::{Bounds, PageId, Position};
use vpdf_reader::reader::SelectionKind;
use vpdf_ui::layout::{Crop, Screen};
const BAND: &str = "\x1b[0;100m";
const SELECTION: &str = "\x1b[0;7;35m";
const CURSOR: &str = "\x1b[0;7;34m";
const LANDING: &str = "\x1b[0;43m";
const SEARCH: &str = "\x1b[0;7;33m";

fn app(text: bool) -> Result<Solo> {
  solo(
    "paper.pdf",
    text,
    Screen { origin: (7, 3), ..crate::common::cells(100, 24) },
  )
}

fn marks(app: &mut Solo, screen: Screen) -> Result<Vec<Bounds>> {
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  {
    let (app, graphics) = app.parts();
    try_draw(&mut out, Some(graphics), |s| app.scene_marks(s, screen))?;
  }
  app.graphics.end(&mut out)?;
  let frame = String::from_utf8(out)?;
  let commands = crate::kitty::commands(&frame);
  let marks =
    commands.iter().filter(|c| matches!(c.get("z"), Some("1" | "-1")));
  Ok(marks.map(|c| c.bounds(screen.origin).expect("mark bounds")).collect())
}

fn transformed(zoom: f32, rotation: u16) -> Result<(Solo, usize)> {
  let mut app = app(false)?;
  app.screen.columns = 40;
  app.screen.rows = 15;
  app.relayout()?;
  app.layout.restore(zoom, rotation, Crop([30, 30, 30, 30]));
  {
    let (app, _) = app.parts();
    app.layout.reflow(app.doc.pages.iter().map(|page| page.bounds));
  }
  let text = app.doc.text(PageId(0))?;
  let line = text.lines.iter().position(|line| line.glyphs.len() > 10).unwrap();
  Ok((app, line))
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_switches_in_cursor_mode() -> Result<()> {
  for text in [false, true] {
    let mut app = app(text)?;
    keys(&mut app, b"i")?;
    let cursor = app.reader.cursor;
    for (command, expected) in [
      ("cursorline off", false),
      ("cursorline off", false),
      ("cursorline on", true),
      ("cursorline on", true),
      ("cursorline", false),
      ("cursorline", true),
    ] {
      app.command(command)?;
      assert_eq!(app.cursorline, expected, "{command}");
      assert_eq!(app.reader.mode(), vpdf_reader::reader::Mode::Cursor);
      assert_eq!(app.reader.cursor, cursor);
      assert!(app.reader.selection.is_none());
      if text {
        let mut out = Vec::new();
        try_draw(&mut out, None, |s| app.scene_text(s))?;
        assert_eq!(String::from_utf8(out)?.contains(BAND), expected);
      } else {
        let screen = app.layout.screen();
        assert_eq!(
          marks(&mut app, screen)?.len(),
          if expected { 2 } else { 1 }
        );
      }
    }
    for argument in ["invalid", "on off"] {
      let error = app.command(&format!("cursorline {argument}")).unwrap_err();
      assert_eq!(error.to_string(), "Usage: :cursorline [on|off]");
      assert!(app.cursorline);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_toggle_preserves_visual_and_copy() -> Result<()> {
  for text in [false, true] {
    let mut app = app(text)?;
    assert!(app.cursorline);
    keys(&mut app, b"vll:cursorline\r")?;
    let range = (app.reader.selection.unwrap().anchor, app.reader.cursor);
    assert!(!app.cursorline);
    keys(&mut app, b":cursorline\r")?;
    assert!(app.cursorline);
    assert_eq!(
      (app.reader.selection.unwrap().anchor, app.reader.cursor),
      range
    );
    keys(&mut app, b"y")?;
    assert!(app.reader.selection.is_none());
    let yank = app.reader.pending_yank.as_ref().unwrap().text.clone();
    app.command("cursorline")?;
    assert_eq!(app.reader.pending_yank.as_ref().unwrap().text, yank);
    if !text {
      let screen = app.layout.screen();
      assert_eq!(marks(&mut app, screen)?, [] as [Bounds; 0]);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_pdf_projects_and_clips() -> Result<()> {
  for rotation in [0, 90, 180, 270] {
    for zoom in [0.5, 2.0] {
      let (mut app, line) = transformed(zoom, rotation)?;
      app.reader.seed_reader(|reader| {
        reader.cursor = Position { page: PageId(0), line, glyph: 5 }
      });
      app.reader.visual(SelectionKind::Character);
      app.ensure_cursor()?;
      app.left = 10;
      let screen = Screen { rows: 14, ..app.layout.screen() };
      let on = marks(&mut app, screen)?;
      app.command("cursorline")?;
      let off = marks(&mut app, screen)?;
      assert!(on.len() > off.len(), "{rotation} {zoom}");
      assert_eq!(off.len(), 2, "selection and cursor remain");
      let band = on[0];
      assert!(band.width() > off[0].width() || band.height() > off[0].height());
      for rect in on {
        assert!(rect.x0 >= 80.0 && rect.x1 <= 80.0 + screen.width());
        assert!(rect.y0 >= 0.0 && rect.y1 <= screen.height());
      }
      app.reader.end_selection();
      app.command("cursorline")?;
      assert_eq!(marks(&mut app, screen)?, [] as [Bounds; 0]);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_text_fills_only_active_content_row() -> Result<()> {
  let mut app = app(true)?;
  app.screen.columns = 20;
  app.left = 4;
  app.reader.seed_reader(|reader| reader.cursor = Position::default());
  let rows = ["abcd", "", "other"];
  let lines = crate::fixtures::lines(rows.map(crate::fixtures::plain));
  app.doc.seed(PageId(0), lines);
  app.reader.visual(SelectionKind::Character);
  app.reader.seed_reader(|reader| reader.cursor.glyph = 1);
  let hit = vpdf_search::search::Hit {
    first: Position { glyph: 2, ..Position::default() },
    last: Position { glyph: 2, ..Position::default() },
    excerpt: "c".into(),
    matched: 0..0,
  };
  app.search.show(vpdf_search::query::Query::default(), vec![hit]);
  let mut out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  let fill = format!("{}{}", BAND, " ".repeat(16));
  assert_eq!(frame.matches(&fill).count(), 1);
  assert!(frame.starts_with(&format!(
    "{}{}",
    vpdf_terminal::terminal::goto(app.at, 0_usize, app.left),
    fill
  )));
  for style in [SELECTION, CURSOR, SEARCH] {
    assert!(frame.contains(style));
  }
  app.command("cursorline")?;
  let mut out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  assert!(!frame.contains(BAND));
  for style in [SELECTION, CURSOR, SEARCH] {
    assert!(frame.contains(style));
  }
  app.command("cursorline")?;
  app.reader.seed_reader(|reader| reader.cursor.line = 1);
  out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  assert!(frame.contains(&format!(
    "{}{}",
    vpdf_terminal::terminal::goto(app.at, 1_usize, app.left),
    fill
  )));
  app.reader.end_selection();
  let mut out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  assert!(!frame.contains(BAND));
  assert!(!frame.contains(CURSOR));
  assert!(frame.contains(SEARCH));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn cursorline_stays_below_existing_selection() -> Result<()> {
  let mut app = app(false)?;
  app.command("2")?;
  app.command("cursorline")?;
  app.reader.visual(SelectionKind::Character);
  let screen = app.layout.screen();
  marks(&mut app, screen)?;
  app.command("cursorline")?;
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  {
    let (app, graphics) = app.parts();
    try_draw(&mut out, Some(graphics), |s| app.scene_marks(s, screen))?;
  }
  app.graphics.end(&mut out)?;
  let frame = String::from_utf8(out)?;
  assert!(frame.contains("z=-1,"), "band stays below selection images");
  assert!(frame.contains("z=1,"), "selection remains above the band");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_jump_modes_and_fixed_destination() -> Result<()> {
  for text_mode in [false, true] {
    for mode in [b"".as_slice(), b"i", b"iv"] {
      let mut app = app(text_mode)?;
      keys(&mut app, mode)?;
      let initial_mode = app.reader.mode();
      let target = Position { line: 5, ..Position::default() };
      app.go_to(target, true)?;
      let target = app.reader.cursor;
      assert_eq!(app.reader.landing().position().unwrap(), target);
      assert_eq!(app.reader.mode(), initial_mode);
      let until = app.reader.landing().deadline().unwrap();
      keys(&mut app, b"j")?;
      assert_eq!(app.reader.landing().position().unwrap(), target);
      assert_eq!(app.reader.landing().deadline().unwrap(), until);
      app.reader.clear_flash();
      keys(&mut app, b"j\x04")?;
      assert!(
        app.reader.landing().position().is_none(),
        "ordinary motions must not flash"
      );
      app.marks.insert('a', target);
      app.go_mark('a', true)?;
      assert_eq!(app.reader.landing().position().unwrap(), target);
      app.reader.clear_flash();
      app.follow(&vpdf_model::document::Target::Page(PageId(1)))?;
      assert_eq!(app.reader.landing().position().unwrap(), app.reader.cursor);
      assert_eq!(app.reader.cursor.page, PageId(1));
      app.reader.clear_flash();
      super::common::outline_loaded(&mut app)?;
      keys(&mut app, b"]]")?;
      assert_eq!(app.reader.landing().position().unwrap(), app.reader.cursor);
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_pdf_reuses_projected_clipped_band() -> Result<()> {
  for rotation in [0, 90, 180, 270] {
    for zoom in [0.5, 2.0] {
      let (mut app, line) = transformed(zoom, rotation)?;
      app.go_to(Position { page: PageId(0), line, glyph: 5 }, true)?;
      app.left = 10;
      let screen = Screen { rows: 14, ..app.layout.screen() };
      let band = marks(&mut app, screen)?;
      assert!(!band.is_empty(), "Normal has landing feedback");
      app.reader.clear_flash();
      app.reader.visual(SelectionKind::Character);
      let visual = marks(&mut app, screen)?;
      let coords = |b: Bounds| (b.x0, b.y0, b.x1, b.y1);
      for (flash, cursorline) in band.iter().zip(&visual) {
        assert_eq!(coords(*flash), coords(*cursorline), "{rotation} {zoom}");
        assert!(flash.x0 >= 80.0 && flash.x1 <= 80.0 + screen.width());
        assert!(flash.y0 >= 0.0 && flash.y1 <= screen.height());
      }
      let cursor = app.reader.cursor;
      app.go_to(cursor, true)?;
      let mut out = Vec::new();
      app.graphics.begin(&mut out)?;
      {
        let (app, graphics) = app.parts();
        try_draw(&mut out, Some(graphics), |s| app.scene_marks(s, screen))?;
      }
      app.graphics.end(&mut out)?;
      let frame = String::from_utf8(out)?;
      assert!(frame.contains("z=-1,"));
      assert!(frame.contains("z=1,"));
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_text_fills_row_below_glyph_styles() -> Result<()> {
  let mut app = app(true)?;
  app.left = 4;
  let position = Position { line: 5, glyph: 1, ..Position::default() };
  app.go_to(position, true)?;
  let position = app.reader.cursor;
  let hit = vpdf_search::search::Hit {
    first: Position { glyph: 3, ..position },
    last: Position { glyph: 3, ..position },
    excerpt: String::new(),
    matched: 0..0,
  };
  app.search.show(vpdf_search::query::Query::default(), vec![hit]);
  let mut out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  let fill = format!("{}{}", LANDING, " ".repeat(96));
  assert_eq!(frame.matches(&fill).count(), 1);
  assert!(!frame.contains(CURSOR));
  assert!(frame.contains(SEARCH));
  keys(&mut app, b"ivl")?;
  out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  for style in [LANDING, CURSOR, SELECTION, SEARCH] {
    assert!(frame.contains(style), "missing {style:?}");
  }
  app.reader.clear_flash();
  out = Vec::new();
  try_draw(&mut out, None, |s| app.scene_text(s))?;
  let frame = String::from_utf8(out)?;
  assert!(!frame.contains(LANDING));
  assert!(frame.contains(BAND));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_search_acceptance_and_cancellation() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app(text_mode)?;
    app.begin_search(false)?;
    app.incremental("attention")?;
    super::common::search_loaded(&mut app)?;
    assert!(app.reader.landing().position().is_none(), "typing only previews");
    app.abort_search()?;
    assert!(app.reader.landing().position().is_none(), "canceling never jumps");
    app.begin_search(false)?;
    app.incremental("attention")?;
    super::common::search_loaded(&mut app)?;
    app.finish_search("attention", false)?;
    assert_eq!(app.reader.landing().position().unwrap(), app.reader.cursor);
    app.reader.clear_flash();
    app.next_result(false, 1)?;
    assert_eq!(app.reader.landing().position().unwrap(), app.reader.cursor);
    app.reader.clear_flash();
    app.start_search("model", false, false)?;
    super::common::search_loaded(&mut app)?;
    assert_eq!(app.reader.landing().position().unwrap(), app.reader.cursor);
    app.reader.clear_flash();
    app.start_search("qqqnothingzzznomatch", false, false)?;
    super::common::search_loaded(&mut app)?;
    assert!(app.reader.landing().position().is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_reload_clears_feedback() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = app(text_mode)?;
    app.go_to(Position { line: 5, ..Position::default() }, true)?;
    assert!(app.reader.landing().position().is_some());
    app.reload_with("")?;
    assert!(app.reader.landing().position().is_none());
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn landing_search_early_commit_uses_current_query() -> Result<()> {
  for text_mode in [false, true] {
    for query in ["model", "qqqnothingzzznomatch"] {
      let mut app = app(text_mode)?;
      app.begin_search(false)?;
      app.incremental("attention")?;
      super::common::search_loaded(&mut app)?;
      let previous = app.reader.cursor;
      assert!(app.reader.landing().position().is_none());
      app.incremental(query)?;
      assert!(matches!(
        app.search.phase(),
        vpdf_ui::find::Phase::Running { jump: true }
      ));
      app.finish_search(query, false)?;
      assert_eq!(app.reader.cursor, previous);
      assert!(
        app.reader.landing().position().is_none(),
        "previous query must not flash"
      );
      super::common::search_loaded(&mut app)?;
      assert_eq!(app.search.query().text, query);
      if query == "model" {
        let target = app.reader.landing().position().unwrap();
        assert_eq!(target, app.reader.cursor);
        assert!(app.search.hits().iter().any(|hit| hit.first == target));
      } else {
        assert!(app.search.hits().is_empty());
        assert!(
          app.reader.landing().position().is_none(),
          "failed query must not flash"
        );
      }
    }
  }
  Ok(())
}
