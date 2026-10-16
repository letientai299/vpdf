use sha2::{Digest, Sha256};
use std::path::PathBuf;
use vpdf::{Result, document::Document, search, text::PageId};

#[test]
#[ignore = "requires verified public PDF corpus"]
fn public_documents() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let cases = [
    (
      "book.pdf",
      38,
      1112,
      "6033000bac3f9268ad75ee041bf8e5d9a3bc8a33bb156399cbd014918cb59f94",
    ),
    (
      "paper.pdf",
      2,
      1800,
      "bdfaa68d8984f0dc02beaca527b76f207d99b666d31d1da728ee0728182df697",
    ),
    (
      "form.pdf",
      0,
      5075,
      "3d31c226df0d189ced80e039d01cf0f8820c1019681a0f0ca6264de277b7e982",
    ),
    (
      "ocr.pdf",
      0,
      4088,
      "7a161be656c7e82986a1959890eadbdcd0876e15e01b28fd00900190de15eed5",
    ),
  ];
  for (name, page, glyphs, expected) in cases {
    let path = root.join(name);
    let bytes = std::fs::read(&path)?;
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected);
    let mut doc = Document::open(&path, "")?;
    let text = doc.text(PageId(page))?;
    assert_eq!(
      text.lines.iter().map(|l| l.glyphs.len()).sum::<usize>(),
      glyphs,
      "{name}"
    );
    assert!(!text.truncated);
    assert!(doc.text(PageId(doc.pages.len())).is_err());
    let index = search::Index::new(PageId(page), &text);
    let first = text.lines[0].text();
    let first = first.split_whitespace().next().unwrap().trim_end_matches('-');
    assert!(
      !index.matches(&search::compile(first, false)?, 1000)?.is_empty(),
      "{name}"
    );
  }
  Ok(())
}
