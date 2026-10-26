use super::common::SeedReader;
use crate::common;
use crate::focus::send;
pub(super) use crate::harness::draw as drawn;
use crate::harness::{corpus, page_route, screen, session, solo, temp_copy};
use unicode_width::UnicodeWidthChar;

use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::solo::Solo;
use vpdf_model::document::Target;
use vpdf_model::input::{
  KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use vpdf_model::text::{Bounds, PageId};
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::layout::Screen;
use vpdf_ui::panel::Row;
use vpdf_ui::pointer::{Hit, Region, Route};

#[test]
#[ignore = "requires verified public PDF corpus"]
fn reader_workflows() -> Result<()> {
  let screen = common::cells(120, 50);
  let mut app = solo("book.pdf", false, screen)?;
  app.command("39")?;
  assert_eq!(app.reader.cursor.page, PageId(38));
  // Labels beat the `:info` prefix, not exact names.
  for (index, label) in ["cover", "title", "i", "iv", "q"].iter().enumerate() {
    app.doc.pages[index].label = (*label).to_owned().into();
  }
  app.command("i")?;
  assert_eq!(app.reader.cursor.page, PageId(2));
  app.command("iv")?;
  assert_eq!(app.reader.cursor.page, PageId(3));
  app.command("q")?;
  assert!(app.quit);
  app.quit = false;
  common::outline_loaded(&mut app)?;
  app.open_panel(PanelKind::Outline)?;
  assert!(!app.panel.as_ref().unwrap().view.rows().is_empty());
  app.close_panel()?;
  app.open_panel(PanelKind::Text)?;
  assert!(app.text_mode);
  app.action(Input::plain(Action::Quit))?;
  assert!(!app.text_mode && !app.quit);
  app.start_search("python", false, false)?;
  common::search_loaded(&mut app)?;
  assert!(!app.search.hits().is_empty());
  app.next_result(true, 2)?;
  app.open_panel(PanelKind::Results)?;
  assert_eq!(
    app.panel.as_ref().unwrap().view.rows().len(),
    app.search.hits().len()
  );
  app.action(Input::plain(Action::Cancel))?;
  assert!(!app.search.hidden());
  app.command("cancel")?;
  assert!(app.search.hidden());
  app.follow(&Target::Page(PageId(usize::MAX)))?;
  assert_eq!(app.reader.cursor.page.0, app.doc.pages.len() - 1);
  assert!(app.follow(&Target::Uri("file:///etc/passwd".into())).is_err());
  app.reader.visual(vpdf_reader::reader::SelectionKind::Character);
  app.command("reload")?;
  assert!(app.reader.last_selection.is_none());
  app.raster_updates();
  assert_eq!(app.graphics.bytes(), 0);
  assert!(app.search.hits().is_empty());
  app.reader.seed_reader(|reader| reader.history(false));
  assert!(app.reader.cursor.page.0 < app.doc.pages.len());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn scrolling_converges_and_jumps_interrupt() -> Result<()> {
  let screen = common::cells(120, 50);
  let mut app = solo("book.pdf", false, screen)?;
  app.scroll(100.0)?;
  app.scroll(100.0)?;
  assert_eq!(app.reader.animation().target().unwrap(), 200.0);
  assert_eq!(app.reader.scroll, 0.0);
  for _ in 0..30 {
    if let Some(target) = app.reader.animation().target() {
      app.reader.animate_to(
        target,
        Instant::now().checked_sub(Duration::from_millis(16)).unwrap(),
      );
    }
    let previous = app.reader.scroll;
    app.animate();
    assert!(app.reader.scroll >= previous && app.reader.scroll <= 200.0);
  }
  assert!(!app.reader.animation().active());
  assert_eq!(app.reader.scroll, 200.0);
  for action in [Action::Cancel, Action::History(false), Action::Align(0.0)] {
    app.scroll(100.0)?;
    app.action(Input::plain(action))?;
    assert!(!app.reader.animation().active());
    let stopped = app.reader.scroll;
    assert!(!app.animate());
    assert_eq!(app.reader.scroll, stopped);
  }
  app.scroll(100.0)?;
  app.command("39")?;
  assert!(!app.reader.animation().active());
  assert_eq!(app.reader.cursor.page, PageId(38));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn links_keep_coordinates_and_text_scrolls_lines() -> Result<()> {
  let screen = common::cells(120, 50);
  let mut app = solo("book.pdf", false, screen)?;
  let target = (0..app.doc.pages.len())
    .find_map(|page| {
      app
        .doc
        .links(PageId(page))
        .ok()?
        .iter()
        .find(
          |link| matches!(link.target, Target::Point { y, .. } if y > 100.0),
        )
        .cloned()
    })
    .expect("real book has positioned internal links")
    .target;
  let Target::Point { page, x, y } = target else { unreachable!() };
  let expected = app.doc.text(page)?.nearest(page, x, y);
  app.follow(&target)?;
  assert_eq!(app.reader.cursor, expected);
  app.command("39")?;
  app.open_panel(PanelKind::Text)?;
  let cursor = app.reader.cursor;
  app.scroll(48.0)?;
  assert_eq!(app.reader.cursor, cursor);
  assert!(!app.reader.animation().active());
  let position = app.text().position(2, 1)?;
  assert!(position.page.0 >= cursor.page.0);
  assert!(position.page != cursor.page || position.line > cursor.line);
  assert_eq!(position.glyph, 2);
  Ok(())
}

/// `paper.pdf` in text mode, 20 rows tall.
fn text_app() -> Result<Solo> {
  let screen = common::cells(120, 20);
  solo("paper.pdf", true, screen)
}

#[test]
fn info_tracks_frames() -> Result<()> {
  let (_directory, path) = common::temp_pdf(|path| {
    common::pages_pdf(path, &[&["First page"], &["Second page"]])
  })?;
  let short = Screen { rows: 12, ..screen() };
  let mut app = Solo::open(&path, "", true, short)?;
  app.command("info")?;
  app.draw(&mut Vec::new())?;
  let panel = &mut app.panel.as_mut().unwrap().view;
  let tile = panel
    .rows()
    .iter()
    .position(|row| row.label.starts_with("Last tile: "))
    .unwrap();
  panel.select(tile);
  panel.begin_search(false);
  panel.preview("Last tile");
  panel.finish_search("Last tile");
  let top = panel.top();
  for focused in [true, false] {
    app.panel.as_mut().unwrap().view.focus(focused);
    app.render_ms = if focused { 12.5 } else { 25.0 };
    app.command("2")?;
    app.screen.columns = 100;
    app.relayout()?;
    app.draw(&mut Vec::new())?;
    let expected: Vec<_> =
      app.info().lines().map(|line| Row::parse(line).label).collect();
    let panel = &app.panel.as_ref().unwrap().view;
    assert_eq!(
      panel.rows().iter().map(|row| &row.label).collect::<Vec<_>>(),
      expected.iter().collect::<Vec<_>>()
    );
    assert_eq!((panel.selected(), panel.top()), (tile, top));
    assert_eq!(panel.focused(), focused);
    assert_eq!(panel.query(), "Last tile");
    assert_eq!(
      panel.rows()[tile].label,
      format!("Last tile: {:.3} ms", app.render_ms)
    );
  }
  let row = tile - app.panel.as_ref().unwrap().view.top() + 1;
  app.panel_mouse(
    MouseEvent::new(MouseEventKind::Down(MouseButton::Right), 1, row as u16),
    1,
  )?;
  assert!(
    app.overlays.menu().is_some_and(vpdf_ui::context::ContextMenu::is_row)
  );
  app.render_ms = 50.0;
  app.draw(&mut Vec::new())?;
  assert!(app.overlays.menu().is_some());
  Ok(())
}

#[test]
fn info_tracks_prefetched_tiles() -> Result<()> {
  use vpdf_model::tile::TileKey;
  use vpdf_terminal::payload::{Encoder, Tile};
  let (_directory, mut app) = common::temp_solo(
    |path| common::text_pdf(path, &["Background tile"]),
    true,
    screen(),
  )?;
  let (send, receive) = std::sync::mpsc::channel();
  app.worker.results = receive;
  let key = TileKey {
    scale: 1.0f32.to_bits(),
    width: 1,
    height: 1,
    ..TileKey::default()
  };
  for info in [false, true] {
    if info {
      app.command("info")?;
    }
    send.send(Ok(Tile {
      key,
      payload: Encoder::default().encode(&[0, 0, 0])?,
      render_ms: 12.5,
    }))?;
    assert_eq!(app.raster_updates(), info);
  }
  assert!(!app.raster_updates(), "idle info needs no frame");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_clicks_keep_the_view_and_select_words() -> Result<()> {
  let mut app = text_app()?;
  let down = MouseEventKind::Down(MouseButton::Left);
  let press = page_route(app.id, down, (4, 17));
  let mouse = press.mouse;
  app.draw(&mut Vec::new())?;
  let target = app.text().position(mouse.column, mouse.row)?;
  app.mouse(press)?;
  app.draw(&mut Vec::new())?;
  assert_eq!(app.reader.cursor, target);
  assert_eq!(app.reader.target(), Some(target));
  assert!(app.reader.selection.is_none());
  let again = app.text().position(mouse.column, mouse.row)?;
  assert_eq!(again, target, "a click keeps the view");
  app.mouse(Route { clicks: 2, ..press })?;
  let text = app.doc.text(target.page)?;
  let word = text.lines[target.line].run(target.glyph);
  let selection = app.reader.selection.expect("a double click selects");
  let ends = (selection.anchor.glyph, app.reader.cursor.glyph);
  assert_eq!(Some(ends), word, "the word under the pointer");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_view_scrolls_without_moving_the_selection() -> Result<()> {
  let mut app = text_app()?;
  let lines = app.doc.text(PageId(0))?.lines.len();
  app.reader.seed_reader(|reader| reader.cursor.line = 30);
  app.reader.visual(vpdf_reader::reader::SelectionKind::Character);
  app.ensure_cursor()?;
  let input = Input::plain(Action::Quit);
  for (fraction, row) in [(0.0, 0), (1.0, 18), (0.5, 9)] {
    app.action(Input { action: Action::Align(fraction), ..input })?;
    {
      let (app, _) = app.parts();
      let cursor = app.reader.cursor;
      assert_eq!(app.text().cell(cursor)?.unwrap().1, row);
    }
  }
  let cursor = app.reader.cursor;
  let before = app.text().position(0, 0)?;
  app.text().scroll(5)?;
  assert_eq!(app.reader.cursor, cursor);
  app.draw(&mut Vec::new())?;
  assert_eq!(app.text().position(0, 0)?.line, before.line + 5);
  app.text().scroll(lines as isize)?;
  assert_eq!(app.navigation_position(), cursor);
  assert_eq!(app.reader.cursor, cursor);
  assert!(app.text().cell(cursor)?.is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn changed_file_reloads_after_settling() -> Result<()> {
  let (_directory, path) = temp_copy("paper.pdf")?;
  let screen = common::cells(120, 50);
  let mut app = Solo::open(&path, "", false, screen)?;
  let pages = app.doc.pages.len();
  app.command("3")?;
  assert!(!app.watch()?);
  std::fs::copy(corpus("book.pdf")?, &path)?;
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
fn hint_labels_keep_their_origin() -> Result<()> {
  let mut app = text_app()?;
  app.at = vpdf_model::geometry::At { column: 17, row: 4 };
  app.layout.relocate((17, 4));
  app.left = 9;
  for text_mode in [true, false] {
    app.text_mode = text_mode;
    for typed in ["", "a"] {
      app.overlays.open_hints(common::hints(
        24,
        (23, 3),
        &Target::Page(PageId(0)),
      ));
      if !typed.is_empty() {
        common::keys(&mut app, typed.as_bytes())?;
      }
      let mut out = Vec::new();
      {
        let (app, graphics) = app.parts();
        vpdf::presentation::draw(&mut out, Some(graphics), |s| {
          app.scene_hints(s);
        })?;
      }
      let drawn = String::from_utf8_lossy(&out);
      let rest = &"aa"[typed.len()..];
      assert!(drawn.starts_with(&format!(
        "\x1b[8;39H\x1b[0;1;30;43m{typed}\x1b[0;30;43m{rest}\x1b[0m"
      )));
      assert_eq!(drawn.contains("z=-1"), !text_mode);
      if !text_mode {
        assert!(drawn.contains("\x1b[8;39H\x1b_Ga=p"));
      }
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn link_hints_follow_typed_labels() -> Result<()> {
  use vpdf_model::input::{KeyCode, KeyEvent, KeyModifiers};
  let screen = common::cells(120, 50);
  let mut app = solo("paper.pdf", false, screen)?;
  app.command("2")?;
  let page = app.reader.cursor.page;
  for modifiers in [KeyModifiers::CONTROL, KeyModifiers::ALT] {
    app.start_hints(false)?;
    let hints = app.overlays.hints().unwrap();
    let first = hints.items()[0].label.chars().next().unwrap();
    app.hint_key(KeyEvent::new(KeyCode::Char(first), modifiers))?;
    assert!(app.overlays.hints().is_none(), "{modifiers:?} closes hints");
    assert_eq!(app.reader.cursor.page, page, "{modifiers:?} follows nothing");
  }
  app.start_hints(false)?;
  let hints = app.overlays.hints().unwrap();
  assert!(hints.items().len() > 3);
  let label = hints.items()[0].label.clone();
  for ch in label.chars() {
    app.hint_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
  }
  assert!(app.overlays.hints().is_none());
  assert!(app.reader.cursor.page.0 > 1, "citation jumps to the references");
  app.start_hints(false)?;
  let escape = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
  app.hint_key(escape)?;
  assert!(app.overlays.hints().is_none());
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn preview_shows_citation_target() -> Result<()> {
  let screen = common::cells(120, 50);
  for text_mode in [false, true] {
    let mut app = solo("paper.pdf", text_mode, screen)?;
    let links = app.doc.links(PageId(1))?;
    let link =
      links.iter().find(|link| !matches!(link.target, Target::Uri(_))).unwrap();
    let (x, y) = (
      f32::midpoint(link.first().x0, link.first().x1),
      f32::midpoint(link.first().y0, link.first().y1),
    );
    {
      let seed_value = app.doc.text(PageId(1))?.nearest(PageId(1), x, y);
      app.reader.seed_reader(|reader| reader.cursor = seed_value);
    };
    app.ensure_cursor()?;
    let cursor = app.reader.cursor;
    app.reader.point_to(cursor);
    app.preview_link()?;
    assert!(app.overlays.preview().is_some());
    let mut out = Vec::new();
    app.draw(&mut out)?;
    let drawn = String::from_utf8_lossy(&out);
    assert_eq!(drawn.contains("z=2"), !text_mode);
    assert!(drawn.contains('╭'));
    assert_eq!(app.reader.cursor.page, PageId(1), "preview does not move");
    // A focused panel shows that closing keys cancel nothing else.
    app.open_panel(PanelKind::Outline)?;
    // q, K, Esc, Ctrl-c, Ctrl-[
    for key in [b"q".as_slice(), b"K", b"\x1b", b"\x03", b"\x1b[91;5u"] {
      app.preview_link()?;
      assert!(app.overlays.preview().is_some());
      common::keys(&mut app, key)?;
      assert!(
        app.overlays.preview().is_none() && !app.quit,
        "{key:?} only closes it"
      );
      assert!(app.panel_focused(), "{key:?} cancels nothing");
    }
    let on_link = app.reader.cursor;
    let target = match &link.target {
      Target::Page(page) | Target::Point { page, .. } => *page,
      Target::Uri(_) => unreachable!("a document link"),
    };
    // Enter, Ctrl-m
    for key in [b"\r".as_slice(), b"\x1b[109;5u"] {
      app.go_to(on_link, true)?;
      app.preview_link()?;
      common::keys(&mut app, key)?;
      assert!(app.overlays.preview().is_none(), "{key:?} closes it");
      assert_eq!(app.reader.cursor.page, target, "{key:?} follows it");
    }
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn overview_places_thumbnails_and_opens_pages() -> Result<()> {
  use vpdf_ui::keys::{Action, Input};
  let screen = common::cells(137, 36);
  let mut app = solo("book.pdf", false, screen)?;
  let press = |app: &mut App, action| app.action(Input::plain(action));
  press(&mut app, Action::Overview)?;
  assert!(app.overlays.overview().is_some());
  let mut out = Vec::new();
  app.draw(&mut out)?;
  common::wait_until(10, "Thumbnails", || {
    app.raster_updates();
    app.draw(&mut out)?;
    Ok(app.graphics.bytes() > 0)
  })?;
  let view = (app.reader.scroll, app.reader.cursor);
  for action in [
    Action::Hints(false),
    Action::Peek,
    Action::Result(false),
    Action::Search(false),
    Action::Rows(1),
  ] {
    press(&mut app, action)?;
    assert!(app.overlays.overview().is_some(), "{action:?} keeps the overview");
    assert!(
      app.overlays.hints().is_none()
        && app.overlays.preview().is_none()
        && app.overlays.prompt().is_none()
    );
    assert_eq!((app.reader.scroll, app.reader.cursor), view, "{action:?}");
    assert!(app.reader.message.is_empty(), "{action:?}");
  }
  press(&mut app, Action::Command)?;
  assert!(
    app.overlays.close_prompt().is_some(),
    "`:` reaches the command line"
  );
  for action in
    [Action::Window(vpdf_ui::window::Window::Next), Action::Tab(true)]
  {
    press(&mut app, action)?;
    assert!(app.request.take().is_some(), "{action:?} reaches the session");
    assert!(app.overlays.overview().is_some(), "{action:?} keeps the overview");
  }
  press(&mut app, Action::Motion(vpdf_reader::motion::Motion::Down))?;
  press(&mut app, Action::Follow)?;
  assert!(app.overlays.overview().is_none());
  assert!(app.reader.cursor.page.0 > 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn unfocused_side_is_dimmed() -> Result<()> {
  let screen = common::cells(120, 40);
  let mut app = solo("paper.pdf", false, screen)?;
  common::outline_loaded(&mut app)?;
  app.open_panel(PanelKind::Outline)?;
  let mut out = Vec::new();
  app.draw(&mut out)?;
  let veil = format!(",c={},r=39", 120 - app.left);
  assert!(String::from_utf8_lossy(&out).contains(&veil), "PDF dims");
  app.panel.as_mut().unwrap().view.focus(false);
  out.clear();
  app.draw(&mut out)?;
  let drawn = String::from_utf8_lossy(&out);
  assert!(!drawn.contains(&veil) && drawn.contains("\x1b[0;2m"), "panel dims");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn leaving_text_preview_keeps_the_panel() -> Result<()> {
  let screen = common::cells(120, 40);
  let mut app = solo("book.pdf", false, screen)?;
  let quit = Input::plain(Action::Quit);
  app.action(Input { action: Action::Panel(PanelKind::Outline), ..quit })?;
  app.action(Input { action: Action::Panel(PanelKind::Text), ..quit })?;
  assert!(app.text_mode);
  app.action(quit)?;
  assert!(!app.text_mode, "back to the rendered view");
  assert!(app.panel.is_some(), "outline stays open");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn escape_restores_the_previous_search() -> Result<()> {
  let screen = common::cells(120, 40);
  let mut app = solo("book.pdf", false, screen)?;
  app.start_search("python", false, false)?;
  common::search_loaded(&mut app)?;
  let hits = app.search.hits().len();
  app.begin_search(false)?;
  app.incremental("pyth")?;
  app.abort_search()?;
  common::search_loaded(&mut app)?;
  assert_eq!(app.search.query().text, "python");
  assert_eq!(app.search.hits().len(), hits);
  // `?<CR>` repeats the pattern backward.
  app.begin_search(true)?;
  app.finish_search("", true)?;
  assert!(app.search.query().backward);
  Ok(())
}

/// Lowest pixel row any graphics placement in `frame` reaches, with or
/// without tmux passthrough; a stretched placement spans `r` rows.
fn placed_bottom(frame: &str, cell_height: u32) -> u32 {
  let reach = |command: crate::kitty::Command| {
    let field = |name| command.number::<u32>(name);
    let top = (u32::from(command.at?.0) - 1) * cell_height;
    let height = field("r").map(|rows| rows * cell_height).or(field("h"))?;
    Some(top + field("Y").unwrap_or(0) + height)
  };
  let commands = crate::kitty::commands(frame).into_iter();
  commands.filter_map(reach).max().unwrap_or(0)
}

/// Opens a view of the pane, such as the overview.
type Open = fn(&mut App) -> Result<()>;

/// A frame once rendering stops bringing tiles.
fn settled(app: &mut Solo) -> Result<String> {
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
  let screen = common::cells(120, 40);
  let mut app = solo("paper.pdf", false, screen)?;
  app.command("2")?;
  // The cursor line sits on the bottom row, under either overlay.
  let bottom = Action::Align(1.0);
  app.action(Input::plain(bottom))?;
  let scroll = app.reader.scroll;
  app.relayout()?;
  assert_eq!(app.reader.scroll, scroll, "the same page area keeps the view");
  let views: [(&str, Open); 3] = [
    ("page", |_| Ok(())),
    ("overview", App::open_overview),
    ("focused outline", |app| app.open_panel(PanelKind::Outline)),
  ];
  // `Esc` after clues closes the overview, so they come last.
  for (view, open) in views {
    for keys in [":\t", "g"] {
      open(&mut app)?;
      let scroll = app.reader.scroll;
      let full = placed_bottom(&settled(&mut app)?, 16);
      common::typed(&mut app, keys.as_bytes())?;
      // Clues show once a prefix idles.
      common::age_clues(&mut app);
      app.clue_due(Instant::now());
      let rows = app.menu_rows();
      let uncovered = u32::from(screen.rows - 1 - rows) * 16;
      let reached = placed_bottom(&drawn(&mut app)?, 16);
      let at = format!(
        "{view} {keys:?}: full {full}, reached {reached}, uncovered {uncovered}"
      );
      assert!(rows > 0, "{at}");
      assert_eq!(app.layout.screen().rows, screen.rows, "{at}");
      assert!(full > uncovered, "the page reaches the overlay: {at}");
      assert!(reached > 0, "graphics still show: {at}");
      assert!(reached <= uncovered, "graphics stop above it: {at}");
      common::typed(&mut app, b"\x1b")?;
      assert!(
        app.overlays.clues().shown().is_none()
          && app.overlays.prompt().is_none(),
        "{at}"
      );
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
  let mut app = text_app()?;
  assert_eq!(app.clue_wait(Instant::now()), None);
  // A wait with nothing to show ends too.
  common::age_clues(&mut app);
  assert!(!app.clue_due(Instant::now()));
  assert_eq!(app.clue_wait(Instant::now()), None);
  for prefix in ["\"", "m", "g"] {
    common::typed(&mut app, prefix.as_bytes())?;
    let wait = app.clue_wait(Instant::now());
    assert!(wait.is_some_and(|wait| wait <= Duration::from_secs(1)));
    common::age_clues(&mut app);
    assert_eq!(app.clue_wait(Instant::now()), Some(Duration::ZERO), "{prefix}");
    assert!(app.clue_due(Instant::now()), "{prefix}");
    assert_eq!(app.clue_wait(Instant::now()), None, "{prefix}");
    common::typed(&mut app, b"\x1b")?;
  }
  Ok(())
}

#[test]
fn recent_view_covers_the_pane() -> Result<()> {
  let (_directory, mut app) = common::temp_solo(
    |path| common::text_pdf(path, &["first page text"]),
    true,
    common::cells(120, 20),
  )?;
  app.doc.pages[0].label = "iv".to_owned().into();
  app.show_recent(&[])?;
  assert_eq!(app.left, app.screen.columns);
  let input = Input::plain;
  let scroll = app.reader.scroll;
  for action in [Action::Rows(5), Action::Panel(PanelKind::Recent)] {
    app.action(input(action))?;
    assert!(app.panel_focused(), "{action:?}");
    assert!(app.covered(), "{action:?}");
  }
  assert_eq!(app.reader.scroll, scroll);
  for command in ["1", "iv"] {
    assert!(app.command(command).is_err(), "{command}");
    assert!(app.covered() && app.overlays.overview().is_none(), "{command}");
  }
  app.draw(&mut Vec::new())?;
  app.action(input(Action::Quit))?;
  assert!(!app.quit && app.panel.is_none());
  let path = app.path.clone();
  app.show_panel(
    PanelKind::Recent,
    vec![Row::new(
      "fixture.pdf".into(),
      String::new(),
      vpdf_ui::panel::Destination::File(path),
    )],
  )?;
  app.action(input(Action::RecentToggle))?;
  app.action(input(Action::Cancel))?;
  assert!(app.covered() && app.panel_focused());
  app.action(input(Action::Cancel))?;
  assert!(!app.covered() && app.panel.is_none());
  assert_eq!(app.reader.scroll, scroll);
  app.show_recent(&[])?;
  app.command("text")?;
  assert!(app.text_mode && !app.covered() && !app.panel_focused());
  assert!(app.panel.as_ref().unwrap().view.is_recent_view());
  Ok(())
}

/// Each page cell with what the pointer hits there.
type Targets = Vec<((u16, u16), Hit)>;

fn hover_targets(app: &mut App) -> Result<Targets> {
  let mut targets = Vec::new();
  for row in 0..app.screen.rows - 1 {
    for column in 0..app.screen.columns {
      let hit = app.hovered(Region::Page, column, row, None)?;
      targets.push(((column, row), hit));
    }
  }
  Ok(targets)
}

/// Whether a projected box covers the cell centre.
fn covers(app: &App, cell: (u16, u16), boxes: &[(PageId, Bounds)]) -> bool {
  let (x, y) = (
    (f32::from(cell.0) + 0.5) * f32::from(app.screen.cell_width),
    (f32::from(cell.1) + 0.5) * f32::from(app.screen.cell_height),
  );
  boxes.iter().any(|&(page, bounds)| {
    app.layout.project(page, bounds, app.reader.scroll).contains(x, y)
  })
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn hover_hits_links_text_and_blanks() -> Result<()> {
  for text_mode in [false, true] {
    let mut app =
      if text_mode { text_app()? } else { solo("paper.pdf", false, screen())? };
    app.command("2")?;
    app.draw(&mut Vec::new())?;
    let targets = hover_targets(&mut app)?;
    let count = |wanted: fn(&Hit) -> bool| {
      targets.iter().filter(|(_, hit)| wanted(hit)).count()
    };
    assert!(count(|hit| matches!(hit, Hit::Link(..))) > 0, "text={text_mode}");
    assert!(count(|hit| *hit == Hit::Text) > 0, "text={text_mode}");
    assert!(count(|hit| *hit == Hit::Nothing) > 0, "text={text_mode}");
    let pages = app.layout.visible(app.reader.scroll);
    let (mut links, mut glyphs, mut gaps) =
      (Vec::new(), Vec::new(), Vec::new());
    for page in pages.map(PageId) {
      let page_links = app.doc.links(page)?;
      links.extend(page_links.iter().map(|link| (page, link.first())));
      let text = app.doc.text(page)?;
      let all = text.lines.iter().flat_map(|line| &line.glyphs);
      glyphs.extend(all.map(|glyph| (page, glyph.bounds)));
      // Half-gap boxes per line pair.
      for pair in text.lines.windows(2).filter(|pair| !pair[1].paragraph) {
        let (Some(upper), Some(lower)) = (pair[0].span(), pair[1].span())
        else {
          continue;
        };
        let middle = f32::midpoint(upper.y1, lower.y0);
        gaps.push((page, Bounds { y0: upper.y1, y1: middle, ..upper }));
        gaps.push((page, Bounds { y0: middle, y1: lower.y0, ..lower }));
      }
    }
    for &(cell, hit) in &targets {
      let at = format!("text={text_mode} {cell:?} {hit:?}");
      if text_mode {
        // The drawn glyph starts at or left of the cell.
        let drawn = app.text().hit(cell.0, cell.1)?;
        let start = match drawn {
          Some(position) => app.text().cell(position)?,
          None => None,
        };
        let width = match drawn {
          Some(position) => {
            let text = app.doc.text(position.page)?;
            let glyph = text.glyph(position);
            glyph.map_or(0, |glyph| glyph.ch.width().unwrap_or(0))
          }
          None => 0,
        };
        let starts = start.is_some_and(|(column, row)| {
          let width = u16::try_from(width).unwrap_or(u16::MAX);
          row == cell.1 && (column..column + width).contains(&cell.0)
        });
        assert_eq!(starts, hit != Hit::Nothing, "{at}");
        let link = match drawn {
          Some(position) => app.link_target(position)?.is_some(),
          None => false,
        };
        assert_eq!(link, matches!(hit, Hit::Link(..)), "{at}");
        continue;
      }
      if hit == Hit::Scrollbar {
        assert_eq!(cell.0 + 1, app.screen.columns, "{at}");
        continue;
      }
      let expected = match hit {
        Hit::Link(page, index) => {
          let bounds = app.doc.links(page)?[index].first();
          covers(&app, cell, &[(page, bounds)])
        }
        Hit::Text => {
          !covers(&app, cell, &links)
            && (covers(&app, cell, &glyphs) || covers(&app, cell, &gaps))
        }
        Hit::Nothing => {
          !covers(&app, cell, &links) && !covers(&app, cell, &glyphs)
        }
        Hit::Note { .. }
        | Hit::Row(_)
        | Hit::Label(_)
        | Hit::Thumbnail(_)
        | Hit::Preview { .. }
        | Hit::Item(..)
        | Hit::Card(_)
        | Hit::Edge { .. }
        | Hit::Chrome
        | Hit::Scrollbar => false,
      };
      assert!(expected, "{at}");
    }
  }
  Ok(())
}

#[test]
fn hover_hits_lines_rows_and_thumbnails() -> Result<()> {
  let lines = ["Hover over these lines", "one block of text", "steady shape"];
  let short = Screen { rows: 170, cell_height: 4, ..screen() };
  // 27 pt keeps one block, with gaps.
  let (_directory, mut app) = common::temp_solo(
    |path| common::spaced_pdf(path, &lines, 27),
    false,
    short,
  )?;
  app.draw(&mut Vec::new())?;
  // Text runs unbroken down a column.
  let column = (0..app.screen.columns)
    .find(|&column| {
      let hits = (0..app.screen.rows - 1).filter(|&row| {
        app.hovered(Region::Page, column, row, None).ok() == Some(Hit::Text)
      });
      hits.count() >= 3
    })
    .expect("a column across the lines");
  let mut hits = Vec::new();
  for row in 0..app.screen.rows - 1 {
    let (page, x, y) = app.page_point(column, row, None).expect("on the page");
    let text = app.doc.text(page)?;
    let glyphs = text.lines.iter().flat_map(|line| &line.glyphs);
    let on_glyph = glyphs.into_iter().any(|glyph| glyph.bounds.contains(x, y));
    hits.push((app.hovered(Region::Page, column, row, None)?, on_glyph));
  }
  let first = hits.iter().position(|hit| hit.0 == Hit::Text).expect("text");
  let last = hits.iter().rposition(|hit| hit.0 == Hit::Text).expect("text");
  assert!(hits[first..=last].iter().all(|hit| hit.0 == Hit::Text), "{hits:?}");
  assert!(hits[first..=last].iter().any(|hit| !hit.1), "a bridged gap");
  assert!(hits.iter().all(|hit| hit.0 == Hit::Text || !hit.1), "{hits:?}");
  // Panel rows hit what a press selects.
  app.open_panel(PanelKind::Version)?;
  app.draw(&mut Vec::new())?;
  let press = |column, row| {
    MouseEvent::new(MouseEventKind::Down(MouseButton::Left), column, row)
  };
  for row in 0..app.screen.rows - 1 {
    let hit = app.hovered(Region::Panel, 1, row, None)?;
    let pressed =
      app.panel.as_mut().expect("panel").view.pointer(press(1, row));
    assert_eq!(hit, pressed.map_or(Hit::Nothing, Hit::Label), "row {row}");
    if row < 2 {
      assert_eq!(hit, [Hit::Nothing, Hit::Label(0)][usize::from(row)]);
    }
  }
  app.close_panel()?;
  // A thumbnail press opens its page.
  app.open_overview()?;
  app.draw(&mut Vec::new())?;
  let thumbnail =
    |app: &mut App, column| app.hovered(Region::Overview, column, 2, None);
  assert_eq!(thumbnail(&mut app, 4)?, Hit::Thumbnail(PageId(0)));
  let right = app.screen.columns - 4;
  assert_eq!(thumbnail(&mut app, right)?, Hit::Nothing);
  app.overview_mouse(press(4, 2))?;
  assert!(app.overlays.overview().is_none(), "the press opened the page");
  Ok(())
}

/// Hover follows thumbnails the frame scrolls.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn frames_follow_scrolled_thumbnails() -> Result<()> {
  use vpdf::trace::Drawable;
  let mut session = session(&corpus("book.pdf")?, screen())?;
  session.app().command("39")?;
  let moved = MouseEvent::new(MouseEventKind::Moved, 20, 5);
  session.mouse(moved)?;
  session.app().open_overview()?;
  // The hit predicts the scroll; no second frame.
  session.draw_frame(&mut Vec::new())?;
  assert!(!session.redraw_due());
  let app = session.app();
  let under = app.hovered(Region::Overview, 20, 5, None)?;
  assert_eq!(app.hover.hit(), under);
  assert!(matches!(under, Hit::Thumbnail(page) if page.0 > 20), "{under:?}");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn motion_redraws_only_for_new_targets() -> Result<()> {
  let mut session = session(&corpus("paper.pdf")?, screen())?;
  let app = session.app();
  app.command("2")?;
  session.draw_focused(&mut Vec::new())?;
  let app = session.app();
  let targets = hover_targets(app)?;
  let link = targets.iter().find(|(_, hit)| matches!(hit, Hit::Link(..)));
  let ((column, row), _) = *link.expect("a visible link");
  // Two neighbours on one text target.
  let text = targets.windows(2).find(|pair| {
    pair.iter().all(|(_, hit)| *hit == Hit::Text) && pair[0].0.1 == pair[1].0.1
  });
  let [(first, _), (second, _)] = *text.expect("a word") else {
    unreachable!()
  };
  app.start_hints(false)?;
  let moved =
    |(column, row)| MouseEvent::new(MouseEventKind::Moved, column, row);
  // cell -> redraw
  let cases = [
    ((column, row), true),
    ((column, row), false),
    (first, true),
    (second, false),
    ((column, row), true),
    ((0, screen().rows - 1), true),
    ((1, screen().rows - 1), false),
  ];
  for (step, (cell, redraw)) in cases.into_iter().enumerate() {
    assert_eq!(session.mouse(moved(cell))?, redraw, "step {step}: {cell:?}");
  }
  assert!(session.app().overlays.hints().is_some(), "motion keeps hints");
  send(&mut session, b"\x1b[27u")?;
  let app = session.app();
  let Hit::Link(page, index) = link.expect("a visible link").1 else {
    unreachable!()
  };
  let bounds = app.doc.links(page)?[index].first();
  let x = f32::midpoint(bounds.x0, bounds.x1);
  let position =
    app.doc.text(page)?.nearest(page, x, f32::midpoint(bounds.y0, bounds.y1));
  app.go_to(position, true)?;
  send(&mut session, b"K")?;
  assert!(session.app().overlays.preview().is_some(), "K previews the link");
  // SGR cells count from one.
  let sgr = |flags, (column, row): (u16, u16)| {
    format!("\x1b[<{flags};{};{}M", column + 1, row + 1)
  };
  for cell in [first, second, (column, row)] {
    send(&mut session, sgr(35, cell).as_bytes())?;
  }
  assert!(
    session.app().overlays.preview().is_some(),
    "motion keeps the preview"
  );
  send(&mut session, b"\x1b[27u")?;
  // Motion after a press: the release got lost.
  let drag = [sgr(0, first), sgr(35, second), sgr(32, (column, row))];
  send(&mut session, drag.concat().as_bytes())?;
  assert!(session.app().reader.selection.is_none(), "no capture to drag");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn text_hover_hits_the_next_frame() -> Result<()> {
  let mut app = text_app()?;
  app.command("2")?;
  app.draw(&mut Vec::new())?;
  let before = hover_targets(&mut app)?;
  for _ in 0..3 {
    app.scroll(16.0)?;
  }
  let upcoming = hover_targets(&mut app)?;
  assert_ne!(before, upcoming, "scrolling moves the text");
  app.draw(&mut Vec::new())?;
  assert_eq!(upcoming, hover_targets(&mut app)?);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn frames_retest_a_resting_pointer() -> Result<()> {
  use vpdf::trace::Drawable;
  let mut session = session(&corpus("paper.pdf")?, screen())?;
  session.app().command("2")?;
  session.draw_frame(&mut Vec::new())?;
  let targets = hover_targets(session.app())?;
  let link = targets.iter().find(|(_, hit)| matches!(hit, Hit::Link(..)));
  let (cell, hit) = *link.expect("a visible link");
  let moved = MouseEvent::new(MouseEventKind::Moved, cell.0, cell.1);
  assert!(session.mouse(moved)?);
  // Half a screen puts another target there.
  session.app().reader.seed_reader(|reader| reader.scroll += 300.0);
  assert_ne!(session.app().hovered(Region::Page, cell.0, cell.1, None)?, hit);
  session.draw_frame(&mut Vec::new())?;
  assert!(!session.mouse(moved)?, "the frame retested the pointer");
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn pixel_reports_hit_their_pixel() -> Result<()> {
  use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
  use vpdf_ui::pointer::Pixel;
  let mut session = session(&corpus("paper.pdf")?, screen())?;
  session.app().command("2")?;
  let at = |x, y| Some(Pixel { x, y, cell: (8, 16) });
  for panel in [false, true] {
    let app = session.app();
    if panel {
      app.open_panel(PanelKind::Version)?;
    }
    let (left, scroll) = (app.left, app.reader.scroll);
    let (column, row) = (left + 40, 12);
    // (pixel offset into the cell) -> page-area pixel hit
    let cases = [
      ((0, 0), (0, 0)),
      ((7, 15), (7, 15)),
      ((3, 9), (3, 9)),
      ((30, 40), (7, 15)),
    ];
    for (offset, hit) in cases {
      let pixel = at(column * 8 + offset.0, row * 16 + offset.1);
      let got = app.page_point(column, row, pixel).expect("on a page");
      let x = f32::from((column - left) * 8 + hit.0);
      let at = (x, f32::from(row * 16 + hit.1));
      let (x, y) = app.layout.unproject(got.0, at, scroll).expect("invertible");
      assert_eq!(got, (got.0, x, y), "panel={panel} {offset:?}");
    }
    let centre = at(column * 8 + 4, row * 16 + 8);
    assert_eq!(
      app.page_point(column, row, None),
      app.page_point(column, row, centre)
    );
  }
  session.app().close_panel()?;
  // A press lands on the glyph nearest its pixel, not the cell centre.
  let app = session.app();
  let nearest = |app: &mut App, column, pixel| -> Result<_> {
    let (page, x, y) = app.page_point(column, 11, pixel).expect("on a page");
    Ok(app.doc.text(page)?.nearest(page, x, y))
  };
  let mut found = None;
  for column in 20..100 {
    let centre = nearest(app, column, None)?;
    for dx in [0, 7] {
      let pixel = at(column * 8 + dx, 11 * 16 + 8);
      let glyph = nearest(app, column, pixel)?;
      if glyph != centre {
        found = found.or(Some((pixel.expect("a pixel"), glyph)));
      }
    }
  }
  let (pixel, glyph) = found.expect("a cell spanning two glyphs");
  let press = Mouse {
    kind: MouseKind::Press(Button::LEFT),
    at: Spot::Pixel { x: pixel.x, y: pixel.y },
    modifiers: KeyModifiers::NONE,
  };
  assert!(session.mouse_report(press)?);
  assert_eq!(session.app().reader.cursor, glyph);
  // Drags draw only once they move the selection.
  let drag = |x| Mouse {
    kind: MouseKind::Drag(Button::LEFT),
    at: Spot::Pixel { x, y: pixel.y },
    modifiers: KeyModifiers::NONE,
  };
  assert!(!session.mouse_report(drag(pixel.x + 1))?, "below the threshold");
  assert!(session.mouse_report(drag(pixel.x + 40))?, "a new glyph");
  assert!(!session.mouse_report(drag(pixel.x + 41))?, "the same glyph");
  let release =
    Mouse { kind: MouseKind::Release(Button::LEFT), ..drag(pixel.x + 41) };
  session.mouse_report(release)?;
  // Leaving the window ends the hover once.
  let moved = Mouse { kind: MouseKind::Moved, ..press };
  assert!(session.mouse_report(moved)?);
  assert!(session.pointer_left());
  assert!(!session.pointer_left());
  Ok(())
}

#[test]
fn pixel_drags_draw_on_new_glyphs() -> Result<()> {
  use vpdf_model::input::{Button, Mouse, MouseKind, Spot};
  let (_directory, path) =
    common::temp_pdf(|path| common::text_pdf(path, &["Hello pointer world"]))?;
  let mut session = session(&path, screen())?;
  let app = session.app();
  let rows = 0..app.screen.rows - 1;
  let text = |hit| hit == Hit::Text;
  let (column, row) =
    crate::harness::cell_where(app, Region::Page, rows, text)?;
  let report = |kind, x| Mouse {
    kind,
    at: Spot::Pixel { x, y: row * 16 + 8 },
    modifiers: KeyModifiers::NONE,
  };
  let x = column * 8 + 4;
  // (report, pixel x) -> frame
  let cases = [
    (MouseKind::Moved, x, true),
    (MouseKind::Moved, x + 1, false),
    (MouseKind::Press(Button::LEFT), x, true),
    (MouseKind::Drag(Button::LEFT), x + 1, false),
    (MouseKind::Drag(Button::LEFT), x + 40, true),
    (MouseKind::Drag(Button::LEFT), x + 40, false),
    (MouseKind::Release(Button::LEFT), x + 40, true),
  ];
  for (step, (kind, x, frame)) in cases.into_iter().enumerate() {
    assert_eq!(session.mouse_report(report(kind, x))?, frame, "step {step}");
  }
  assert!(session.app().reader.selection.is_some(), "the drag selected");
  assert!(session.pointer_left());
  Ok(())
}

/// A covered pane counts as closed, so it adds no recent entry.
#[test]
fn covered_panes_record_no_visit() -> Result<()> {
  let (_root, path) =
    common::temp_pdf(|path| common::text_pdf(path, &["Visited text"]))?;
  let screen = common::cells(80, 24);
  let mut app = Solo::open(&path, "", false, screen)?;
  let mut state = vpdf::state::State::default();
  app.show_recent(&[])?;
  assert!(!state.remember_pane(&app));
  assert_eq!(state.recent, [] as [vpdf::state::Entry; 0]);
  app.close_panel()?;
  assert!(state.remember_pane(&app));
  assert_eq!(state.recent.len(), 1);
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_clues_use_current_focus_and_scope() -> Result<()> {
  let mut app = text_app()?;
  let show = |app: &mut App, prefix: &str| -> Result<Vec<String>> {
    common::typed(app, prefix.as_bytes())?;
    common::age_clues(app);
    app.clue_due(Instant::now());
    Ok(app.overlays.clues().shown().map_or_else(Vec::new, |list| {
      list.items().iter().map(|(key, _)| key.clone()).collect()
    }))
  };
  app.show_recent(&[])?;
  assert_eq!(show(&mut app, "y")?, ["p", "y"]);
  app.action(Input::plain(Action::Cancel))?;
  assert_eq!(show(&mut app, "m")?, [] as [String; 0]);
  assert_eq!(app.clue_wait(Instant::now()), None);
  app.action(Input::plain(Action::Cancel))?;
  app.close_panel()?;
  app.open_panel(PanelKind::Outline)?;
  let focused = show(&mut app, "g")?;
  assert!(focused.contains(&"g".into()) && !focused.contains(&"d".into()));
  app.action(Input::plain(Action::Cancel))?;
  app.action(Input::plain(Action::LocalFocus))?;
  assert!(show(&mut app, "g")?.contains(&"d".into()));
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn overlay_clues_follow_policy() -> Result<()> {
  use vpdf::pdf_text::PdfText;
  use vpdf_model::text::Position;
  use vpdf_ui::context::ContextMenu;
  use vpdf_ui::input::InputOwner;
  use vpdf_ui::preview::Preview;
  use vpdf_ui::region::{CaptureId, Region, Snapshot};
  let mut app = App::with_screen(&corpus("paper.pdf")?, "", false, screen())?;
  for owner in [
    InputOwner::Context,
    InputOwner::Peek,
    InputOwner::Region,
    InputOwner::Overview,
  ] {
    match owner {
      InputOwner::Context => {
        let menu = ContextMenu::new(Vec::new(), (0, 0), (0, 80), 23, false);
        app.overlays.open_menu(menu);
      }
      InputOwner::Peek => {
        let preview = Preview::open(
          Target::Page(PageId(0)),
          Position::default(),
          false,
          false,
          &app.layout,
          &mut PdfText(&mut app.doc),
        )?;
        app.overlays.open_preview(preview);
      }
      InputOwner::Region => {
        app.overlays.open_region(Region::open(Snapshot {
          id: CaptureId::new(1),
          page: PageId(0),
          bounds: app.doc.pages[0].bounds,
        }));
      }
      _ => app.open_overview()?,
    }
    for prefix in ["g", "z"] {
      app.keys.clear();
      common::keys(&mut app, prefix.as_bytes())?;
      common::age_clues(&mut app);
      assert_eq!(app.clue_due(Instant::now()), prefix == "g");
      assert_eq!(app.clue_wait(Instant::now()), None);
      if let Some(list) = app.overlays.clues().shown() {
        let has = |key| list.items().iter().any(|(shown, _)| shown == key);
        assert!(has("q") && has("?"));
        assert!(!has("d") && !has("E") && !has("v"));
        assert_eq!(has("F"), owner == InputOwner::Region);
        assert_eq!(has("p"), owner == InputOwner::Overview);
      }
    }
    app.keys.clear();
    app.overlays.close_menu();
    app.overlays.close_preview();
    app.overlays.close_region();
    app.close_overview();
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn contents_manual_folding() -> Result<()> {
  use vpdf_reader::motion::Motion;
  use vpdf_ui::keys::Fold;
  let mut app = solo("paper.pdf", false, common::cells(120, 50))?;
  common::outline_loaded(&mut app)?;
  app.open_panel(PanelKind::Outline)?;
  let branch = app
    .outline
    .tree()
    .nodes()
    .iter()
    .position(|node| node.descendants > 0)
    .unwrap();
  let end = app.outline.tree().nodes()[branch].end;
  let cursor = app.reader.cursor;
  app.panel.as_mut().unwrap().view.choose(branch);
  common::keys(&mut app, b"h")?;
  assert!(app.outline.closed(branch));
  common::keys(&mut app, b"j")?;
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), end);
  app.close_panel()?;
  app.open_panel(PanelKind::Outline)?;
  assert!(app.outline.closed(branch));
  app.panel.as_mut().unwrap().view.choose(branch);
  app.action(Input::plain(Action::Motion(Motion::Down)))?;
  assert_eq!(app.panel.as_ref().unwrap().view.selected(), end);
  common::keys(&mut app, b"zR")?;
  assert!(!app.outline.closed(branch));
  app.panel.as_mut().unwrap().view.choose(branch + 1);
  common::keys(&mut app, b"zM")?;
  assert_eq!(
    app.outline.tree().nodes()[app.panel.as_ref().unwrap().view.selected()]
      .parent,
    None
  );
  assert_eq!(app.reader.cursor, cursor);
  app.open_panel(PanelKind::Version)?;
  app.action(Input::plain(Action::Fold(Fold::ExpandAll)))?;
  assert!(app.outline.closed(branch));
  Ok(())
}
