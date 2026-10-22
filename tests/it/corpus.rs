use std::fs::File;
use std::path::PathBuf;
use vpdf::Result;
use vpdf_model::text::PageId;
use vpdf_pdf::content_cache::{alias, hex};
use vpdf_pdf::document::Document;
use vpdf_search::search;

#[test]
#[ignore = "requires PMPP PDF in VPDF_SEARCH_PMPP"]
fn pmpp_search_visual_order() -> Result<()> {
  use vpdf::solo::Solo;
  use vpdf_model::text::Position;
  use vpdf_search::query::Query;

  let path = PathBuf::from(std::env::var("VPDF_SEARCH_PMPP")?);
  assert_eq!(
    hex(&alias(&mut File::open(&path)?)?),
    "58678d336170e1b4e134b432c71d3c684f6be45f9dae33838f9576020b9af0f9"
  );
  let page = PageId(160);
  let mut app = Solo::open(&path, "", false, super::common::cells(120, 50))?;
  let text = app.doc.text(page)?;
  let index = search::Index::new(page, &text);
  let mut encoded = Vec::new();
  index.encode(&mut encoded);
  for index in [index, search::Index::decode(page, &encoded)?] {
    let hits = index.matches(&search::compile("thread", false)?, 1000)?;
    assert_eq!(hits.len(), 11);
    assert!(hits[0].excerpt.starts_with("Thread coarsening"));
    assert!(hits[1].excerpt.starts_with("Thread coarsening for tiled matrix"));
    let positions: Vec<_> = hits.iter().map(|hit| hit.first).collect();
    let tops: Vec<_> = positions
      .iter()
      .map(|&position| text.glyph(position).unwrap().bounds.y0)
      .collect();
    assert!(tops.windows(2).all(|pair| pair[0] <= pair[1]));
    app.search.show(Query { text: "thread".into(), ..Query::default() }, hits);
    app.go_to(Position { page, ..Position::default() }, true)?;
    for expected in positions.iter().chain(positions.iter().take(1)) {
      super::common::keys(&mut app, b"n")?;
      assert_eq!(app.reader.cursor, *expected);
      assert_eq!(app.reading_position()?, *expected);
    }
    for expected in positions.iter().rev() {
      super::common::keys(&mut app, b"N")?;
      assert_eq!(app.reader.cursor, *expected);
      assert_eq!(app.reading_position()?, *expected);
    }
  }
  Ok(())
}

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
