//! Single hot functions, timed in isolation, so a change is measured
//! without the noise of whole-reader runs. Arguments filter cases by name.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mupdf::IRect;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::document::{Document, Handle};
use vpdf::layout::{Screen, page_matrix, transform};
use vpdf::pdf_text::PdfText;
use vpdf::solo::Solo;
use vpdf::text::PageId;

mod support;
use support::measure;

/// A 120x40 pane.
const PANE: Screen = Screen {
  columns: 120,
  rows: 40,
  cell_width: 8,
  cell_height: 16,
  origin: (0, 0),
};

/// The corpus paper on a 120x40 pane.
fn paper(text_mode: bool) -> Result<Solo> {
  Solo::open(&corpus("paper.pdf")?, "", text_mode, PANE)
}

/// Text-mode frame.
fn text_canvas() -> Result<()> {
  let mut app = paper(true)?;
  // Main's centering draws this same view.
  let (rows, columns) =
    (usize::from(app.screen.rows), usize::from(app.screen.columns));
  let page = app.reader.cursor.page;
  let text = app.doc.text(page)?;
  let line = text
    .lines
    .iter()
    .take((rows - 1) / 2)
    .position(|l| l.glyphs.len() >= columns / 2)
    .ok_or_else(|| anyhow::anyhow!("no long line near the top"))?;
  let glyph = columns / 2 - 1;
  app.reader.cursor = vpdf::text::Position { page, line, glyph };
  let mut out = Vec::new();
  app.text_canvas(&mut out)?;
  measure("App::text_canvas", || {
    out.clear();
    app.text_canvas(&mut out).expect("cached page text");
    black_box(&out);
  })
}

fn cursorline() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.reader.cursor.line = 3;
    app.reader.visual(vpdf_reader::reader::SelectionKind::Character);
    overlay_case(&mut app, &format!("cursorline text={text_mode}"))?;
  }
  Ok(())
}

fn landing() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.go_to(vpdf::text::Position { line: 3, ..Default::default() }, true)?;
    overlay_case(&mut app, &format!("landing text={text_mode}"))?;
  }
  Ok(())
}

fn overlay_case(app: &mut Solo, name: &str) -> Result<()> {
  let screen = app.layout.screen;
  let mut out = Vec::new();
  let mut draw = |out: &mut Vec<u8>| {
    out.clear();
    if app.text_mode {
      app.text_canvas(out).expect("cached text");
    } else {
      app.graphics.begin(out).expect("begin marks");
      {
        let (app, graphics) = app.parts();
        app.marks(graphics, screen, out).expect("cached marks");
      }
      app.graphics.end(out).expect("end marks");
    }
  };
  draw(&mut out);
  measure(name, || {
    draw(&mut out);
    black_box(&out);
  })
}

/// Hovered citation: label, then tint.
fn link_hover() -> Result<()> {
  use vpdf::pointer::{Hit, Hover, Region};
  let on = |pane, page, index| {
    let hit = Hit::Link(page, index);
    let beneath = Some((page, index));
    Some(Hover { pane, region: Region::Page, hit, beneath })
  };
  let mut app = paper(false)?;
  app.command("2")?;
  let page = app.navigation_position().page;
  anyhow::ensure!(app.doc.links(page)?.len() >= 2, "citations on page 2");
  let mut index = 0;
  measure("App::point", || {
    index = 1 - index;
    {
      let (app, _) = app.parts();
      app.point(on(app.id, page, index));
    }
    black_box(app.hover);
  })?;
  measure("App::hover_label", || {
    black_box(app.hover_label());
  })?;
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.command("2")?;
    overlay_case(&mut app, &format!("no link text={text_mode}"))?;
    {
      let (app, _) = app.parts();
      app.point(on(app.id, page, 0));
    }
    anyhow::ensure!(app.hover_link().is_some(), "a hovered citation");
    overlay_case(&mut app, &format!("link tint text={text_mode}"))?;
  }
  // A web address wrapped over two lines.
  let mut app = Solo::open(&corpus("book.pdf")?, "", false, PANE)?;
  app.command("4")?;
  let page = PageId(3);
  let index = app.doc.links(page)?.iter().position(|link| {
    matches!(&link.target, vpdf::document::Target::Uri(uri)
      if uri.contains("greenteapress"))
  });
  let index = index.ok_or_else(|| anyhow::anyhow!("an address on p. 4"))?;
  {
    let (app, _) = app.parts();
    app.point(on(app.id, page, index));
  }
  overlay_case(&mut app, "link tint address")
}

/// An open context menu each frame.
fn context_draw() -> Result<()> {
  use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let right = |kind| MouseEvent {
    kind,
    column: 40,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  session.mouse(right(MouseEventKind::Down(MouseButton::Right)))?;
  anyhow::ensure!(session.app().context_menu.is_some(), "an open menu");
  let tab = &mut session.tabs[session.active];
  let (app, graphics) = (&mut tab.panes[tab.focus], &mut session.graphics);
  let mut out = Vec::new();
  measure("App::draw_context", || {
    out.clear();
    graphics.begin(&mut out).expect("begin menu frame");
    app.draw_context(graphics, &mut out).expect("menu rows");
    graphics.end(&mut out).expect("menu placements");
    black_box(&out);
  })
}

/// Visible citation labels on the corpus paper.
fn draw_hints() -> Result<()> {
  let mut app = paper(false)?;
  app.command("2")?;
  app.start_hints(false)?;
  let mut out = Vec::new();
  {
    let (app, graphics) = app.parts();
    app.draw_hints(graphics, &mut out)?;
  }
  measure("App::draw_hints", || {
    out.clear();
    app.graphics.begin(&mut out).expect("begin hint frame");
    app
      .app
      .draw_hints(&mut app.graphics, &mut out)
      .expect("visible citation labels");
    app.graphics.end(&mut out).expect("hint placements");
    black_box(&out);
  })
}

/// Rows each frame clips page graphics to while clues show.
fn menu_rows() -> Result<()> {
  let mut app = paper(false)?;
  let prefix = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
  app.key(prefix, &mut vpdf::prompt::History::default())?;
  app.awaiting = Instant::now().checked_sub(Duration::from_secs(1));
  let _ = app.clue_due();
  assert!(app.clues.is_some());
  measure("App::menu_rows clues", || {
    black_box(app.menu_rows());
  })
}

/// A drag across a two-pane split.
fn route() -> Result<()> {
  use crossterm::event::{
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
  };
  use vpdf::pointer::{Capture, Pane, Region};
  use vpdf::split::Rect;
  let pane = |id, column, panel| Pane {
    id,
    rect: Rect { column, row: 0, columns: 60, rows: 40 },
    panel,
    overlay: 3,
    overview: false,
    preview: None,
    menu: None,
    actions: None,
  };
  let panes = [pane(1, 0, 20), pane(2, 61, 0)];
  let capture =
    Some(Capture { pane: 1, button: MouseButton::Left, region: Region::Page });
  let drag = MouseEvent {
    kind: MouseEventKind::Drag(MouseButton::Left),
    column: 90,
    row: 39,
    modifiers: KeyModifiers::NONE,
  };
  measure("pointer::route", || {
    black_box(vpdf::pointer::route(
      black_box(&panes),
      1,
      black_box(capture),
      black_box(drag),
    ));
  })
}

/// Motion along a citation-dense row.
fn hover() -> Result<()> {
  use crossterm::event::{MouseEvent, MouseEventKind};
  use vpdf::pointer::Region;
  let motion = |column| MouseEvent {
    kind: MouseEventKind::Moved,
    column,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let mut column = 0;
  measure("Session::mouse motion", || {
    column = (column + 1) % 120;
    black_box(session.mouse(black_box(motion(column))).expect("hover"));
  })?;
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.command("2")?;
    app.draw(&mut Vec::new())?;
    let mut column = 0;
    measure(&format!("App::hovered text={text_mode}"), || {
      column = (column + 1) % 120;
      let hit = app.hovered(Region::Page, black_box(column), 20, None);
      black_box(hit.expect("hit test"));
    })?;
  }
  Ok(())
}

/// Wheel turns over the page: scroll, Ctrl zoom, and Shift pan.
fn wheel() -> Result<()> {
  use crossterm::event::{MouseEvent, MouseEventKind};
  let turn =
    |kind, modifiers| MouseEvent { kind, column: 60, row: 20, modifiers };
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let cases = [
    ("scroll", KeyModifiers::NONE),
    ("ctrl zoom", KeyModifiers::CONTROL),
    ("shift pan", KeyModifiers::SHIFT),
  ];
  for (name, modifiers) in cases {
    if modifiers == KeyModifiers::SHIFT {
      session.app().layout.zoom = 2.0;
      session.app().relayout()?;
    }
    measure(&format!("Session::mouse wheel {name}"), || {
      for kind in [MouseEventKind::ScrollUp, MouseEventKind::ScrollDown] {
        black_box(
          session.mouse(black_box(turn(kind, modifiers))).expect("wheel"),
        );
      }
    })?;
  }
  Ok(())
}

/// Row drags: plain, by word, edge.
fn drag() -> Result<()> {
  use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
  let event = |kind, column| MouseEvent {
    kind,
    column,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  for clicks in [1, 2] {
    let mut session = vpdf::session::Session::new(None);
    session.open(&corpus("paper.pdf")?, true, PANE)?;
    session.app().command("2")?;
    let press = MouseEventKind::Down(MouseButton::Left);
    for _ in 1..clicks {
      session.mouse(event(press, 40))?;
      session.mouse(event(MouseEventKind::Up(MouseButton::Left), 40))?;
    }
    session.mouse(event(press, 40))?;
    let mut column = 40;
    measure(&format!("Session::mouse drag clicks={clicks}"), || {
      column = if column == 80 { 50 } else { 80 };
      let drag = event(MouseEventKind::Drag(MouseButton::Left), column);
      black_box(session.mouse(black_box(drag)).expect("drag"));
    })?;
  }
  // Middle drags pan a zoomed page.
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  session.app().layout.zoom = 3.0;
  session.app().relayout()?;
  session.mouse(event(MouseEventKind::Down(MouseButton::Middle), 60))?;
  let mut column = 60;
  measure("Session::mouse drag middle", || {
    column = if column == 70 { 50 } else { 70 };
    let drag = event(MouseEventKind::Drag(MouseButton::Middle), column);
    black_box(session.mouse(black_box(drag)).expect("pan"));
  })?;
  // Held at the status row: no scroll.
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("2")?;
  let base = session.app().reader.scroll;
  session.mouse(event(MouseEventKind::Down(MouseButton::Left), 40))?;
  let mut column = 40;
  measure("Session::mouse drag held edge", || {
    column = if column == 80 { 50 } else { 80 };
    let drag = MouseEvent {
      row: 39,
      ..event(MouseEventKind::Drag(MouseButton::Left), column)
    };
    black_box(session.mouse(black_box(drag)).expect("drag"));
  })?;
  // A step scrolls a row and extends.
  let mut now = session.autoscroll_due().expect("a held edge drag");
  measure("Session::autoscroll step", || {
    now += Duration::from_millis(50);
    session.app().reader.scroll = base;
    black_box(session.autoscroll(black_box(now)));
  })
}

/// A warm frame of the corpus paper, scrolled 4 px from the last one,
/// with every visible tile resident; prints the frame's bytes too.
fn frame() -> Result<()> {
  let mut app = paper(false)?;
  let base = app.reader.scroll;
  for scroll in [base, base + 4.0] {
    for key in vpdf::tiles::visible(&app.layout, scroll) {
      if !app.graphics.contains(&(app.id, key)) {
        let tile = vpdf::tiles::render(&mut app.doc, key)?;
        app.graphics.upload(app.id, &tile, &mut io::sink())?;
      }
    }
  }
  let mut out = Vec::with_capacity(64 * 1024);
  let mut step = 0_u32;
  let mut draw = |out: &mut Vec<u8>| {
    app.reader.scroll = base + f32::from(u8::from(step % 2 == 1)) * 4.0;
    step += 1;
    out.clear();
    app.draw(out).expect("resident tiles");
  };
  draw(&mut out);
  draw(&mut out);
  writeln!(io::stdout(), "App::draw bytes {}", out.len())?;
  measure("App::draw scroll", || {
    draw(&mut out);
    black_box(&out);
  })
}

/// Two panes, the pointer resting on one.
fn split_frame() -> Result<()> {
  use crossterm::event::{MouseEvent, MouseEventKind};
  use vpdf::trace::Drawable;
  let mut session = vpdf::session::Session::new(None);
  session.open(&corpus("paper.pdf")?, true, PANE)?;
  session.app().command("vsplit")?;
  session.settle()?;
  let rest = |column| MouseEvent {
    kind: MouseEventKind::Moved,
    column,
    row: 20,
    modifiers: KeyModifiers::NONE,
  };
  session.mouse(rest(30))?;
  let mut out = Vec::with_capacity(64 * 1024);
  session.draw_frame(&mut out)?;
  measure("Session::draw_frame split", || {
    out.clear();
    session.draw_frame(&mut out).expect("warm frame");
    black_box(&out);
  })
}

fn viewport() -> Result<()> {
  for text_mode in [false, true] {
    let mut app = paper(text_mode)?;
    app.command("4")?;
    app.ensure_cursor()?;
    let cursor = app.reader.cursor;
    let scroll = app.reader.scroll;
    measure(&format!("App::scroll text={text_mode}"), || {
      app.reader.cursor = cursor;
      app.reader.scroll = scroll;
      app.animation = None;
      app.scroll(16.0).expect("scroll");
      app.scroll(-16.0).expect("scroll");
    })?;
    let mut narrow = false;
    measure(&format!("App::relayout text={text_mode}"), || {
      app.reader.cursor = cursor;
      app.reader.scroll = scroll;
      narrow = !narrow;
      app.screen.columns = if narrow { 100 } else { 120 };
      app.relayout().expect("layout");
    })?;
  }
  Ok(())
}

/// A file `mise corpus` fetches; `mise bench:micro` sets `VPDF_CORPUS`.
fn corpus(name: &str) -> Result<PathBuf> {
  let root = std::env::var_os("VPDF_CORPUS").ok_or_else(|| {
    anyhow::anyhow!("VPDF_CORPUS unset; run mise bench:micro")
  })?;
  let path = PathBuf::from(root).join(name);
  anyhow::ensure!(path.is_file(), "{name} missing; run mise corpus");
  Ok(path)
}

/// One whole draw of a scan 1913 pixels wide: the work a band of
/// another scan in between repeats.
fn scan_whole() -> Result<()> {
  let mut doc = Document::open(&corpus("ocr.pdf")?, "")?;
  let bounds = doc.pages[0].bounds;
  let matrix = page_matrix(bounds, 900, 0);
  let rect = transform(bounds, &matrix);
  let area = IRect::new(0, 0, rect.x1.ceil() as i32, rect.y1.ceil() as i32);
  let list = doc.display(PageId(0))?;
  measure("tiles::draw whole scan", || {
    drop(black_box(vpdf::tiles::draw(&list, &matrix, area)));
  })
}

/// Every page record of the corpus book, read back from its pack.
fn pack_read() -> Result<()> {
  use vpdf_pdf::content_cache::{Pack, Records, alias, save};
  let path = corpus("book.pdf")?;
  let mut doc = Document::open(&path, "")?;
  let pages = doc.pages.len();
  let mut records = Records::new(pages);
  for page in (0..pages).map(PageId) {
    let text = doc.text(page)?;
    records.add(page, &vpdf::search::Index::new(page, &text))?;
  }
  let root = tempfile::tempdir()?;
  let alias = alias(&mut std::fs::File::open(&path)?)?;
  save(root.path(), &alias, &records, None)?;
  let pack = Pack::open(root.path(), &alias)
    .ok_or_else(|| anyhow::anyhow!("pack missing"))?;
  measure(&format!("Pack::index {pages} pages"), || {
    for page in (0..pages).map(PageId) {
      drop(black_box(pack.index(page)));
    }
  })?;
  let all: Vec<PageId> = (0..pages).map(PageId).collect();
  measure(&format!("Pack::indexes {pages} pages"), || {
    drop(black_box(pack.indexes(
      &all,
      usize::MAX,
      vpdf_pdf::jobs::Config::default().helpers,
    )));
  })
}

/// A page's first hover: text and links.
fn cold_page() -> Result<()> {
  for (name, page) in [("paper.pdf", 1), ("book.pdf", 99)] {
    let mut doc = Document::open(&corpus(name)?, "")?;
    measure(&format!("cold text {name}"), || {
      doc.trim();
      drop(black_box(doc.text(PageId(page)).expect("text")));
    })?;
    measure(&format!("cold text+links {name}"), || {
      doc.trim();
      drop(black_box(doc.links(PageId(page)).expect("links")));
    })?;
    // Seeding drops the page's links and keeps its text.
    let text = doc.text(PageId(page))?;
    measure(&format!("links, text read {name}"), || {
      doc.seed(PageId(page), std::sync::Arc::clone(&text));
      drop(black_box(doc.links(PageId(page)).expect("links")));
    })?;
  }
  Ok(())
}

/// Interpreting a book page, as a visible page `zc` evicted does again.
fn page_list() -> Result<()> {
  let doc = Document::open(&corpus("book.pdf")?, "")?;
  measure("display list book p100", || {
    let page = doc.native.load_page(99);
    drop(black_box(page.and_then(|page| page.to_display_list(true))));
  })
}

/// Uncached margin detection across a book.
fn crop_margins() -> Result<()> {
  let mut doc = Document::open(&corpus("book.pdf")?, "")?;
  vpdf::crop::margins(&mut doc)?;
  measure("crop::margins book", || {
    drop(black_box(vpdf::crop::margins(&mut doc)));
  })
}

/// `zc` on, then off, at page 100 of a book.
fn crop_toggle() -> Result<()> {
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = Solo::open(&corpus("book.pdf")?, "", false, screen)?;
  app.command("100")?;
  app.toggle_crop()?;
  let started = Instant::now();
  while app.crop_job.is_some() {
    app.crop_updates()?;
    anyhow::ensure!(
      started.elapsed() < Duration::from_secs(30),
      "Crop timeout"
    );
    std::thread::sleep(Duration::from_millis(1));
  }
  app.toggle_crop()?;
  measure("App::toggle_crop on, off", || {
    black_box(app.toggle_crop()).expect("crop toggle");
    black_box(app.toggle_crop()).expect("crop toggle");
  })
}

/// Every page's bounds and the title.
fn document_open() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Document::open book", || {
    drop(black_box(Document::open(&path, "")));
  })
}

/// Opening plus a label lookup that matches nothing, which reads every
/// page's label: the most a later command or outline panel pays once.
fn label_scan() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Document::open + label miss", || {
    let doc = Document::open(&path, "").expect("corpus PDF");
    black_box(doc.page_number(black_box("missing")));
  })
}

/// The native document and page count, as workers open it.
fn handle_open() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Handle::open book", || {
    drop(black_box(Handle::open(&path, "")));
  })
}

/// One page's bounds, which the render coordinator reads per band.
fn handle_bounds() -> Result<()> {
  let doc = Handle::open(&corpus("book.pdf")?, "")?;
  measure("Handle::bounds book p200", || {
    drop(black_box(doc.bounds(PageId(199))));
  })
}

/// Page-space bounds projected for highlights and hints.
fn projection() -> Result<()> {
  let mut app = paper(false)?;
  let bounds = app.doc.text(PageId(0))?.lines[0].glyphs[0].bounds;
  for zoom in [0.5, 2.0] {
    app.layout.zoom = zoom;
    app.layout.pan = 96.0;
    {
      let (app, _) = app.parts();
      app.layout.reflow(app.doc.pages.iter().map(|page| page.bounds));
    }
    measure(&format!("Layout::project zoom {zoom}"), || {
      black_box(app.layout.project(
        black_box(PageId(0)),
        black_box(bounds),
        27.0,
      ));
    })?;
    measure(&format!("tiles::visible zoom {zoom}"), || {
      black_box(vpdf::tiles::visible(black_box(&app.layout), 27.0));
    })?;
  }
  Ok(())
}

fn key_prefixes() -> Result<()> {
  for (name, input) in
    [("Keys::feed navigation", "12j2gg"), ("Keys::feed sections", "2[[3]]")]
  {
    let events: Vec<_> = input
      .chars()
      .map(|ch| KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
      .collect();
    measure(name, || {
      let mut keys = vpdf::keys::Keys::default();
      for event in &events {
        black_box(keys.feed(black_box(*event), vpdf::keys::Grammar::Objects));
      }
    })?;
  }
  Ok(())
}

fn routing_key() -> Result<()> {
  let mut app = paper(false)?;
  let mut history = vpdf::prompt::History::default();
  let key = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
  measure("App::key reading", || {
    app.reader.scroll = 0.0;
    app.animation = None;
    app.key(black_box(key), &mut history).expect("reading key");
  })
}

fn cursor_key() -> Result<()> {
  let mut app = paper(false)?;
  let mut history = vpdf::prompt::History::default();
  app
    .key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE), &mut history)?;
  let cursor = app.reader.cursor;
  let key = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE);
  measure("App::key Cursor glyph", || {
    app.reader.cursor = cursor;
    app.key(black_box(key), &mut history).expect("glyph key");
  })
}

fn preview_idle() -> Result<()> {
  let mut app = paper(false)?;
  measure("App::draw_preview absent", || {
    let Solo { app, graphics } = black_box(&mut app);
    app.draw_preview(graphics, &mut io::sink()).expect("no preview");
  })
}

fn preview_scene(text: bool) -> Result<Solo> {
  use vpdf::document::Target;
  let mut app = paper(text)?;
  let link = app
    .doc
    .links(PageId(1))?
    .iter()
    .find(|link| !matches!(link.target, Target::Uri(_)))
    .cloned()
    .expect("citation");
  let position = app.doc.text(PageId(1))?.nearest(
    PageId(1),
    f32::midpoint(link.first().x0, link.first().x1),
    f32::midpoint(link.first().y0, link.first().y1),
  );
  app.go_to(position, true)?;
  app.preview_link()?;
  Ok(app)
}

fn preview_draw() -> Result<()> {
  for text in [false, true] {
    let mut app = preview_scene(text)?;
    let mut out = Vec::new();
    {
      let (app, graphics) = app.parts();
      app.draw_preview(graphics, &mut out)?;
    }
    app.graphics.end(&mut out)?;
    measure(&format!("preview draw text={text}"), || {
      out.clear();
      let Solo { app, graphics } = black_box(&mut app);
      graphics.begin(&mut out).expect("begin preview");
      app.draw_preview(graphics, &mut out).expect("warm preview");
      graphics.end(&mut out).expect("end preview");
      black_box(&out);
    })?;
    measure(&format!("preview stray check text={text}"), || {
      black_box(&mut app).drop_stray_preview();
    })?;
  }
  Ok(())
}

fn preview_live() -> Result<()> {
  preview_draw()?;
  for text in [false, true] {
    let mut app = preview_scene(text)?;
    let mut history = vpdf::prompt::History::default();
    let key = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE);
    measure(&format!("preview prefix text={text}"), || {
      app.keys.clear();
      app.key(black_box(key), &mut history).expect("preview prefix");
    })?;
    app.keys.clear();
    measure(&format!("preview scroll text={text}"), || {
      black_box(&mut app).scroll_preview(black_box(1.0)).expect("preview down");
      black_box(&mut app).scroll_preview(black_box(-1.0)).expect("preview up");
    })?;
    // Warm: pages stay cached between runs.
    measure(&format!("preview reopen cross text={text}"), || {
      app.peek = None;
      app.preview_link().expect("preview");
      black_box(&mut app).scroll_preview(black_box(40.0)).expect("down");
    })?;
  }
  Ok(())
}

fn routing_pointer() -> Result<()> {
  use crossterm::event::{MouseEvent, MouseEventKind};
  let app = paper(false)?;
  let mut session = vpdf::session::Session::new(None);
  session.open(&app.path, true, app.screen)?;
  let mouse = MouseEvent {
    kind: MouseEventKind::ScrollDown,
    column: 80,
    row: 12,
    modifiers: KeyModifiers::NONE,
  };
  measure("Session::mouse wheel", || {
    session.app().reader.scroll = 0.0;
    session.app().animation = None;
    session.mouse(black_box(mouse)).expect("reading wheel");
  })
}

fn routing_status() -> Result<()> {
  let mut app = paper(false)?;
  app.open_panel(vpdf::keys::PanelKind::Help)?;
  let mut out = Vec::new();
  measure("App::status owner", || {
    out.clear();
    app.status(&mut out, 39).expect("owner badge");
    black_box(&out);
  })
}

fn routing_panel() -> Result<()> {
  let mut app = paper(false)?;
  app.open_panel(vpdf::keys::PanelKind::Help)?;
  let panel = app.panel.as_mut().expect("help panel");
  let mut out = Vec::new();
  measure("Panel::list", || {
    out.clear();
    panel
      .list(&mut out, vpdf_terminal::terminal::At::default(), 50, 40)
      .expect("panel rows");
    black_box(&out);
  })
}

fn routing_outline() -> Result<()> {
  let mut app = paper(false)?;
  app.open_panel(vpdf::keys::PanelKind::Outline)?;
  app.panel.as_mut().expect("outline").focused = false;
  measure("App::track_outline", || {
    app.track_outline();
    black_box(app.current_section);
  })?;
  // Landed views match headings by glyph.
  app.doc.load_outline()?;
  let target = app.doc.outline[5].target.clone();
  app.follow(&target)?;
  measure("App::track_outline landed", || {
    app.track_outline();
    black_box(app.current_section);
  })
}

fn routing_panel_press() -> Result<()> {
  use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
  let mut app = paper(false)?;
  app.open_panel(vpdf::keys::PanelKind::Help)?;
  let mouse = MouseEvent {
    kind: MouseEventKind::Down(MouseButton::Left),
    column: 2,
    row: 2,
    modifiers: KeyModifiers::NONE,
  };
  measure("App::panel_mouse press", || {
    app.panel_mouse(black_box(mouse), 1).expect("help row press");
  })
}

fn transitions() -> Result<()> {
  use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
  let app = paper(false)?;
  let mut session = vpdf::session::Session::new(None);
  session.open(&app.path, true, app.screen)?;
  session.app().command("vsplit")?;
  session.settle()?;
  measure("Session::mouse pane press", || {
    let column = if session.app().at.column == 0 { 90 } else { 10 };
    let mouse = MouseEvent {
      kind: MouseEventKind::Down(MouseButton::Left),
      column,
      row: 39,
      modifiers: KeyModifiers::NONE,
    };
    session.mouse(black_box(mouse)).expect("pane press");
  })?;
  session.open(&corpus("book.pdf")?, true, app.screen)?;
  measure("Session::activate", || {
    session.activate(black_box(1 - session.active)).expect("tab activation");
  })?;
  let mut app = paper(false)?;
  measure("App::blur", || app.blur().expect("pane blur"))
}

/// `yy` joining a prose page whole, and three of its lines.
fn reflow() -> Result<()> {
  let mut doc = Document::open(&corpus("paper.pdf")?, "")?;
  let lines = doc.text(PageId(1))?.lines.len();
  for (name, count) in
    [("yy paper p2 whole", lines), ("yy paper p2 3 lines", 3)]
  {
    measure(name, || {
      let mut reader = vpdf_reader::reader::Reader::default();
      reader.cursor.page = PageId(1);
      reader.yank(1, '"', &mut PdfText(&mut doc)).expect("operator");
      reader.yank(count, '"', &mut PdfText(&mut doc)).expect("paper lines");
      black_box(reader.pending_yank);
    })?;
  }
  Ok(())
}

fn lexical_objects() -> Result<()> {
  let mut app = paper(false)?;
  let text = app.doc.text(PageId(0))?;
  let line = text.lines.iter().position(|line| line.glyphs.len() > 15).unwrap();
  let cursor = vpdf::text::Position { page: PageId(0), line, glyph: 4 };
  measure("Reader::object paper", || {
    let (app, _) = app.parts();
    app.reader.cursor = cursor;
    app.reader.object(false, &mut PdfText(&mut app.doc)).expect("paper word");
    black_box(app.reader.selection);
  })?;
  let letters: Vec<_> = text.lines[line]
    .glyphs
    .iter()
    .filter(|g| g.ch.is_alphabetic())
    .take(8)
    .map(|g| KeyEvent::new(KeyCode::Char(g.ch), KeyModifiers::NONE))
    .collect();
  let mut keys = vpdf::keys::Keys::default();
  measure("Keys::feed objects paper", || {
    keys.clear();
    for ch in "3iw".chars() {
      black_box(keys.feed(
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE),
        vpdf::keys::Grammar::Objects,
      ));
    }
    for key in &letters {
      black_box(keys.feed(*key, vpdf::keys::Grammar::Objects));
    }
  })
}

fn figures() -> Result<()> {
  let mut app = paper(false)?;
  let page = PageId(2);
  let figures = app.doc.figures(page)?;
  let figure = figures
    .iter()
    .find(|figure| figure.caption.starts_with("Figure 1"))
    .ok_or_else(|| anyhow::anyhow!("Figure 1 missing"))?;
  measure("Document::figures warm", || {
    black_box(app.doc.figures(black_box(page)).expect("cached figures"));
  })?;
  let text = app.doc.text(page)?;
  measure("region::region_text", || {
    black_box(vpdf::region::region_text(black_box(&text), figure.bounds));
  })?;
  app.select_region(page, figure.bounds)?;
  app.reader.scroll = app.layout.project(page, figure.bounds, 0.0).y0;
  let mut out = Vec::new();
  let mut draw = |out: &mut Vec<u8>| {
    out.clear();
    app.graphics.begin(out).expect("begin region");
    {
      let (app, graphics) = app.parts();
      app.draw_region(graphics, out).expect("region bar");
    }
    app.graphics.end(out).expect("end region");
  };
  draw(&mut out);
  measure("App::draw_region warm", || {
    draw(&mut out);
    black_box(&out);
  })
}

type Case = fn() -> Result<()>;

const CASES: &[(&str, Case)] = &[
  ("figures", figures),
  ("lexical objects", lexical_objects),
  ("reflow", reflow),
  ("text_canvas", text_canvas),
  ("cursorline", cursorline),
  ("landing", landing),
  ("draw_hints", draw_hints),
  ("route", route),
  ("hover", hover),
  ("wheel", wheel),
  ("cold page", cold_page),
  ("pack read", pack_read),
  ("key prefixes", key_prefixes),
  ("scan whole", scan_whole),
  ("page list", page_list),
  ("crop margins", crop_margins),
  ("crop toggle", crop_toggle),
  ("document open", document_open),
  ("label scan", label_scan),
  ("handle open", handle_open),
  ("handle bounds", handle_bounds),
  ("projection", projection),
  ("menu_rows", menu_rows),
  ("link hover", link_hover),
  ("drag", drag),
  ("context draw", context_draw),
  ("frame", frame),
  ("viewport", viewport),
  ("transitions", transitions),
  ("routing key", routing_key),
  ("cursor key", cursor_key),
  ("preview absent", preview_idle),
  ("preview live", preview_live),
  ("preview draw", preview_draw),
  ("split frame", split_frame),
  ("routing pointer", routing_pointer),
  ("routing status", routing_status),
  ("routing panel", routing_panel),
  ("routing fixes outline", routing_outline),
  ("routing fixes panel press", routing_panel_press),
];

fn main() -> Result<()> {
  let mut failed = 0;
  for (name, case) in CASES {
    if support::selected(name) {
      // One failing case leaves the others' timings.
      if let Err(error) = case() {
        writeln!(io::stderr(), "{name}: {error:#}")?;
        failed += 1;
      }
    }
  }
  anyhow::ensure!(failed == 0, "{failed} cases failed");
  Ok(())
}
