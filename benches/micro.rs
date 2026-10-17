//! Single hot functions, timed in isolation, so a change is measured
//! without the noise of whole-reader runs. Arguments filter cases by name.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use mupdf::IRect;
use std::hint::black_box;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf::document::{Document, Handle};
use vpdf::layout::{Screen, page_matrix, transform};
use vpdf::text::PageId;

const SAMPLES: usize = 31;

/// Median, p10, and p90 nanoseconds per call.
fn measure(name: &str, mut call: impl FnMut()) -> Result<()> {
  // Batches of about a millisecond hide the clock's resolution.
  let mut batch = 1_u32;
  loop {
    let started = Instant::now();
    for _ in 0..batch {
      call();
    }
    if started.elapsed() >= Duration::from_millis(1) {
      break;
    }
    batch *= 2;
  }
  let mut samples: Vec<f64> = (0..SAMPLES)
    .map(|_| {
      let started = Instant::now();
      for _ in 0..batch {
        call();
      }
      started.elapsed().as_nanos() as f64 / f64::from(batch)
    })
    .collect();
  samples.sort_by(f64::total_cmp);
  let at = |share: usize| samples[(SAMPLES - 1) * share / 100];
  writeln!(
    io::stdout(),
    "{name:<28} {:>12.1} ns  p10 {:.1}  p90 {:.1}",
    at(50),
    at(10),
    at(90)
  )?;
  Ok(())
}

/// A status line row as `App::status` cleans it each frame.
fn clean() -> Result<()> {
  let status =
    " NORMAL  1:paper.pdf  2:book.pdf  3.2 Results  12/40 [xii] 31% ";
  measure("terminal::clean", || {
    black_box(vpdf::terminal::clean(black_box(status), 80));
  })
}

/// A hostile title: wide emoji, then zero-width marks.
fn clean_marks() -> Result<()> {
  let title = "❤\u{FE0F}".repeat(100) + &"\u{301}".repeat(50_000);
  measure("terminal::clean marks", || {
    black_box(vpdf::terminal::clean(black_box(&title), 160));
  })
}

/// The corpus paper on a 120x40 pane.
fn paper(text_mode: bool) -> Result<App> {
  let screen = Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  App::with_screen(&corpus("paper.pdf")?, "", text_mode, screen)
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

/// Each bare pointer move `run` skips.
fn hover() -> Result<()> {
  let mut pointer = vpdf::pointer::Pointer::default();
  measure("Pointer::hover", || black_box(&mut pointer).hover())
}

/// One terminal read: pointer motion, keys, and a graphics reply, as
/// kitty sends them while the reader scrolls.
fn decode() -> Result<()> {
  use std::collections::VecDeque;
  use vpdf::decode::Parser;
  let mut read = Vec::new();
  for column in 0..64 {
    write!(read, "\x1b[<35;{};14M", column + 1)?;
  }
  read.extend_from_slice(b"jjk\x1b[105;5u\x1b_Gi=31;OK\x1b\\");
  let mut parser = Parser::default();
  let mut events = VecDeque::new();
  measure("Parser::feed 69 events", || {
    parser.feed(black_box(&read), &mut events);
    events.clear();
  })
}

/// A warm frame of the corpus paper, scrolled 4 px from the last one,
/// with every visible tile resident; prints the frame's bytes too.
fn frame() -> Result<()> {
  let mut app = paper(false)?;
  let base = app.reader.scroll;
  for scroll in [base, base + 4.0] {
    for key in vpdf::tiles::visible(&app.layout, scroll) {
      if !app.graphics.contains(&key) {
        let tile = vpdf::tiles::render(&mut app.doc, key)?;
        app.graphics.upload(&tile, &mut io::sink())?;
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

/// Interpreting a book page, as a visible page `zc` evicted does again.
fn page_list() -> Result<()> {
  let doc = Document::open(&corpus("book.pdf")?, "")?;
  measure("display list book p100", || {
    let page = doc.native.load_page(99);
    drop(black_box(page.and_then(|page| page.to_display_list(true))));
  })
}

/// `zc` sampling 24 pages of a book for its margins.
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
  let mut app = App::with_screen(&corpus("book.pdf")?, "", false, screen)?;
  app.command("page 100")?;
  measure("App::toggle_crop on, off", || {
    drop(black_box(app.toggle_crop()));
    drop(black_box(app.toggle_crop()));
  })
}

/// Every page's bounds and label, the outline, and the title.
fn document_open() -> Result<()> {
  let path = corpus("book.pdf")?;
  measure("Document::open book", || {
    drop(black_box(Document::open(&path, "")));
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
    app.layout.reflow(&app.doc.pages);
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

type Case = fn() -> Result<()>;

const CASES: &[(&str, Case)] = &[
  ("clean", clean),
  ("clean marks", clean_marks),
  ("text_canvas", text_canvas),
  ("route", route),
  ("hover", hover),
  ("decode", decode),
  ("scan whole", scan_whole),
  ("page list", page_list),
  ("crop margins", crop_margins),
  ("crop toggle", crop_toggle),
  ("document open", document_open),
  ("handle open", handle_open),
  ("handle bounds", handle_bounds),
  ("projection", projection),
  ("menu_rows", menu_rows),
  ("frame", frame),
];

fn main() -> Result<()> {
  // Cargo passes `--bench`; other arguments name cases.
  let filters: Vec<String> =
    std::env::args().skip(1).filter(|arg| !arg.starts_with("--")).collect();
  let mut failed = 0;
  for (name, case) in CASES {
    if filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())) {
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
