use std::path::PathBuf;
use vpdf::Result;
use vpdf::document::Document;
use vpdf::text::PageId;

fn key(title: &str) -> String {
  title
    .chars()
    .filter(|c| c.is_alphanumeric())
    .collect::<String>()
    .to_lowercase()
}

/// Inferred headings against each corpus PDF's real outline.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn inferred_outline_matches_authored_one() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for (name, recall_floor, precision_floor) in
    [("paper.pdf", 0.95, 0.75), ("book.pdf", 0.95, 0.7)]
  {
    let mut doc = Document::open(&root.join(name), "")?;
    let pages = (0..doc.pages.len())
      .map(|page| doc.text(PageId(page)).map(|t| vpdf::outline::summarize(&t)))
      .collect::<Result<Vec<_>>>()?;
    let inferred = vpdf::outline::infer(&pages);
    let real: Vec<_> = doc.outline.iter().map(|h| key(&h.title)).collect();
    let guessed: Vec<_> = inferred.iter().map(|h| key(&h.title)).collect();
    let found = real
      .iter()
      .filter(|r| guessed.iter().any(|g| g.ends_with(r.as_str())))
      .count();
    let right = guessed
      .iter()
      .filter(|g| real.iter().any(|r| g.ends_with(r.as_str())))
      .count();
    let recall = found as f32 / real.len() as f32;
    let precision = right as f32 / guessed.len().max(1) as f32;
    eprintln!(
      "{name}: recall {recall:.2} precision {precision:.2} ({} guessed)",
      guessed.len()
    );
    for h in inferred
      .iter()
      .filter(|h| !real.iter().any(|r| key(&h.title).ends_with(r.as_str())))
      .take(15)
    {
      eprintln!("  extra: {} {}", h.depth, h.title);
    }
    for r in doc
      .outline
      .iter()
      .filter(|r| !guessed.iter().any(|g| g.ends_with(&key(&r.title))))
      .take(15)
    {
      eprintln!("  missed: {}", r.title);
    }
    assert!(recall >= recall_floor && precision >= precision_floor, "{name}");
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn documents_without_outline_infer_one_in_background() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let screen = vpdf::layout::Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app =
    vpdf::app::App::with_screen(&root.join("form.pdf"), "", false, screen)?;
  assert!(app.outline_job.is_some());
  let started = std::time::Instant::now();
  while !app.outline_updates()? {
    assert!(started.elapsed().as_secs() < 10);
    std::thread::sleep(std::time::Duration::from_millis(5));
  }
  assert!(app.doc.inferred);
  let book =
    vpdf::app::App::with_screen(&root.join("book.pdf"), "", false, screen)?;
  assert!(book.outline_job.is_none(), "authored outlines stay");
  Ok(())
}
