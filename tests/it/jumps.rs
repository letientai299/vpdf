use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::{Heading, Target};
use vpdf::keys::PanelKind;
use vpdf::layout::Screen;
use vpdf::panel::Destination;
use vpdf::prompt::History;
use vpdf::reader::Reader;
use vpdf::text::{PageId, Position};

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

fn command(app: &mut App, history: &mut History, text: &str) -> Result<()> {
  for code in std::iter::once(KeyCode::Char(':'))
    .chain(text.chars().map(KeyCode::Char))
    .chain(std::iter::once(KeyCode::Enter))
  {
    app.key(KeyEvent::new(code, KeyModifiers::NONE), history)?;
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn jumps_panel_empty_and_recorded_destinations() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let mut history = History::default();
  command(&mut app, &mut history, "jumps")?;
  let panel = app.panel.as_ref().unwrap();
  assert_eq!(panel.kind, PanelKind::Jumps);
  assert!(panel.rows.is_empty());
  let origin = app.reader.cursor;
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  assert_eq!(app.reader.cursor, origin);
  app.close_panel()?;

  let mut positions = vec![origin];
  for page in [9, 19] {
    app.go_to(Position { page: PageId(page), line: 2, glyph: 3 }, true)?;
    positions.push(app.reader.cursor);
  }
  for (forward, expected) in [(false, [0, 2]), (false, [1, 2]), (true, [0, 2])]
  {
    let code = KeyCode::Char(if forward { 'i' } else { 'o' });
    app.key(KeyEvent::new(code, KeyModifiers::CONTROL), &mut history)?;
    command(&mut app, &mut history, "jumps")?;
    let rows = &app.panel.as_ref().unwrap().rows;
    assert_eq!(rows.len(), expected.len());
    for (row, index) in rows.iter().zip(expected) {
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
    app.close_panel()?;
  }
  command(&mut app, &mut history, "jumps")?;
  let origin = app.reader.cursor;
  app.panel.as_mut().unwrap().selected = 1;
  // History changes must preserve recorded destinations.
  app.reader.clear_history();
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  assert_eq!(app.reader.cursor, positions[2]);
  assert!(app.panel.is_some() && !app.panel_focused());
  app.key(
    KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
    &mut history,
  )?;
  assert_eq!(app.reader.cursor, origin);
  Ok(())
}

fn text_app(lines: &[&str]) -> Result<(tempfile::TempDir, App)> {
  let dir = tempfile::tempdir()?;
  let path = dir.path().join("jumps.pdf");
  super::common::text_pdf(&path, lines)?;
  let mut app = App::with_screen(
    &path,
    "",
    true,
    Screen {
      columns: 120,
      rows: 50,
      cell_width: 8,
      cell_height: 16,
      origin: (0, 0),
    },
  )?;
  super::common::outline_loaded(&mut app)?;
  app.doc.outline.clear();
  Ok((dir, app))
}

fn at(line: usize, glyph: usize) -> Position {
  Position { page: PageId(0), line, glyph }
}

fn history_key(app: &mut App, forward: bool) -> Result<()> {
  app.key(
    KeyEvent::new(
      KeyCode::Char(if forward { 'i' } else { 'o' }),
      KeyModifiers::CONTROL,
    ),
    &mut History::default(),
  )?;
  Ok(())
}

#[test]
fn fallback_context() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "na\\357ve words,"])?;
  app.doc.pages[0].label = "iv".to_owned().into();
  for (glyph, word) in
    [(0, "“naïve”"), (5, "whitespace"), (6, "“words”"), (11, "“,”")]
  {
    app.go_to(at(1, glyph), true)?;
    app.reader.clear_history();
    app.go_to(at(0, 0), true)?;
    let rows = app.panel_rows(PanelKind::Jumps)?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].label, "naïve words,");
    assert_eq!(
      rows[0].detail,
      format!("p. iv (#1) · L2 C{} · word {word}", glyph + 1)
    );
  }
  let (_dir, mut app) = text_app(&[""])?;
  app.reader.remember();
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows[0].label, "No text");
  assert_eq!(rows[0].detail, "p. 1 · L1 C1 · word —");
  Ok(())
}

fn heading(app: &mut App, line: usize, title: &str) -> Result<()> {
  use vpdf::document::{Heading, Target};
  let bounds = app.doc.text(PageId(0))?.lines[line].glyphs[0].bounds;
  let (x, y) = bounds.centre();
  app.doc.outline.push(Heading {
    title: title.into(),
    depth: 0,
    target: Target::Point { page: PageId(0), x, y },
  });
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
  heading(&mut app, 1, "First section")?;
  heading(&mut app, 2, "Second section")?;
  let mut history = History::default();
  app.open_panel(PanelKind::Outline)?;
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  app.close_panel()?;
  for _ in 0..2 {
    app.key(
      KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
      &mut history,
    )?;
  }
  app.start_search("needle", false, false)?;
  super::common::search_loaded(&mut app)?;
  app.go_to(at(4, 0), true)?;
  let labels = |app: &mut App| -> Result<Vec<String>> {
    Ok(
      app
        .panel_rows(PanelKind::Jumps)?
        .into_iter()
        .map(|row| row.label)
        .collect(),
    )
  };
  let expected = vec![
    "Opening",
    "Heading: First section",
    "Heading: Second section",
    "Search [1/1] /needle/",
  ];
  assert_eq!(labels(&mut app)?, expected);
  // Later searches cannot relabel earlier visits.
  app.search.query.text = "other".into();
  history_key(&mut app, false)?;
  history_key(&mut app, true)?;
  assert_eq!(labels(&mut app)?, expected);
  app.open_panel(PanelKind::Jumps)?;
  app.panel.as_mut().unwrap().selected = 3;
  app.reader.clear_history();
  app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
  assert_eq!(app.reader.cursor, at(3, 0));
  app.close_panel()?;
  app.go_to(at(4, 0), true)?;
  assert_eq!(labels(&mut app)?.last().unwrap(), "Search [1/1] /needle/");
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
  app.prompt = None;
  app.next_result(false, 1)?;
  app.go_to(at(0, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  let labels: Vec<_> = rows
    .iter()
    .filter(|row| row.label.starts_with("Search ["))
    .map(|row| row.label.as_str())
    .collect();
  assert_eq!(labels, ["Search [1/2] /needle/", "Search [2/2] /needle/"]);
  app.open_panel(PanelKind::Results)?;
  app.panel.as_mut().unwrap().selected = 1;
  app.key(
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    &mut History::default(),
  )?;
  app.close_panel()?;
  app.go_to(at(0, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows.last().unwrap().label, "Search [2/2] /needle/");
  app.start_search("needle", true, false)?;
  super::common::search_loaded(&mut app)?;
  app.go_to(at(0, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows.last().unwrap().label, "Search [2/2] ?needle?");
  Ok(())
}

#[test]
fn followed_links() -> Result<()> {
  use vpdf::document::Target;
  let (_dir, mut app) = text_app(&["Opening", "Reference title", "End"])?;
  let bounds = app.doc.text(PageId(0))?.lines[1].glyphs[0].bounds;
  let (x, y) = bounds.centre();
  app.follow(&Target::Point { page: PageId(0), x, y })?;
  app.go_to(at(2, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert!(rows[1].label.starts_with("Link: Reference title"));
  assert!(matches!(rows[1].destination,
    Destination::Jump { position, .. } if position == at(1, 0)));
  Ok(())
}

#[test]
fn outline_preserves_selected_title() -> Result<()> {
  let (_dir, mut app) = text_app(&["Opening", "Shared destination", "End"])?;
  heading(&mut app, 1, "Chapter")?;
  heading(&mut app, 1, "Section")?;
  app.open_panel(PanelKind::Outline)?;
  app.panel.as_mut().unwrap().selected = 1;
  app.key(
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
    &mut History::default(),
  )?;
  app.close_panel()?;
  app.go_to(at(2, 0), true)?;
  let rows = app.panel_rows(PanelKind::Jumps)?;
  assert_eq!(rows.last().unwrap().label, "Heading: Section");
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

fn landing_app(text_mode: bool) -> Result<(tempfile::TempDir, App)> {
  let dir = tempfile::tempdir()?;
  let path = dir.path().join("landing.pdf");
  super::common::pages_pdf(&path, &[&PAGE, &PAGE])?;
  let (columns, rows) = if text_mode { (10, 5) } else { (40, 6) };
  let screen =
    Screen { columns, rows, cell_width: 8, cell_height: 16, origin: (0, 0) };
  let mut app = App::with_screen(&path, "", text_mode, screen)?;
  super::common::outline_loaded(&mut app)?;
  app.doc.outline.clear();
  Ok((dir, app))
}

fn on(page: usize, line: usize, glyph: usize) -> Position {
  Position { page: PageId(page), line, glyph }
}

/// Scroll and every line start's cell.
type View = (f32, Vec<Option<(u16, u16)>>);

fn view(app: &mut App) -> Result<View> {
  let mut cells = Vec::new();
  for page in 0..app.doc.pages.len() {
    let lines = app.doc.text(PageId(page))?.lines.len();
    for line in 0..lines {
      cells.push(app.text_cell(on(page, line, 0))?);
    }
  }
  Ok((app.reader.scroll, cells))
}

/// Cursor glyph's top and bottom in rows.
fn cursor_rows(app: &mut App) -> Result<(f32, f32)> {
  let cursor = app.reader.cursor;
  if app.text_mode {
    let cell = app.text_cell(cursor)?.expect("cursor shows");
    let row = f32::from(cell.1);
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
      let index = app.search.hits.iter().position(|hit| hit.first == at);
      app.open_panel(PanelKind::Results)?;
      app.panel.as_mut().unwrap().selected = index.expect("hit at target");
      app.key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut History::default(),
      )?;
      Ok(())
    }
    Jump::Outline => {
      heading_on(app, at, "Heading")?;
      app.open_panel(PanelKind::Outline)?;
      app.key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut History::default(),
      )?;
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
    app.reader.scroll = start;
    let before = view(&mut app)?;
    jump(&mut app, kind, at)?;
    assert_eq!(app.reader.cursor, at, "{case}");
    let (top, _) = cursor_rows(&mut app)?;
    match lands {
      Lands::Top => assert!((top - 1.0).abs() < 0.01, "{case}: {top}"),
      Lands::Stays => {
        assert_eq!(app.reader.scroll, before.0, "{case}");
        if text_mode {
          let row = before.1[at.line].map(|(_, row)| f32::from(row));
          assert_eq!(Some(top), row, "{case}");
        }
      }
      Lands::PageTop => {
        let edge = app.layout.pages[at.page.0].top - app.reader.scroll;
        let row = f32::from(app.screen.cell_height);
        assert!((0.0..row).contains(&edge), "{case}: {edge}");
      }
    }
    // Previews record no jump and tint nothing.
    if matches!(kind, Jump::Preview(_)) {
      continue;
    }
    let landing = app.landing.as_ref().map(|landing| landing.position);
    assert_eq!(landing, Some(at), "{case}");
    if app.panel.is_some() {
      app.close_panel()?;
    }
    // `''` restores the view.
    for _ in 0..2 {
      app.key(
        KeyEvent::new(KeyCode::Char('\''), KeyModifiers::NONE),
        &mut History::default(),
      )?;
    }
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
    app.reader.scroll = app.layout.pages[1].top;
    let back = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    app.key(back, &mut History::default())?;
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
      let dir = tempfile::tempdir()?;
      let path = dir.path().join("landing.pdf");
      super::common::pages_pdf(&path, &[&PAGE, &PAGE])?;
      let rows = if text_mode { 5 } else { 6 };
      let screen = Screen {
        columns: 80,
        rows,
        cell_width: 8,
        cell_height: 16,
        origin: (0, 0),
      };
      let mut session = vpdf::session::Session::new(None);
      session.open(&path, true, screen)?;
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
    app.layout.zoom = zoom;
    app.relayout()?;
    let target = heading_on(&mut app, first, "First")?;
    heading_on(&mut app, on(1, 2, 0), "Second")?;
    let mut history = History::default();
    if matches!(then, Then::ClosePanel) {
      app.open_panel(PanelKind::Outline)?;
      app
        .key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), &mut history)?;
      app.close_panel()?;
    } else {
      app.follow(&target)?;
    }
    if matches!(then, Then::Zoom) {
      app.key(
        KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE),
        &mut history,
      )?;
    }
    assert_eq!(app.reader.cursor, first, "{case}");
    for _ in 0..2 {
      app.key(
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE),
        &mut history,
      )?;
    }
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
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("history.pdf");
  crate::common::pages_pdf(
    &path,
    &[&["one"], &["two"], &["three"], &["four"]],
  )?;
  let mut app = App::with_screen(&path, "", false, crate::focus::screen())?;
  for page in ["2", "3", "4"] {
    app.command(page)?;
  }
  for (sequence, page) in [
    (b"3\x0f".as_slice(), 0),
    (b"2\x1b[105;5u", 2),
    (b"99\x1b[105;5u", 3),
    (b"99\x0f", 0),
  ] {
    app.landing = None;
    crate::modes::keys(&mut app, sequence)?;
    assert_eq!(app.reader.cursor.page, PageId(page));
    assert_eq!(app.landing.as_ref().unwrap().position, app.reader.cursor);
  }
  Ok(())
}
