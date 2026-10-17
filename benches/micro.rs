//! Single hot functions, timed in isolation, so a change is measured
//! without the noise of whole-reader runs. Arguments filter cases by name.
use std::hint::black_box;
use std::io::{self, Write};
use std::time::{Duration, Instant};
use vpdf::Result;

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

/// Text-mode frame; needs `VPDF_CORPUS`.
fn text_canvas() -> Result<()> {
  let Some(root) = std::env::var_os("VPDF_CORPUS") else {
    writeln!(io::stdout(), "text_canvas: VPDF_CORPUS unset, skipped")?;
    return Ok(());
  };
  let screen = vpdf::layout::Screen {
    columns: 120,
    rows: 40,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let path = std::path::Path::new(&root).join("paper.pdf");
  let mut app = vpdf::app::App::with_screen(&path, "", true, screen)?;
  // Main's centering draws this same view.
  let (rows, columns) = (usize::from(screen.rows), usize::from(screen.columns));
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

type Case = fn() -> Result<()>;

const CASES: &[(&str, Case)] = &[
  ("clean", clean),
  ("clean marks", clean_marks),
  ("text_canvas", text_canvas),
];

fn main() -> Result<()> {
  // Cargo passes `--bench`; other arguments name cases.
  let filters: Vec<String> =
    std::env::args().skip(1).filter(|arg| !arg.starts_with("--")).collect();
  for (name, case) in CASES {
    if filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str())) {
      case()?;
    }
  }
  Ok(())
}
