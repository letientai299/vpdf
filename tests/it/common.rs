use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};
use vpdf::Result;
use vpdf::app::App;

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
  while !matches!(app.search.phase, vpdf::find::Phase::Complete) {
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
  let mut content = format!("BT /F1 18 Tf {leading} TL 20 170 Td");
  for line in lines {
    write!(content, " ({line}) Tj T*")?;
  }
  content.push_str(" ET");
  let objects = [
    "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] \
     /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
      .to_owned(),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
  ];
  write_pdf(path, &objects, "")
}
