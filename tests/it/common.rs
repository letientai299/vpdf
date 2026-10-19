use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;
use vpdf_terminal::decode::{Event, Parser};
use vpdf_ui::prompt::History;

/// The focused pane of `session` drawn alone.
pub fn focused_frame(session: &mut vpdf::session::Session) -> Result<String> {
  let mut out = Vec::new();
  session.draw_focused(&mut out)?;
  Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Events decoded from `bytes`, without waiting for a quiet pause.
pub fn events(bytes: &[u8]) -> VecDeque<Event> {
  let mut events = VecDeque::new();
  Parser::default().feed(bytes, &mut events);
  events
}

/// Feeds key `bytes` to `app`.
pub fn keys(app: &mut App, bytes: &[u8]) -> Result<()> {
  let events = events(bytes);
  assert!(bytes.is_empty() || !events.is_empty(), "undecoded key bytes");
  for event in events {
    let Event::Key(key) = event else { panic!("Expected key") };
    app.key(key, &mut History::default())?;
  }
  Ok(())
}

/// Starts the outline the app loads off the input thread and waits.
pub fn outline_loaded(app: &mut App) -> Result<()> {
  app.want_outline();
  let started = Instant::now();
  while app.outline_pending() {
    app.outline_updates()?;
    anyhow::ensure!(started.elapsed() < Duration::from_secs(10), "Outline");
    std::thread::sleep(Duration::from_millis(5));
  }
  Ok(())
}

pub fn search_loaded(app: &mut App) -> Result<()> {
  let started = Instant::now();
  while !matches!(app.search.phase(), vpdf_ui::find::Phase::Complete) {
    app.search_updates()?;
    anyhow::ensure!(started.elapsed() < Duration::from_secs(15), "Search");
    std::thread::sleep(Duration::from_millis(1));
  }
  Ok(())
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
  pages_of(path, &[lines], leading, &[])
}

/// Pages of `text_pdf` lines.
pub fn pages_pdf(path: &Path, pages: &[&[&str]]) -> Result<()> {
  pages_of(path, pages, 24, &[])
}

/// `pages_pdf` with web links over `(page, line)`s.
pub fn linked_pages_pdf(
  path: &Path,
  pages: &[&[&str]],
  links: &[(usize, usize)],
) -> Result<()> {
  pages_of(path, pages, 24, links)
}

fn pages_of(
  path: &Path,
  pages: &[&[&str]],
  leading: u16,
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
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
  ];
  let first = 4 + 2 * pages.len();
  for (index, lines) in pages.iter().enumerate() {
    let mut content = format!("BT /F1 18 Tf {leading} TL 20 170 Td");
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
  let started = Instant::now();
  while app.crop_job.is_some() {
    app.crop_updates()?;
    anyhow::ensure!(
      started.elapsed() < Duration::from_secs(30),
      "Crop timeout"
    );
    std::thread::sleep(Duration::from_millis(5));
  }
  Ok(())
}

/// Generated labels with separate rows.
pub fn hints(
  count: u16,
  cell: (u16, u16),
  target: &vpdf::document::Target,
) -> vpdf_ui::hints::Hints {
  let found = (0..count)
    .map(|row| (((cell.0, cell.1 + row), cell.0), target.clone()))
    .collect();
  vpdf_ui::hints::Hints::new(found, false, 0, 80).expect("hint candidates")
}

#[path = "../support/reader.rs"]
mod reader_fixture;
pub use reader_fixture::SeedReader;
