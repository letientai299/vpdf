use crate::common;

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::Target;
use vpdf::keys::{Action, Input, PanelKind};
use vpdf::layout::Screen;
use vpdf::pointer::{Region, Route};
use vpdf::text::PageId;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn reader_workflows() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  app.command("page 39")?;
  assert_eq!(app.reader.cursor.page, PageId(38));
  common::outline_loaded(&mut app)?;
  app.open_panel(PanelKind::Outline)?;
  assert!(!app.panel.as_ref().unwrap().rows.is_empty());
  app.close_panel()?;
  app.open_panel(PanelKind::Text)?;
  assert!(app.text_mode);
  app.action(Input {
    action: Action::Quit,
    count: 1,
    explicit: false,
    register: '"',
  })?;
  assert!(!app.text_mode && !app.quit);
  app.start_search("python", false, false)?;
  let start = Instant::now();
  while !matches!(app.search.phase, vpdf::find::Phase::Complete) {
    assert!(start.elapsed() < Duration::from_secs(15));
    app.search_updates()?;
    std::thread::sleep(Duration::from_millis(1));
  }
  assert!(!app.search.hits.is_empty());
  app.next_result(true, 2)?;
  app.open_panel(PanelKind::Results)?;
  assert_eq!(app.panel.as_ref().unwrap().rows.len(), app.search.hits.len());
  app.action(Input {
    action: Action::Cancel,
    count: 1,
    explicit: false,
    register: '"',
  })?;
  assert!(app.search.hidden);
  app.follow(&Target::Page(PageId(usize::MAX)))?;
  assert_eq!(app.reader.cursor.page.0, app.doc.pages.len() - 1);
  assert!(app.follow(&Target::Uri("file:///etc/passwd".into())).is_err());
  app.reader.visual(vpdf::reader::SelectionKind::Character);
  app.command("reload")?;
  assert!(app.reader.last_selection.is_none());
  app.raster_updates();
  assert_eq!(app.graphics.bytes(), 0);
  assert!(app.search.hits.is_empty());
  app.reader.history(false);
  assert!(app.reader.cursor.page.0 < app.doc.pages.len());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn scrolling_converges_and_jumps_interrupt() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  app.scroll(100.0)?;
  app.scroll(100.0)?;
  assert_eq!(app.animation.as_ref().unwrap().target, 200.0);
  assert_eq!(app.reader.scroll, 0.0);
  for _ in 0..30 {
    if let Some(animation) = &mut app.animation {
      animation.updated =
        Instant::now().checked_sub(Duration::from_millis(16)).unwrap();
    }
    let previous = app.reader.scroll;
    app.animate();
    assert!(app.reader.scroll >= previous && app.reader.scroll <= 200.0);
  }
  assert!(app.animation.is_none());
  assert_eq!(app.reader.scroll, 200.0);
  for action in [Action::Cancel, Action::History(false), Action::Align(0.0)] {
    app.scroll(100.0)?;
    app.action(Input { action, count: 1, explicit: false, register: '"' })?;
    assert!(app.animation.is_none());
    let stopped = app.reader.scroll;
    assert!(!app.animate());
    assert_eq!(app.reader.scroll, stopped);
  }
  app.scroll(100.0)?;
  app.command("page 39")?;
  assert!(app.animation.is_none());
  assert_eq!(app.reader.cursor.page, PageId(38));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn links_keep_coordinates_and_text_scrolls_lines() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let target = (0..app.doc.pages.len())
    .find_map(|page| {
      app.doc.links(PageId(page)).ok()?.into_iter().find(
        |link| matches!(link.target, Target::Point { y, .. } if y > 100.0),
      )
    })
    .expect("real book has positioned internal links")
    .target;
  let Target::Point { page, x, y } = target else { unreachable!() };
  let expected = app.doc.text(page)?.nearest(page, x, y);
  app.follow(&target)?;
  assert_eq!(app.reader.cursor, expected);
  app.command("page 39")?;
  app.text_mode = true;
  app.scroll(48.0)?;
  assert_eq!(app.reader.cursor.line, 3);
  assert!(app.animation.is_none());
  let position = app.text_position(2, 1)?;
  assert_eq!(position.line, 1);
  assert_eq!(position.glyph, 2);
  Ok(())
}

/// `paper.pdf` in text mode, 20 rows tall.
fn text_app() -> Result<App> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 20,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  App::with_screen(&root.join("paper.pdf"), "", true, screen)
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_clicks_keep_the_view_and_select_words() -> Result<()> {
  let mut app = text_app()?;
  let mouse = MouseEvent {
    kind: MouseEventKind::Down(MouseButton::Left),
    column: 4,
    row: 17,
    modifiers: KeyModifiers::NONE,
  };
  let press = Route { pane: app.id, region: Region::Page, focus: true, mouse };
  app.draw(&mut Vec::new())?;
  let target = app.text_position(mouse.column, mouse.row)?;
  app.mouse(press)?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.reader.cursor, target);
  let again = app.text_position(mouse.column, mouse.row)?;
  assert_eq!(again, target, "a click keeps the view");
  app.mouse(press)?;
  let text = app.doc.text(target.page)?;
  let word = text.lines[target.line].run(target.glyph);
  let selection = app.reader.selection.expect("a double click selects");
  let ends = (selection.anchor.glyph, app.reader.cursor.glyph);
  assert_eq!(Some(ends), word, "the word under the pointer");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_view_aligns_and_scrolls_with_the_cursor() -> Result<()> {
  fn cursor_row(app: &mut App) -> Result<u16> {
    app.draw(&mut Vec::new())?;
    let cell = app.text_cell(app.reader.cursor)?;
    Ok(cell.expect("cursor in view").1)
  }
  let mut app = text_app()?;
  let lines = app.doc.text(PageId(0))?.lines.len();
  assert!(lines >= 30 + 19, "zt on line 30 fills the view");
  app.reader.cursor.line = 30;
  let input =
    Input { action: Action::Quit, count: 1, explicit: false, register: '"' };
  for (fraction, row) in [(0.0, 0), (1.0, 18), (0.5, 9)] {
    app.action(Input { action: Action::Align(fraction), ..input })?;
    assert_eq!(cursor_row(&mut app)?, row, "align {fraction}");
  }
  for (delta, line) in [(5, 35), (-3, 32)] {
    app.scroll_text(delta)?;
    assert_eq!(app.reader.cursor.line, line);
    assert_eq!(cursor_row(&mut app)?, 9, "the cursor keeps its row");
  }
  app.scroll_text((lines - 31) as isize)?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  let row = usize::from(cursor_row(&mut app)?);
  assert_eq!(row, app.reader.cursor.line, "a new page starts at its top");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn changed_file_reloads_after_settling() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("live.pdf");
  std::fs::copy(root.join("paper.pdf"), &path)?;
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&path, "", false, screen)?;
  let pages = app.doc.pages.len();
  app.command("page 3")?;
  assert!(!app.watch()?);
  std::fs::copy(root.join("book.pdf"), &path)?;
  assert!(!app.watch()?, "a change waits for the write to settle");
  assert!(!app.watch()?);
  std::thread::sleep(Duration::from_millis(120));
  assert!(app.watch()?);
  assert_ne!(app.doc.pages.len(), pages);
  assert_eq!(app.reader.cursor.page, PageId(2));
  assert!(!app.watch()?);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn link_hints_follow_typed_labels() -> Result<()> {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
  app.command("page 2")?;
  let page = app.reader.cursor.page;
  for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
    app.start_hints(false)?;
    let hints = app.hints.as_ref().unwrap();
    let first = hints.items[0].label.chars().next().unwrap();
    app.hint_key(KeyEvent::new(KeyCode::Char(first), modifiers))?;
    assert!(app.hints.is_none(), "{modifiers:?} closes hints");
    assert_eq!(app.reader.cursor.page, page, "{modifiers:?} follows nothing");
  }
  app.start_hints(false)?;
  let hints = app.hints.as_ref().unwrap();
  assert!(hints.items.len() > 3);
  let label = hints.items[0].label.clone();
  for ch in label.chars() {
    app.hint_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
  }
  assert!(app.hints.is_none());
  assert!(app.reader.cursor.page.0 > 1, "citation jumps to the references");
  app.start_hints(false)?;
  let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
  app.hint_key(escape)?;
  assert!(app.hints.is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_shows_citation_target() -> Result<()> {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut history = vpdf::prompt::History::default();
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  for text_mode in [false, true] {
    let mut app =
      App::with_screen(&root.join("paper.pdf"), "", text_mode, screen)?;
    let links = app.doc.links(PageId(1))?;
    let link =
      links.iter().find(|link| !matches!(link.target, Target::Uri(_))).unwrap();
    let (x, y) = (
      f32::midpoint(link.bounds.x0, link.bounds.x1),
      f32::midpoint(link.bounds.y0, link.bounds.y1),
    );
    app.reader.cursor = app.doc.text(PageId(1))?.nearest(PageId(1), x, y);
    app.ensure_cursor()?;
    app.preview_link()?;
    assert!(app.peek.is_some());
    let mut out = Vec::new();
    app.draw(&mut out)?;
    let drawn = String::from_utf8_lossy(&out);
    assert_eq!(drawn.contains("z=2"), !text_mode);
    assert_eq!(drawn.contains('╭'), text_mode);
    assert_eq!(app.reader.cursor.page, PageId(1), "preview does not move");
    // A focused panel shows that closing keys cancel nothing else.
    app.open_panel(PanelKind::Outline)?;
    let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
    let control = |ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL);
    for key in [
      plain(KeyCode::Char('q')),
      plain(KeyCode::Char('K')),
      plain(KeyCode::Esc),
      control('c'),
      control('['),
    ] {
      app.preview_link()?;
      assert!(app.peek.is_some());
      app.key(key, &mut history)?;
      assert!(app.peek.is_none() && !app.quit, "{key:?} only closes it");
      assert!(app.panel_focused(), "{key:?} cancels nothing");
    }
    let on_link = app.reader.cursor;
    let target = match &link.target {
      Target::Page(page) | Target::Point { page, .. } => *page,
      Target::Uri(_) => unreachable!("a document link"),
    };
    for key in [plain(KeyCode::Enter), control('m')] {
      app.reader.cursor = on_link;
      app.preview_link()?;
      app.key(key, &mut history)?;
      assert!(app.peek.is_none(), "{key:?} closes it");
      assert_eq!(app.reader.cursor.page, target, "{key:?} follows it");
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn overview_places_thumbnails_and_opens_pages() -> Result<()> {
  use vpdf::keys::{Action, Input};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 137,
    rows: 36,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let press = |app: &mut App, action| {
    app.action(Input { action, count: 1, explicit: false, register: '"' })
  };
  press(&mut app, Action::Overview)?;
  assert!(app.overview.is_some());
  let mut out = Vec::new();
  app.draw(&mut out)?;
  let started = Instant::now();
  while app.graphics.bytes() == 0 {
    assert!(started.elapsed() < Duration::from_secs(10), "thumbnails render");
    app.raster_updates();
    app.draw(&mut out)?;
    std::thread::sleep(Duration::from_millis(5));
  }
  let view = (app.reader.scroll, app.reader.cursor);
  for action in [
    Action::Hints(false),
    Action::Peek,
    Action::Result(false),
    Action::Search(false),
    Action::Rows(1),
  ] {
    press(&mut app, action)?;
    assert!(app.overview.is_some(), "{action:?} keeps the overview");
    assert!(app.hints.is_none() && app.peek.is_none() && app.prompt.is_none());
    assert_eq!((app.reader.scroll, app.reader.cursor), view, "{action:?}");
    assert!(app.reader.message.is_empty(), "{action:?}");
  }
  press(&mut app, Action::Command)?;
  assert!(app.prompt.take().is_some(), "`:` reaches the command line");
  for action in [Action::Window(vpdf::app::Window::Next), Action::Tab(true)] {
    press(&mut app, action)?;
    assert!(app.request.take().is_some(), "{action:?} reaches the session");
    assert!(app.overview.is_some(), "{action:?} keeps the overview");
  }
  press(&mut app, Action::Motion(vpdf::motion::Motion::Down))?;
  press(&mut app, Action::Follow)?;
  assert!(app.overview.is_none());
  assert!(app.reader.cursor.page.0 > 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn unfocused_side_is_dimmed() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
  common::outline_loaded(&mut app)?;
  app.open_panel(PanelKind::Outline)?;
  let mut out = Vec::new();
  app.draw(&mut out)?;
  let veil = format!(",c={},r=39", 120 - app.left);
  assert!(String::from_utf8_lossy(&out).contains(&veil), "PDF dims");
  app.panel.as_mut().unwrap().focused = false;
  out.clear();
  app.draw(&mut out)?;
  let drawn = String::from_utf8_lossy(&out);
  assert!(!drawn.contains(&veil) && drawn.contains("\x1b[0;2m"), "panel dims");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn leaving_text_preview_keeps_the_panel() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let quit =
    Input { action: Action::Quit, count: 1, explicit: false, register: '"' };
  for (open, close) in [
    (Action::Panel(PanelKind::Outline), quit),
    (
      Action::Panel(PanelKind::Outline),
      Input { action: Action::Panel(PanelKind::Text), ..quit },
    ),
  ] {
    app.action(Input { action: open, ..quit })?;
    app.action(Input { action: Action::Panel(PanelKind::Text), ..quit })?;
    assert!(app.text_mode);
    app.action(close)?;
    assert!(!app.text_mode, "back to the rendered view");
    assert!(app.panel.is_some(), "outline stays open");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn escape_restores_the_previous_search() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  let settle = |app: &mut App| -> Result<()> {
    let start = Instant::now();
    while !matches!(app.search.phase, vpdf::find::Phase::Complete) {
      assert!(start.elapsed() < Duration::from_secs(15));
      app.search_updates()?;
      std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
  };
  app.start_search("python", false, false)?;
  settle(&mut app)?;
  let hits = app.search.hits.len();
  app.begin_search(false);
  app.incremental("pyth")?;
  app.abort_search()?;
  settle(&mut app)?;
  assert_eq!(app.search.query.text, "python");
  assert_eq!(app.search.hits.len(), hits);
  // `?<CR>` repeats the pattern backward.
  app.begin_search(true);
  app.finish_search("", true)?;
  assert!(app.search.query.backward);
  Ok(())
}

/// Lowest pixel row any graphics placement in `frame` reaches, with or
/// without tmux passthrough; a stretched placement spans `r` rows.
fn placed_bottom(frame: &str, cell_height: u32) -> u32 {
  let reach = |part: &str| {
    let (row, rest) = part.split_once(';')?;
    let control = rest.split_once("\x1b_G")?.1.split_once(';')?.0;
    let field = |name: &str| {
      control
        .split(',')
        .find_map(|pair| pair.strip_prefix(name)?.parse::<u32>().ok())
    };
    let top =
      (row.parse::<u32>().ok()? - 1) * cell_height + field("Y=").unwrap_or(0);
    let height = field("r=").map(|rows| rows * cell_height).or(field("h="))?;
    Some(top + height)
  };
  let frame = frame.replace("\x1b\x1b", "\x1b");
  frame.split("\x1b7\x1b[").skip(1).filter_map(reach).max().unwrap_or(0)
}

/// Opens a view of the pane, such as the overview.
type Open = fn(&mut App) -> Result<()>;

fn drawn(app: &mut App) -> Result<String> {
  let mut out = Vec::new();
  app.draw(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

/// A frame once rendering stops bringing tiles.
fn settled(app: &mut App) -> Result<String> {
  let started = Instant::now();
  let mut idle = 0;
  loop {
    assert!(started.elapsed() < Duration::from_secs(20), "tiles render");
    idle = if app.raster_updates() { 0 } else { idle + 1 };
    let frame = drawn(app)?;
    if frame.contains("z=-2") && idle > 100 {
      return Ok(frame);
    }
    std::thread::sleep(Duration::from_millis(5));
  }
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn clues_and_menu_cover_the_page_without_scrolling() -> Result<()> {
  use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(&root.join("paper.pdf"), "", false, screen)?;
  let mut history = vpdf::prompt::History::default();
  let mut key = |app: &mut App, code| {
    let result = app.key(KeyEvent::new(code, KeyModifiers::NONE), &mut history);
    app.after_key();
    result
  };
  app.command("page 2")?;
  // The cursor line sits on the bottom row, under either overlay.
  let bottom = Action::Align(1.0);
  app.action(Input {
    action: bottom,
    count: 1,
    explicit: false,
    register: '"',
  })?;
  let scroll = app.reader.scroll;
  app.relayout()?;
  assert_eq!(app.reader.scroll, scroll, "the same page area keeps the view");
  let views: [(&str, Open); 3] = [
    ("page", |_| Ok(())),
    ("overview", App::open_overview),
    ("focused outline", |app| app.open_panel(PanelKind::Outline)),
  ];
  // `Esc` after clues closes the overview, so they come last.
  let menu: &[KeyCode] = &[KeyCode::Char(':'), KeyCode::Tab];
  let clues: &[KeyCode] = &[KeyCode::Char('g')];
  for (view, open) in views {
    for keys in [menu, clues] {
      open(&mut app)?;
      let scroll = app.reader.scroll;
      let full = placed_bottom(&settled(&mut app)?, 16);
      for &code in keys {
        key(&mut app, code)?;
      }
      // Clues show once a prefix idles.
      app.awaiting = Instant::now().checked_sub(Duration::from_secs(1));
      app.clue_due();
      let rows = app.menu_rows();
      let uncovered = u32::from(screen.rows - 1 - rows) * 16;
      let reached = placed_bottom(&drawn(&mut app)?, 16);
      let at = format!(
        "{view} {keys:?}: full {full}, reached {reached}, uncovered {uncovered}"
      );
      assert!(rows > 0, "{at}");
      assert_eq!(app.layout.screen.rows, screen.rows, "{at}");
      assert!(full > uncovered, "the page reaches the overlay: {at}");
      assert!(reached > 0, "graphics still show: {at}");
      assert!(reached <= uncovered, "graphics stop above it: {at}");
      key(&mut app, KeyCode::Esc)?;
      assert!(app.clues.is_none() && app.prompt.is_none(), "{at}");
      assert_eq!(app.reader.scroll, scroll, "{at}");
    }
  }
  Ok(())
}

/// Once due, a prefix stops waiting, or the idle loop would poll with a
/// zero timeout.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn clue_wait_ends_once_due() -> Result<()> {
  use crossterm::event::{KeyCode, KeyEvent};
  let mut app = text_app()?;
  let mut history = vpdf::prompt::History::default();
  let mut press = |app: &mut App, code| {
    let result = app.key(KeyEvent::new(code, KeyModifiers::NONE), &mut history);
    app.after_key();
    result
  };
  assert_eq!(app.clue_wait(), None);
  // A wait with nothing to show ends too.
  app.awaiting = Instant::now().checked_sub(Duration::from_secs(1));
  assert!(!app.clue_due());
  assert_eq!(app.clue_wait(), None);
  for prefix in ['"', 'm', 'g'] {
    press(&mut app, KeyCode::Char(prefix))?;
    let wait = app.clue_wait();
    assert!(wait.is_some_and(|wait| wait <= Duration::from_secs(1)));
    app.awaiting = Instant::now().checked_sub(Duration::from_secs(1));
    assert_eq!(app.clue_wait(), Some(Duration::ZERO), "{prefix}");
    assert!(app.clue_due(), "{prefix}");
    assert_eq!(app.clue_wait(), None, "{prefix}");
    press(&mut app, KeyCode::Esc)?;
  }
  Ok(())
}
