//! Small PDFs written by tests.
use crate::Result;
use std::fmt::Write as _;
use std::path::Path;

/// Writes a PDF to `path` with one page per entry of `pages`, each
/// showing that line of Helvetica text; `annots` joins every page's
/// dictionary. `MuPDF` repairs the missing cross-reference table.
pub fn pdf(path: &Path, pages: &[&str], annots: &str) -> Result<()> {
  let kids = (0..pages.len()).fold(String::new(), |mut kids, page| {
    let _ = write!(kids, "{} 0 R ", 4 + 2 * page);
    kids
  });
  let mut out = format!(
    "%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n\
     2 0 obj <</Type/Pages/Kids[{kids}]/Count {}>> endobj\n\
     3 0 obj <</Type/Font/Subtype/Type1/BaseFont/Helvetica>> endobj\n",
    pages.len()
  );
  for (page, text) in pages.iter().enumerate() {
    let content = format!("BT /F1 24 Tf 20 100 Td ({text}) Tj ET");
    let _ = write!(
      out,
      "{} 0 obj <</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]\
       /Resources<</Font<</F1 3 0 R>>>>/Contents {} 0 R{annots}>> endobj\n\
       {} 0 obj <</Length {}>> stream\n{content}\nendstream endobj\n",
      4 + 2 * page,
      5 + 2 * page,
      5 + 2 * page,
      content.len()
    );
  }
  out.push_str("trailer <</Root 1 0 R>>\n%%EOF\n");
  std::fs::write(path, out)?;
  Ok(())
}

/// Records each worker wake.
pub fn wake() -> (crate::jobs::Wake, std::sync::mpsc::Receiver<()>) {
  let (send, receive) = std::sync::mpsc::channel();
  let wake = crate::jobs::Wake::new(move || {
    let _ = send.send(());
  });
  (wake, receive)
}
