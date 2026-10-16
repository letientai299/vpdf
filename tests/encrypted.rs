use mupdf::pdf::{Encryption, PageLabelRule, PageLabelStyle, PdfWriteOptions};
use std::path::PathBuf;
use vpdf::{Result, document::Document, text::PageId};

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
