use mupdf::pdf::{Encryption, PageLabelRule, PageLabelStyle, PdfWriteOptions};
use std::path::PathBuf;
use vpdf::Result;
use vpdf_model::text::PageId;
use vpdf_pdf::document::Document;

#[test]
#[ignore = "requires verified public PDF corpus"]
fn password_and_page_labels() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut source = Document::open(&root.join("paper.pdf"), "")?;
  let expected = source.text(PageId(0))?.plain();
  source
    .native
    .set_page_label_rule(PageLabelRule::new(0, PageLabelStyle::LowerRoman))?;
  let mut options = PdfWriteOptions::default();
  options
    .set_encryption(Encryption::Aes256)
    .set_owner_password("test-owner")
    .set_user_password("test-reader");
  let mut file = tempfile::NamedTempFile::new()?;
  source.native.write_to_with_options(&mut file, options)?;
  assert!(Document::open(file.path(), "").is_err());
  assert!(Document::open(file.path(), "incorrect").is_err());
  let mut opened = Document::open(file.path(), "test-reader")?;
  assert_eq!(opened.page_number("iv"), Some(PageId(3)));
  assert_eq!(opened.page_number("4"), Some(PageId(3)));
  assert_eq!(opened.text(PageId(0))?.plain(), expected);
  Ok(())
}

/// paper.pdf rewritten with label `rules`: start page, style, prefix.
fn relabeled(
  rules: &[(usize, PageLabelStyle, &str)],
) -> Result<tempfile::NamedTempFile> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut source = Document::open(&root.join("paper.pdf"), "")?;
  for &(index, style, prefix) in rules {
    let mut rule = PageLabelRule::new(index, style);
    rule.prefix = prefix.into();
    source.native.set_page_label_rule(rule)?;
  }
  let mut file = tempfile::NamedTempFile::new()?;
  source.native.write_to_with_options(&mut file, PdfWriteOptions::default())?;
  Ok(file)
}

fn resolved(doc: &Document) -> usize {
  doc.pages.iter().filter(|page| page.label.get().is_some()).count()
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn page_labels_resolve_on_demand() -> Result<()> {
  use PageLabelStyle::{Decimal, LowerRoman, None as Bare};
  let file = relabeled(&[
    (0, LowerRoman, ""),
    (3, Decimal, ""),
    (6, Bare, "Cover"),
    (8, Decimal, "第"),
    // UTF-16 keeps the escape; PDFDocEncoding maps it to a glyph.
    (10, Decimal, "界\x1b]2;x"),
    (12, Bare, ""),
  ])?;
  let doc = Document::open(file.path(), "")?;
  assert_eq!(resolved(&doc), 0, "open reads no label");
  let expected = [
    "i", "ii", "iii", "1", "2", "3", "Cover", "Cover", "第1", "第2", "界]2;x1",
    "界]2;x2", "13", "14", "15",
  ];
  assert_eq!(doc.pages.len(), expected.len());
  assert!(doc.native.page_label(10)?.contains('\x1b'), "hostile input");
  assert_eq!(doc.label(PageId(4)), "2");
  assert_eq!(resolved(&doc), 1, "one page read");
  for (index, label) in expected.iter().enumerate() {
    assert_eq!(doc.label(PageId(index)), *label, "page {index}");
  }
  assert_eq!(doc.get_label(PageId(15)), None);
  for (query, page) in [
    ("iii", Some(2)),
    // Numbers name physical pages, never labels.
    ("1", Some(0)),
    ("4", Some(3)),
    ("13", Some(12)),
    ("Cover", Some(6)),
    ("第2", Some(9)),
    ("界]2;x1", Some(10)),
    ("0", None),
    ("16", None),
    ("missing", None),
  ] {
    assert_eq!(doc.page_number(query), page.map(PageId), "{query}");
  }
  let screen = vpdf_ui::layout::Screen {
    columns: 120,
    rows: 50,
    cell_width: 8,
    cell_height: 16,
    origin: (0, 0),
  };
  let mut app = vpdf::solo::Solo::open(file.path(), "", false, screen)?;
  app.draw(&mut Vec::new())?;
  assert!(resolved(&app.doc) <= 2, "first frame reads visible labels");
  app.command("1")?;
  assert_eq!(app.reader.cursor.page, PageId(0));
  app.command("ii")?;
  assert_eq!(app.reader.cursor.page, PageId(1));
  // A reload reads the replacement's labels.
  relabeled(&[(0, Decimal, "P")])?.persist(file.path())?;
  app.command("reload")?;
  assert_eq!(resolved(&app.doc), 0);
  assert_eq!(app.doc.label(PageId(0)), "P1");
  app.command("P3")?;
  assert_eq!(app.reader.cursor.page, PageId(2));
  Ok(())
}
