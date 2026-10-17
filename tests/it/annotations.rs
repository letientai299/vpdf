use mupdf::pdf::PdfAnnotationType;
use mupdf::{Point, Quad, Rect};
use std::path::PathBuf;
use vpdf::Result;
use vpdf::app::App;
use vpdf::keys::PanelKind;
use vpdf::layout::Screen;
use vpdf::text::PageId;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn panel_lists_marked_text_and_notes() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut source = vpdf::document::Document::open(&root.join("paper.pdf"), "")?;
  let line = source.text(PageId(1))?.lines[2].clone();
  let first = line.glyphs.first().unwrap().bounds;
  let last = line.glyphs.last().unwrap().bounds;
  let mut page = source.native.load_pdf_page(1)?;
  let mut highlight = page.create_annotation(PdfAnnotationType::Highlight)?;
  highlight.set_quad_points(Quad::new(
    Point::new(first.x0, first.y0),
    Point::new(last.x1, first.y0),
    Point::new(first.x0, last.y1),
    Point::new(last.x1, last.y1),
  ))?;
  highlight.set_contents("check this")?;
  let mut note = page.create_annotation(PdfAnnotationType::Text)?;
  note.set_rect(Rect::new(10.0, 10.0, 30.0, 30.0))?;
  note.set_contents("margin note")?;
  drop((highlight, note, page));
  let file = tempfile::NamedTempFile::new()?;
  source.native.save(&file.path().to_string_lossy())?;
  let screen = Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = App::with_screen(file.path(), "", false, screen)?;
  app.open_panel(PanelKind::Annotations)?;
  let rows = &app.panel.as_ref().unwrap().rows;
  let labels: Vec<_> = rows.iter().map(|row| row.label.as_str()).collect();
  let text: String = line.glyphs.iter().map(|g| g.ch).collect();
  let words = text.split_whitespace().collect::<Vec<_>>().join(" ");
  assert_eq!(
    labels,
    [format!("Highlight: {words} — check this"), "Note: margin note".into()]
  );
  Ok(())
}
