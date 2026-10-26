use super::common::{SeedReader, keys};
use crate::harness::solo;
use vpdf::Result;
use vpdf::app::App;
use vpdf::solo::Solo;
use vpdf_model::document::{Heading, Target};
use vpdf_model::text::{PageId, Position};
use vpdf_reader::reader::Reader;
use vpdf_ui::keys::PanelKind;
use vpdf_ui::panel::Destination;

#[test]
fn jumps_include_both_history_directions() {
  let mut reader = Reader::default();
  assert_eq!(reader.jumps().count(), 0);
  for page in 0..3 {
    reader.cursor.page = PageId(page);
    reader.remember();
  }
  reader.cursor.page = PageId(3);
  for (forward, expected) in [
    (false, vec![0, 1, 3]),
    (false, vec![0, 2, 3]),
    (false, vec![1, 2, 3]),
    (true, vec![0, 2, 3]),
    (true, vec![0, 1, 3]),
    (true, vec![0, 1, 2]),
  ] {
    reader.history(forward);
    assert_eq!(reader.jumps().map(|p| p.page.0).collect::<Vec<_>>(), expected);
  }
  reader.clear_history();
  assert_eq!(reader.jumps().count(), 0);
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn jumps_panel_empty_and_recorded_destinations() -> Result<()> {
  let screen = crate::common::cells(120, 50);
  let mut app = solo("book.pdf", false, screen)?;
  keys(&mut app, b":jumps\r")?;
  let panel = app.panel.as_ref().unwrap();
  assert_eq!(panel.view.kind(), PanelKind::Jumps);
  assert!(panel.view.rows().is_empty());
  let origin = app.reader.cursor;
  keys(&mut app, b"\r")?;
  assert_eq!(app.reader.cursor, origin);
  app.close_panel()?;

  let mut positions = vec![origin];
  for page in [9, 19] {
    app.go_to(Position { page: PageId(page), line: 2, glyph: 3 }, true)?;
    positions.push(app.reader.cursor);
  }
  // Newest first: every stop, keyed by its distance from the cursor.
  for (forward, expected) in [
    (false, ["+1", "now", "−1"]),
    (false, ["+2", "+1", "now"]),
    (true, ["+1", "now", "−1"]),
  ] {
    keys(&mut app, if forward { b"\x1b[105;5u" } else { b"\x0f" })?;
    keys(&mut app, b":jumps\r")?;
    {
      let (app, _) = app.parts();
      let rows = &app.panel.as_ref().unwrap().view.rows();
      let keys: Vec<_> = rows.iter().map(|row| row.key.as_str()).collect();
      assert_eq!(keys, expected);
      for (row, index) in rows.iter().zip([2, 1, 0]) {
        let Destination::Jump { position, .. } = row.destination else {
          panic!("Jump destination");
        };
        assert_eq!(position, positions[index]);
        assert!(row.detail.contains(&format!(
          "L{} C{}",
          position.line + 1,
          position.glyph + 1
        )));
        let text = app.doc.text(position.page)?;
        let line = text.lines.get(position.line).unwrap();
        assert_eq!(
          row.label,
          line.text().split_whitespace().collect::<Vec<_>>().join(" ")
        );
      }
    }
    app.close_panel()?;
  }
  keys(&mut app, b":jumps\r")?;
  let origin = app.reader.cursor;
  app.panel.as_mut().unwrap().view.choose(0);
  // History changes must preserve recorded destinations.
  app.reader.clear_history();
  keys(&mut app, b"\r")?;
  assert_eq!(app.reader.cursor, positions[2]);
  assert!(app.panel.is_some() && !app.panel_focused());
  keys(&mut app, b"\x0f")?;
  assert_eq!(app.reader.cursor, origin);
  Ok(())
}

fn text_app(lines: &[&str]) -> Result<(tempfile::TempDir, Solo)> {
  let (dir, mut app) = crate::common::temp_solo(
    |path| super::common::text_pdf(path, lines),
    true,
    crate::common::cells(120, 50),
  )?;
  super::common::outline_loaded(&mut app)?;
  app.doc.outline.clear();
  Ok((dir, app))
}

/// Stops behind the cursor, oldest first.
fn past(app: &mut App) -> Result<Vec<vpdf_ui::panel::Row>> {
  let mut rows = app.panel_rows(PanelKind::Jumps)?;
  rows.retain(|row| row.key.starts_with('−'));
  rows.reverse();
  Ok(rows)
}

fn at(line: usize, glyph: usize) -> Position {
  Position { page: PageId(0), line, glyph }
}

#[test]
fn fallback_context() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "na\\357ve words,"])?;
  app.doc.pages[0].label = "iv".to_owned().into();
  for glyph in [0, 5, 11] {
    app.go_to(at(1, glyph), true)?;
    app.reader.clear_history();
    app.go_to(at(0, 0), true)?;
    let rows = past(&mut app)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "naïve words,");
    assert_eq!(rows[0].detail, format!("piv L2 C{}", glyph + 1));
  }
  let (_dir, mut app) = text_app(&[""])?;
  app.reader.seed_reader(Reader::remember);
  let rows = past(&mut app)?;
  assert_eq!(rows[0].label, "No text");
  assert_eq!(rows[0].detail, "p1 L1 C1");
  Ok(())
}

#[test]
fn semantic_history() -> Result<()> {
  let (_dir, mut app) = text_app(&[
    "Opening",
    "First section",
    "Second section",
    "needle target",
    "End",
  ])?;
  heading_on(&mut app, at(1, 0), "First section")?;
  heading_on(&mut app, at(2, 0), "Second section")?;
  app.open_panel(PanelKind::Outline)?;
  keys(&mut app, b"\r")?;
  app.close_panel()?;
  keys(&mut app, b"]]")?;
  app.start_search("needle", false, false)?;
  super::common::search_loaded(&mut app)?;
  app.go_to(at(4, 0), true)?;
  let labels = |app: &mut App| -> Result<Vec<String>> {
    Ok(past(app)?.into_iter().map(|row| row.label).collect())
  };
  let expected =
    vec!["Opening", "First section", "Second section", "/needle/ 1/1"];
  assert_eq!(labels(&mut app)?, expected);
  // Later searches cannot relabel earlier visits.
  let other =
    vpdf_search::query::Query { text: "other".into(), ..Default::default() };
  app.search.show(other, Vec::new());
  keys(&mut app, b"\x0f\x1b[105;5u")?;
  assert_eq!(labels(&mut app)?, expected);
  app.open_panel(PanelKind::Jumps)?;
  // Newest first: `now`, then the search.
  app.panel.as_mut().unwrap().view.choose(1);
  app.reader.clear_history();
  keys(&mut app, b"\r")?;
  assert_eq!(app.reader.cursor, at(3, 0));
  app.close_panel()?;
  app.go_to(at(4, 0), true)?;
  assert_eq!(labels(&mut app)?.last().unwrap(), "/needle/ 1/1");
  Ok(())
}

#[test]
fn committed_searches() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "needle one", "needle two"])?;
  app.begin_search(false)?;
  app.incremental("needle")?;
  super::common::search_loaded(&mut app)?;
  assert_eq!(app.reader.jumps().count(), 0);
  app.finish_search("needle", false)?;
  app.overlays.close_prompt();
  app.next_result(false, 1)?;
  app.go_to(at(0, 0), true)?;
  let rows = past(&mut app)?;
  let labels: Vec<_> = rows
    .iter()
    .filter(|row| row.kind == vpdf_ui::panel::RowKind::Search)
    .map(|row| row.label.as_str())
    .collect();
  assert_eq!(labels, ["/needle/ 1/2", "/needle/ 2/2"]);
  app.open_panel(PanelKind::Results)?;
  app.panel.as_mut().unwrap().view.choose(2);
  keys(&mut app, b"\r")?;
  app.close_panel()?;
  app.go_to(at(0, 0), true)?;
  let rows = past(&mut app)?;
  assert_eq!(rows.last().unwrap().label, "/needle/ 2/2");
  app.start_search("needle", true, false)?;
  super::common::search_loaded(&mut app)?;
  app.go_to(at(0, 0), true)?;
  let rows = past(&mut app)?;
  assert_eq!(rows.last().unwrap().label, "?needle? 2/2");
  Ok(())
}

#[test]
fn followed_links() -> Result<()> {
  use vpdf_model::document::Target;
  let (_dir, mut app) = text_app(&["Opening", "Reference title", "End"])?;
  let bounds = app.doc.text(PageId(0))?.lines[1].glyphs[0].bounds;
  let (x, y) = bounds.centre();
  app.follow(&Target::Point { page: PageId(0), x, y })?;
  app.go_to(at(2, 0), true)?;
  let rows = past(&mut app)?;
  assert!(rows[1].label.starts_with("Reference title"));
  assert_eq!(rows[1].kind, vpdf_ui::panel::RowKind::Link);
  assert!(matches!(rows[1].destination,
    Destination::Jump { position, .. } if position == at(1, 0)));
  Ok(())
}

#[test]
fn outline_preserves_selected_title() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "Shared destination", "End"])?;
  heading_on(&mut app, at(1, 0), "Chapter")?;
  heading_on(&mut app, at(1, 0), "Section")?;
  app.open_panel(PanelKind::Outline)?;
  app.panel.as_mut().unwrap().view.choose(1);
  keys(&mut app, b"\r")?;
  app.close_panel()?;
  app.go_to(at(2, 0), true)?;
  let rows = past(&mut app)?;
  assert_eq!(rows.last().unwrap().label, "Section");
  Ok(())
}

const PAGE: [&str; 7] = [
  "L0 alpha beta gamma",
  "L1 alpha beta gamma",
  "L2 alpha beta gamma",
  "L3 alpha beta gamma",
  "L4 alpha beta gamma",
  "L5 alpha beta gamma",
  "L6 alpha beta gamma",
];

fn landing_app(text_mode: bool) -> Result<(tempfile::TempDir, Solo)> {
  // Text keeps four rows under its header.
  let (columns, rows) = if text_mode { (10, 6) } else { (40, 6) };
  let (dir, mut app) = crate::common::temp_solo(
    |path| super::common::pages_pdf(path, &[&PAGE, &PAGE]),
    text_mode,
    crate::common::cells(columns, rows),
  )?;
  super::common::outline_loaded(&mut app)?;
  app.doc.outline.clear();
  Ok((dir, app))
}

fn on(page: usize, line: usize, glyph: usize) -> Position {
  Position { page: PageId(page), line, glyph }
}

/// Scroll and every line's text view row.
type View = (f32, Vec<Option<usize>>);

fn view(app: &mut App) -> Result<View> {
  let mut rows = Vec::new();
  for page in 0..app.doc.pages.len() {
    let lines = app.doc.text(PageId(page))?.lines.len();
    for line in 0..lines {
      rows.push(app.text().row(on(page, line, 0))?);
    }
  }
  Ok((app.reader.scroll, rows))
}

/// Cursor glyph's top and bottom in rows.
fn cursor_rows(app: &mut App) -> Result<(f32, f32)> {
  let cursor = app.reader.cursor;
  if app.text_mode {
    // View rows, below the header.
    let row = app.text().row(cursor)?.expect("cursor shows") as f32;
    return Ok((row, row + 1.0));
  }
  let text = app.doc.text(cursor.page)?;
  let glyph = text.glyph(cursor).expect("cursor glyph").bounds;
  let rect = app.layout.project(cursor.page, glyph, app.reader.scroll);
  let row = f32::from(app.screen.cell_height);
  Ok((rect.y0 / row, rect.y1 / row))
}

#[derive(Clone, Copy, Debug)]
enum Jump {
  Mark,
  /// `n` to the next match of a query.
  Next(&'static str),
  /// Typing a query previews its first match.
  Preview(&'static str),
  Link,
  Outline,
  Result,
  Page,
}

fn heading_on(app: &mut App, at: Position, title: &str) -> Result<Target> {
  let bounds = app.doc.text(at.page)?.lines[at.line].glyphs[0].bounds;
  let (x, y) = bounds.centre();
  app.doc.outline.push(Heading {
    title: title.into(),
    depth: 0,
    target: Target::Point { page: at.page, x, y },
  });
  Ok(Target::Point { page: at.page, x, y })
}

fn jump(app: &mut App, kind: Jump, at: Position) -> Result<()> {
  match kind {
    Jump::Mark => app.go_to(at, true),
    Jump::Next(_) => app.next_result(false, 1),
    Jump::Preview(query) => {
      app.begin_search(false)?;
      app.start_search(query, false, false)?;
      super::common::search_loaded(app)
    }
    Jump::Page => app.follow(&Target::Page(at.page)),
    Jump::Link => {
      let text = app.doc.text(at.page)?;
      let bounds = text.glyph(at).expect("target glyph").bounds;
      let (x, y) = bounds.centre();
      app.follow(&Target::Point { page: at.page, x, y })
    }
    Jump::Result => {
      app.open_panel(PanelKind::Results)?;
      let view = &mut app.panel.as_mut().unwrap().view;
      let index = view.rows().iter().position(|row| {
        matches!(row.destination, Destination::Position(hit) if hit == at)
      });
      view.choose(index.expect("hit at target"));
      keys(app, b"\r")?;
      Ok(())
    }
    Jump::Outline => {
      heading_on(app, at, "Heading")?;
      app.open_panel(PanelKind::Outline)?;
      keys(app, b"\r")?;
      // Closing would reflow the landed view.
      Ok(())
    }
  }
}

/// Where a jump leaves its target.
#[derive(Clone, Copy, Debug)]
enum Lands {
  /// Below one row of context.
  Top,
  /// Where it already showed.
  Stays,
  /// The page's top edge on row 0.
  PageTop,
}

/// U1: links and rows land below context.
#[test]
fn jumps_land_near_the_top() -> Result<()> {
  for (text_mode, kind, start, at, lands) in [
    (false, Jump::Link, 0.0, on(1, 4, 0), Lands::Top),
    (false, Jump::Link, 0.0, on(0, 4, 0), Lands::Top),
    (false, Jump::Outline, 0.0, on(1, 4, 0), Lands::Top),
    (false, Jump::Result, 0.0, on(1, 4, 0), Lands::Top),
    (true, Jump::Result, 0.0, on(0, 4, 0), Lands::Top),
    (false, Jump::Link, 40.0, on(0, 2, 0), Lands::Stays),
    // Position targets stay when shown, else land.
    (false, Jump::Mark, 0.0, on(0, 4, 0), Lands::Top),
    (false, Jump::Mark, 0.0, on(0, 1, 3), Lands::Stays),
    (true, Jump::Mark, 0.0, on(0, 1, 0), Lands::Stays),
    (true, Jump::Mark, 0.0, on(1, 4, 0), Lands::Top),
    (false, Jump::Next("L4 alpha"), 0.0, on(0, 4, 0), Lands::Top),
    (true, Jump::Next("L4 alpha"), 0.0, on(1, 4, 0), Lands::Top),
    (false, Jump::Next("alpha"), 0.0, on(0, 1, 3), Lands::Stays),
    (true, Jump::Next("beta"), 0.0, on(0, 1, 9), Lands::Stays),
    (false, Jump::Preview("L4 alpha"), 0.0, on(0, 4, 0), Lands::Top),
    (true, Jump::Preview("L5 alpha"), 0.0, on(0, 5, 0), Lands::Top),
    (false, Jump::Preview("L2 alpha"), 40.0, on(0, 2, 0), Lands::Stays),
    (true, Jump::Preview("L2 alpha"), 0.0, on(0, 2, 0), Lands::Stays),
    (false, Jump::Page, 60.0, on(1, 0, 0), Lands::PageTop),
    (true, Jump::Link, 0.0, on(1, 4, 0), Lands::Top),
    (true, Jump::Outline, 0.0, on(0, 4, 0), Lands::Top),
    (true, Jump::Link, 0.0, on(0, 2, 0), Lands::Stays),
    (true, Jump::Link, 0.0, on(0, 2, 15), Lands::Stays),
    // A page's last screenful lands too.
    (true, Jump::Link, 0.0, on(0, 6, 0), Lands::Top),
    (true, Jump::Outline, 0.0, on(0, 5, 0), Lands::Top),
    (true, Jump::Link, 0.0, on(1, 0, 0), Lands::Top),
  ] {
    let case = format!("{kind:?} text {text_mode} from {start} to {at:?}");
    let (_dir, mut app) = landing_app(text_mode)?;
    let query = match kind {
      Jump::Result => Some("L4 alpha"),
      Jump::Next(query) => Some(query),
      _ => None,
    };
    if let Some(query) = query {
      app.start_search(query, false, false)?;
      super::common::search_loaded(&mut app)?;
      app.reader.clear_history();
    }
    app.reader.seed_reader(|reader| reader.scroll = start);
    let before = view(&mut app)?;
    jump(&mut app, kind, at)?;
    assert_eq!(app.reader.cursor, at, "{case}");
    let (top, _) = cursor_rows(&mut app)?;
    match lands {
      Lands::Top => assert!((top - 1.0).abs() < 0.01, "{case}: {top}"),
      Lands::Stays => {
        assert_eq!(app.reader.scroll, before.0, "{case}");
        if text_mode {
          let row = before.1[at.line].map(|row| row as f32);
          assert_eq!(Some(top), row, "{case}");
        }
      }
      Lands::PageTop => {
        let edge = app.layout.pages()[at.page.0].top - app.reader.scroll;
        let row = f32::from(app.screen.cell_height);
        assert!((0.0..row).contains(&edge), "{case}: {edge}");
      }
    }
    // Previews record no jump and tint nothing.
    if matches!(kind, Jump::Preview(_)) {
      continue;
    }
    let landing = app.reader.landing().position();
    assert_eq!(landing, Some(at), "{case}");
    if app.panel.is_some() {
      app.close_panel()?;
    }
    // `''` restores the view.
    keys(&mut app, b"''")?;
    assert_eq!(view(&mut app)?, before, "{case}");
  }
  Ok(())
}

/// Each typed query previews from the view the search started at.
#[test]
fn chained_previews_judge_the_start_view() -> Result<()> {
  for text_mode in [false, true] {
    let (_dir, mut app) = landing_app(text_mode)?;
    let before = view(&mut app)?;
    app.begin_search(false)?;
    for query in ["L5 alpha", "L1 alpha"] {
      app.start_search(query, false, false)?;
      super::common::search_loaded(&mut app)?;
    }
    assert_eq!(app.reader.cursor, on(0, 1, 0), "text {text_mode}");
    assert_eq!(view(&mut app)?, before, "text {text_mode}");
  }
  Ok(())
}

/// History from the other presentation lands its position against the
/// current view; its stale view never returns.
#[test]
fn history_across_presentations_lands() -> Result<()> {
  for text_first in [false, true] {
    let (_dir, mut app) = landing_app(text_first)?;
    app.go_to(on(0, 5, 0), true)?;
    app.reader.clear_history();
    app.go_to(on(1, 4, 0), true)?;
    app.text_mode = !text_first;
    app.relayout()?;
    // Away from the origin: page 2 in PDF; text keeps the last landing.
    {
      let seed_value = app.layout.pages()[1].top;
      app.reader.seed_reader(|reader| reader.scroll = seed_value);
    };
    keys(&mut app, b"\x0f")?;
    assert_eq!(app.reader.cursor, on(0, 5, 0), "text first {text_first}");
    let (top, _) = cursor_rows(&mut app)?;
    assert!((top - 1.0).abs() < 0.01, "text first {text_first}: {top}");
  }
  Ok(())
}

/// U1: a split lands its target as a follow does.
#[test]
fn splits_land_near_the_top() -> Result<()> {
  for text_mode in [false, true] {
    for point in [false, true] {
      let case = format!("text {text_mode} point {point}");
      let (_dir, path) = crate::common::temp_pdf(|path| {
        super::common::pages_pdf(path, &[&PAGE, &PAGE])
      })?;
      let rows = if text_mode { 5 } else { 6 };
      let screen = crate::common::cells(80, rows);
      let mut session = crate::harness::session(&path, screen)?;
      let app = session.app();
      super::common::outline_loaded(app)?;
      if text_mode {
        // Both panes keep the visible presentation.
        app.text_mode = true;
        app.relayout()?;
      }
      let at = on(1, 4, 0);
      let bounds = app.doc.text(at.page)?.lines[at.line].glyphs[0].bounds;
      let (x, y) = bounds.centre();
      let target = Target::Point { page: at.page, x, y };
      app.request = Some(if point {
        vpdf::app::Request::SplitTo(target)
      } else {
        vpdf::app::Request::SplitAt(at)
      });
      session.settle()?;
      let tab = session.tab();
      assert_eq!((tab.panes.len(), tab.focus), (2, 0), "{case}");
      let app = &mut tab.panes[1];
      assert_eq!(app.text_mode, text_mode, "{case}");
      assert_eq!(app.reader.cursor, at, "{case}");
      let (top, _) = cursor_rows(app)?;
      assert!((top - 1.0).abs() < 0.01, "{case}: {top}");
    }
  }
  Ok(())
}

/// What happens between the landing and `]]`.
#[derive(Clone, Copy, Debug)]
enum Then {
  Nothing,
  /// Lands from the outline, then closes it.
  ClosePanel,
  Zoom,
}

/// `]]` moves past a landed heading.
#[test]
fn sections_continue_after_landing() -> Result<()> {
  for (text_mode, zoom, first, then) in [
    (false, 1.0, on(0, 4, 0), Then::Nothing),
    (false, 2.0, on(0, 4, 0), Then::Nothing),
    (false, 1.0, on(0, 1, 0), Then::Nothing),
    (false, 1.0, on(0, 4, 0), Then::ClosePanel),
    (false, 1.0, on(0, 4, 0), Then::Zoom),
    (true, 1.0, on(0, 4, 0), Then::Nothing),
    (true, 1.0, on(0, 1, 0), Then::Nothing),
    (true, 1.0, on(0, 4, 0), Then::ClosePanel),
  ] {
    let case = format!("text {text_mode} zoom {zoom} {first:?} {then:?}");
    let (_dir, mut app) = landing_app(text_mode)?;
    app.layout.request_zoom(vpdf_ui::layout::Zoom::To(zoom));
    app.relayout()?;
    let target = heading_on(&mut app, first, "First")?;
    heading_on(&mut app, on(1, 2, 0), "Second")?;
    if matches!(then, Then::ClosePanel) {
      app.open_panel(PanelKind::Outline)?;
      keys(&mut app, b"\r")?;
      app.close_panel()?;
    } else {
      app.follow(&target)?;
    }
    if matches!(then, Then::Zoom) {
      keys(&mut app, b"+")?;
    }
    assert_eq!(app.reader.cursor, first, "{case}");
    keys(&mut app, b"]]")?;
    assert_eq!(app.reader.cursor, on(1, 2, 0), "{case}");
  }
  Ok(())
}

/// A rowless text view lands safely.
#[test]
fn rowless_text_landing() -> Result<()> {
  let (_dir, mut app) = landing_app(true)?;
  app.screen.rows = 1;
  jump(&mut app, Jump::Link, on(1, 4, 0))?;
  assert_eq!(app.reader.cursor, on(1, 4, 0));
  Ok(())
}

#[test]
fn counted_history_keeps_both_directions() -> Result<()> {
  let (_directory, mut app) = crate::common::temp_solo(
    |path| {
      crate::common::pages_pdf(
        path,
        &[&["one"], &["two"], &["three"], &["four"]],
      )
    },
    false,
    crate::focus::screen(),
  )?;
  for page in ["2", "3", "4"] {
    app.command(page)?;
  }
  for (sequence, page) in [
    (b"3\x0f".as_slice(), 0),
    (b"2\x1b[105;5u", 2),
    (b"99\x1b[105;5u", 3),
    (b"99\x0f", 0),
  ] {
    app.reader.clear_flash();
    keys(&mut app, sequence)?;
    assert_eq!(app.reader.cursor.page, PageId(page));
    assert_eq!(app.reader.landing().position().unwrap(), app.reader.cursor);
  }
  Ok(())
}

#[test]
fn history_clears_column_goal() -> Result<()> {
  let lines = &["abcdefghijklmnopqrstuvwxyz"; 3];
  let (_directory, mut app) = crate::common::temp_solo(
    |path| crate::common::pages_pdf(path, &[lines, lines]),
    true,
    crate::focus::screen(),
  )?;
  keys(&mut app, b"i20lj:2\r0j\x0fj")?;
  assert_eq!(
    app.reader.cursor,
    Position { page: PageId(0), line: 2, glyph: 20 }
  );
  keys(&mut app, b"\x1b[105;5uj")?;
  assert_eq!(
    app.reader.cursor,
    Position { page: PageId(1), line: 2, glyph: 0 }
  );
  Ok(())
}

#[test]
fn results_mark_the_landed_hit() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "needle one", "needle two"])?;
  app.start_search("needle", false, false)?;
  super::common::search_loaded(&mut app)?;
  app.open_panel(PanelKind::Results)?;
  let panel = &app.panel.as_ref().unwrap().view;
  let depths: Vec<_> = panel.rows().iter().map(|row| row.depth).collect();
  assert_eq!(depths, [0, 1, 1]);
  assert_eq!(panel.current(), Some(1));
  app.next_result(false, 1)?;
  assert_eq!(app.panel.as_ref().unwrap().view.current(), Some(2));
  app.panel.as_mut().unwrap().view.choose(1);
  keys(&mut app, b"\r")?;
  assert_eq!(app.panel.as_ref().unwrap().view.current(), Some(1));
  Ok(())
}

#[test]
fn typed_searches_mark_the_landed_hit() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "needle one", "needle two"])?;
  app.open_panel(PanelKind::Results)?;
  keys(&mut app, b"\t/needle\r")?;
  super::common::search_loaded(&mut app)?;
  let panel = &app.panel.as_ref().unwrap().view;
  assert_eq!(panel.current(), Some(1));
  assert_eq!(panel.selected(), 1);
  Ok(())
}

#[test]
fn stops_count_from_the_cursor() {
  type Stops = &'static [(isize, usize)];
  let mut reader = Reader::default();
  assert_eq!(reader.stops().count(), 0);
  for page in 0..3 {
    reader.cursor.page = PageId(page);
    reader.remember();
  }
  reader.cursor.page = PageId(3);
  let stops = |reader: &Reader| -> Vec<(isize, usize)> {
    reader.stops().map(|(offset, at, _)| (offset, at.page.0)).collect()
  };
  assert_eq!(stops(&reader), [(0, 3), (-1, 2), (-2, 1), (-3, 0)]);
  // (forward, stops)
  let moves: [(bool, Stops); 3] = [
    (false, &[(1, 3), (0, 2), (-1, 1), (-2, 0)]),
    (false, &[(2, 3), (1, 2), (0, 1), (-1, 0)]),
    (true, &[(1, 3), (0, 2), (-1, 1), (-2, 0)]),
  ];
  for (forward, want) in moves {
    reader.history(forward);
    assert_eq!(stops(&reader), want, "{forward}");
  }
}

#[test]
fn jumps_panel_keys_and_marks_now() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "Middle", "End"])?;
  app.go_to(at(1, 0), true)?;
  app.go_to(at(2, 0), true)?;
  app.open_panel(PanelKind::Jumps)?;
  let state = |app: &mut Solo| -> Result<(Vec<String>, Option<usize>)> {
    let (app, graphics) = app.parts();
    app.draw_alone(graphics, &mut Vec::new())?;
    let panel = &app.panel.as_ref().unwrap().view;
    let keys = panel.rows().iter().map(|row| row.key.clone()).collect();
    Ok((keys, panel.current()))
  };
  assert_eq!(
    state(&mut app)?,
    (vec!["now".into(), "−1".into(), "−2".into()], Some(0))
  );
  // `C-o` moves back; the panel follows without a refill request.
  keys(&mut app, b"\t\x0f")?;
  assert_eq!(
    state(&mut app)?,
    (vec!["+1".into(), "now".into(), "−1".into()], Some(1))
  );
  // Following `now` records nothing.
  app.panel.as_mut().unwrap().view.choose(1);
  keys(&mut app, b"\t\r")?;
  assert_eq!(state(&mut app)?.0, ["+1", "now", "−1"]);
  Ok(())
}
