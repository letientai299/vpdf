use std::fs::File;
use std::path::PathBuf;
use vpdf::content_cache::{alias, hex};
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
    (
      "review.pdf",
      0,
      3243,
      "0bbddcd853aebe2e02cd786d43e30a568f2f7eb9c1a6393c6990a82dce1f76a3",
    ),
  ];
  for (name, page, glyphs, expected) in cases {
    let path = root.join(name);
    assert_eq!(hex(&alias(&mut File::open(&path)?)?), expected);
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
    let plain = text.plain();
    let first = plain
      .split_whitespace()
      .find(|word| word.chars().all(char::is_alphabetic))
      .unwrap();
    assert!(
      !index.matches(&search::compile(first, false)?, 1000)?.is_empty(),
      "{name}"
    );
  }
  Ok(())
}

/// Text read from a page's display list, which the reader shares with
/// its tiles, equals text read by interpreting the page.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn display_list_text_matches_page_text() -> Result<()> {
  use vpdf::document::{extract, extract_list};
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for name in ["book.pdf", "paper.pdf", "form.pdf", "ocr.pdf"] {
    let doc = Document::open(&root.join(name), "")?;
    for index in 0..doc.pages.len() as i32 {
      let page = doc.native.load_page(index)?;
      let list = page.to_display_list(true)?;
      assert_eq!(
        format!("{:?}", extract(&page)?),
        format!("{:?}", extract_list(&list)?),
        "{name} page {}",
        index + 1
      );
    }
  }
  Ok(())
}

/// Page bounds read from page dictionaries equal those of loaded pages.
#[test]
#[ignore = "requires verified public PDF corpus"]
fn dictionary_bounds_match_loaded_pages() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for name in ["book.pdf", "paper.pdf", "form.pdf", "ocr.pdf"] {
    let doc = Document::open(&root.join(name), "")?;
    for (index, page) in doc.pages.iter().enumerate() {
      let loaded: vpdf::text::Bounds =
        doc.native.load_page(index as i32)?.bounds()?.into();
      assert_eq!(
        (page.bounds.x0, page.bounds.y0, page.bounds.x1, page.bounds.y1),
        (loaded.x0, loaded.y0, loaded.x1, loaded.y1),
        "{name} page {}",
        index + 1
      );
    }
  }
  Ok(())
}
