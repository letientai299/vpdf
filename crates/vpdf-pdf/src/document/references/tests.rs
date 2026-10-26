use super::*;

#[test]
fn destination_and_object_targets() -> Result<()> {
  let file = crate::reference_fixture::pdf(0)?;
  let doc = Document::open(file.path(), "")?;
  assert!(matches!(doc.named_destination(b"proof")?, Target::Page(PageId(1))));
  assert!(matches!(
    doc.named_destination(b"a&b")?,
    Target::Point { page: PageId(0), .. }
  ));
  assert!(doc.named_destination(b"missing").is_err());
  assert!(matches!(doc.structure_target(&[255])?, Target::Page(PageId(1))));
  assert!(doc.structure_target(b"missing").is_err());
  assert!(doc.comment_bounds(PageId(0), b"missing").is_err());
  assert!(doc.comment_bounds(PageId(2), b"note").is_err());
  Ok(())
}

#[test]
fn invalid_destinations_fail() -> Result<()> {
  for destination in [
    "[22 0 R /Fit]",
    "[null /Fit]",
    "[-1 /Fit]",
    "[2 /Fit]",
    "[]",
    "<</D[22 0 R /Fit]>>",
    "(proof)",
  ] {
    let file = crate::reference_fixture::pdf(0)?;
    let data = std::fs::read_to_string(file.path())?
      .replace("(proof)[6 0 R /Fit]", &format!("(proof){destination}"));
    std::fs::write(file.path(), data)?;
    let doc = Document::open(file.path(), "")?;
    assert!(doc.named_destination(b"proof").is_err(), "{destination}");
  }
  Ok(())
}

#[test]
fn destination_values_preserve_semantics() -> Result<()> {
  for destination in ["<</D[6 0 R /Fit]>>", "[1 /Fit]", "(proof-alias)"] {
    let file = crate::reference_fixture::pdf(0)?;
    let data = std::fs::read_to_string(file.path())?
      .replace("(proof)[6 0 R /Fit]", &format!("(proof){destination}"))
      .replace("/Limits[(a&b)(proof)]", "/Limits[(a&b)(proof-alias)]")
      .replace("]>> endobj\n22", "(proof-alias)[6 0 R /Fit]]>> endobj\n22");
    std::fs::write(file.path(), data)?;
    let doc = Document::open(file.path(), "")?;
    let target = doc
      .named_destination(b"proof")
      .with_context(|| format!("Destination {destination}"))?;
    assert!(matches!(target, Target::Page(PageId(1))));
  }
  Ok(())
}

#[test]
fn rectangles_follow_engine_geometry() -> Result<()> {
  for rotation in [0, 90, 180, 270] {
    let file = crate::reference_fixture::pdf(rotation)?;
    let doc = Document::open(file.path(), "")?;
    let page = PageId(0);
    let object = doc.native.find_page(0)?;
    let expected = rect(
      mupdf::Rect::new(30.0, 40.0, 50.0, 60.0).transform(&object.page_ctm()?),
    );
    assert_eq!(doc.comment_bounds(page, b"note")?, expected);
    let engine = rect(doc.native.load_page(0)?.bounds()?);
    assert_eq!(doc.pages[0].bounds, engine);
    let values = [5.0, 25.0, 10.0, 30.0];
    assert_eq!(
      doc.highlight_bounds(page, values)?,
      Bounds {
        x0: engine.x0 + 5.0,
        x1: engine.x0 + 25.0,
        y0: engine.y0 + 10.0,
        y1: engine.y0 + 30.0,
      }
    );
    assert!(
      doc
        .highlight_bounds(page, [0.0, engine.width() + 1.0, 0.0, 10.0])
        .is_err()
    );
    assert!(doc.highlight_bounds(page, [10.0, 5.0, 0.0, 10.0]).is_err());
  }
  Ok(())
}

#[test]
fn legacy_destination_dictionary() -> Result<()> {
  for destination in ["[4 0 R /Fit]", "/legacy-alias", "(legacy-alias)"] {
    let file = crate::reference_fixture::pdf(0)?;
    let data = std::fs::read_to_string(file.path())?
      .replace("/Names<</Dests", "/UnusedNames<</Dests")
      .replace(
        "/Pages 2 0 R",
        &format!(
          "/Pages 2 0 R/Dests<</legacy{destination}/legacy-alias[4 0 R /Fit]>>"
        ),
      );
    std::fs::write(file.path(), data)?;
    let doc = Document::open(file.path(), "")?;
    assert!(matches!(
      doc.named_destination(b"legacy")?,
      Target::Page(PageId(0))
    ));
  }
  Ok(())
}

#[test]
fn legacy_destinations_take_precedence() -> Result<()> {
  let file = crate::reference_fixture::pdf(0)?;
  let data = std::fs::read_to_string(file.path())?
    .replace("/Pages 2 0 R", "/Pages 2 0 R/Dests<</proof[4 0 R /Fit]>>");
  std::fs::write(file.path(), data)?;
  let doc = Document::open(file.path(), "")?;
  assert!(matches!(doc.named_destination(b"proof")?, Target::Page(PageId(0))));
  assert!(doc.named_destination(b"a&b").is_err());
  Ok(())
}

#[test]
fn text_names_decode_pdf_unicode() -> Result<()> {
  let file = crate::reference_fixture::pdf(0)?;
  let encoded = "<FEFF00630061006600E9>";
  let data = std::fs::read_to_string(file.path())?
    .replace("/NM(note)", &format!("/NM{encoded}"))
    .replace("(proof)", encoded);
  std::fs::write(file.path(), data)?;
  let doc = Document::open(file.path(), "")?;
  assert!(matches!(
    doc.named_destination("café".as_bytes())?,
    Target::Page(PageId(1))
  ));
  assert!(doc.comment_bounds(PageId(0), "café".as_bytes()).is_ok());
  assert!(matches!(doc.structure_target(&[255])?, Target::Page(PageId(1))));
  Ok(())
}

#[test]
fn cropped_comments_use_visible_area() -> Result<()> {
  for rotation in [0, 90, 180, 270] {
    let file = crate::reference_fixture::pdf(rotation)?;
    let data = std::fs::read_to_string(file.path())?
      .replace("/Rect[30 40 50 60]", "/Rect[0 10 30 40]");
    std::fs::write(file.path(), data)?;
    let doc = Document::open(file.path(), "")?;
    let raw = mupdf::Rect::new(0.0, 10.0, 30.0, 40.0)
      .transform(&doc.native.find_page(0)?.page_ctm()?);
    let expected =
      vpdf_model::geometry::clip(rect(raw), doc.pages[0].bounds).unwrap();
    assert_eq!(doc.comment_bounds(PageId(0), b"note")?, expected);
  }
  Ok(())
}
