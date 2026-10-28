pub use crate::fixtures::screen::cells;
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use vpdf::Result;
use vpdf::app::App;
use vpdf::solo::Solo;
use vpdf_model::input::Event;
use vpdf_terminal::decode::Parser;
use vpdf_ui::layout::Screen;
use vpdf_ui::prompt::History;

pub fn preview_frame(app: &mut Solo) -> Result<String> {
  let mut out = Vec::new();
  app.graphics.begin(&mut out)?;
  {
    let (app, graphics) = app.parts();
    vpdf::presentation::try_draw(&mut out, Some(graphics), |s| {
      app.scene_preview(s)
    })?;
  }
  app.graphics.end(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Events decoded from `bytes`; a trailing ESC is the Esc key.
pub fn events(bytes: &[u8]) -> VecDeque<Event> {
  let mut events = VecDeque::new();
  let mut parser = Parser::default();
  parser.feed(bytes, &mut events);
  parser.end(&mut events);
  let events: VecDeque<_> =
    events.into_iter().filter_map(crate::input::event).collect();
  assert!(bytes.is_empty() || !events.is_empty(), "undecoded input bytes");
  events
}

/// Feeds key `bytes` to `app`.
pub fn keys(app: &mut App, bytes: &[u8]) -> Result<()> {
  fed(app, bytes, |_| {})
}

/// `keys` with the input loop's clue restarts.
pub fn typed(app: &mut App, bytes: &[u8]) -> Result<()> {
  fed(app, bytes, App::after_key)
}

fn fed(app: &mut App, bytes: &[u8], after: fn(&mut App)) -> Result<()> {
  for event in events(bytes) {
    let Event::Key(key) = event else { panic!("Expected key") };
    let result = app.key(key, &mut History::default());
    after(app);
    result?;
  }
  Ok(())
}

/// Polls until done or `limit` seconds pass.
pub fn wait_until(
  limit: u64,
  label: &str,
  mut poll: impl FnMut() -> Result<bool>,
) -> Result<()> {
  let started = Instant::now();
  while !poll()? {
    let late = started.elapsed() > Duration::from_secs(limit);
    anyhow::ensure!(!late, "{label} timed out");
    std::thread::sleep(Duration::from_millis(2));
  }
  Ok(())
}

/// Backdates the clue timer one second, past its delay.
pub fn age_clues(app: &mut App) {
  let ago = Instant::now().checked_sub(Duration::from_secs(1));
  app.overlays.clues_mut().restart(ago);
}

/// Starts the outline the app loads off the input thread and waits.
pub fn outline_loaded(app: &mut App) -> Result<()> {
  app.want_outline();
  wait_until(10, "Outline", || {
    app.outline_updates()?;
    Ok(!app.outline_pending())
  })
}

pub fn annotations_loaded(app: &mut App) -> Result<()> {
  wait_until(10, "Annotations", || {
    app.sidebar_updates()?;
    Ok(app.panel.as_ref().unwrap().view.note() != "loading")
  })
}

pub fn search_loaded(app: &mut App) -> Result<()> {
  wait_until(15, "Search", || {
    app.search_updates()?;
    Ok(matches!(app.search.phase(), vpdf_ui::find::Phase::Complete))
  })
}

/// A temporary PDF that `write` makes.
pub fn temp_pdf(
  write: impl FnOnce(&Path) -> Result<()>,
) -> Result<(TempDir, PathBuf)> {
  let directory = tempfile::tempdir()?;
  let path = directory.path().join("test.pdf");
  write(&path)?;
  Ok((directory, path))
}

/// A pane on `temp_pdf`.
pub fn temp_solo(
  write: impl FnOnce(&Path) -> Result<()>,
  text_mode: bool,
  screen: Screen,
) -> Result<(TempDir, Solo)> {
  let (directory, path) = temp_pdf(write)?;
  Ok((directory, Solo::open(&path, "", text_mode, screen)?))
}

/// A PDF of `objects`, numbered from 1; object 1 is the catalog.
pub fn write_pdf(path: &Path, objects: &[String], trailer: &str) -> Result<()> {
  let mut out = String::from("%PDF-1.7\n");
  let mut offsets = Vec::new();
  for (index, body) in objects.iter().enumerate() {
    offsets.push(out.len());
    write!(out, "{} 0 obj\n{body}\nendobj\n", index + 1)?;
  }
  let xref = out.len();
  let size = objects.len() + 1;
  write!(out, "xref\n0 {size}\n0000000000 65535 f \n")?;
  for offset in offsets {
    writeln!(out, "{offset:010} 00000 n ")?;
  }
  write!(
    out,
    "trailer\n<< /Size {size} /Root 1 0 R {trailer} >>\n\
     startxref\n{xref}\n%%EOF\n"
  )?;
  std::fs::write(path, out)?;
  Ok(())
}

/// One page of Helvetica `line`s, 24 pt apart from the top.
pub fn text_pdf(path: &Path, lines: &[&str]) -> Result<()> {
  spaced_pdf(path, lines, 24)
}

/// One page of 18 pt Helvetica `line`s, `leading` pt apart.
pub fn spaced_pdf(path: &Path, lines: &[&str], leading: u16) -> Result<()> {
  pages_of(path, &[lines], ("Helvetica", 18, leading), &[])
}

/// Pages of `text_pdf` lines.
pub fn pages_pdf(path: &Path, pages: &[&[&str]]) -> Result<()> {
  pages_of(path, pages, HELVETICA, &[])
}

/// `pages_pdf` with web links over `(page, line)`s.
pub fn linked_pages_pdf(
  path: &Path,
  pages: &[&[&str]],
  links: &[(usize, usize)],
) -> Result<()> {
  pages_of(path, pages, HELVETICA, links)
}

/// Base font, size, and leading in points.
pub type Style = (&'static str, u16, u16);

const HELVETICA: Style = ("Helvetica", 18, 24);

/// 300×200 pt pages; links span `(page, line)`s.
pub fn pages_of(
  path: &Path,
  pages: &[&[&str]],
  (font, size, leading): Style,
  links: &[(usize, usize)],
) -> Result<()> {
  let kids: Vec<_> =
    (0..pages.len()).map(|index| format!("{} 0 R", 4 + 2 * index)).collect();
  let mut objects = vec![
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    format!(
      "<< /Type /Pages /Kids [{}] /Count {} >>",
      kids.join(" "),
      pages.len()
    ),
    format!("<< /Type /Font /Subtype /Type1 /BaseFont /{font} >>"),
  ];
  let first = 4 + 2 * pages.len();
  for (index, lines) in pages.iter().enumerate() {
    let mut content = format!("BT /F1 {size} Tf {leading} TL 20 170 Td");
    for line in *lines {
      write!(content, " ({line}) Tj T*")?;
    }
    content.push_str(" ET");
    let annots: Vec<_> = (links.iter().enumerate())
      .filter(|(_, (page, _))| *page == index)
      .map(|(link, _)| format!("{} 0 R", first + link))
      .collect();
    objects.push(format!(
      "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
       /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R \
       /Annots [{}] >>",
      5 + 2 * index,
      annots.join(" ")
    ));
    objects.push(format!(
      "<< /Length {} >>\nstream\n{content}\nendstream",
      content.len()
    ));
  }
  for (_, line) in links {
    let base = 170 - i32::from(leading) * i32::try_from(*line)?;
    objects.push(format!(
      "<< /Type /Annot /Subtype /Link /Rect [15 {} 285 {}] \
       /A << /S /URI /URI (https://example.com/) >> >>",
      base - 6,
      base + 16
    ));
  }
  write_pdf(path, &objects, "")
}

pub fn crop_loaded(app: &mut App) -> Result<()> {
  wait_until(30, "Crop", || {
    app.crop_updates()?;
    Ok(!app.crop_scan.active())
  })
}

/// Generated labels with separate rows.
pub fn hints(
  count: u16,
  cell: (u16, u16),
  target: &vpdf_model::document::Target,
) -> vpdf_ui::hints::Hints {
  let found = (0..count)
    .map(|row| (((cell.0, cell.1 + row), cell.0), target.clone()))
    .collect();
  vpdf_ui::hints::Hints::new(found, false, 0, 80).expect("hint candidates")
}

#[path = "../support/reader.rs"]
mod reader_fixture;
pub use reader_fixture::SeedReader;
