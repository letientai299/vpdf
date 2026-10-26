use crate::Result;

pub fn pdf(rotation: u16) -> Result<tempfile::NamedTempFile> {
  let file = tempfile::NamedTempFile::new()?;
  crate::fixtures::pdf(
    file.path(),
    &["one", "two"],
    &format!("/CropBox[10 20 290 180]/Rotate {rotation}/Annots[20 0 R]"),
  )?;
  let mut data = std::fs::read_to_string(file.path())?;
  data = data.replace(
    "/Pages 2 0 R",
    concat!(
      "/Pages 2 0 R/Names<</Dests<</Kids[21 0 R]>>>>",
      "/StructTreeRoot<</IDTree<</Names[<ff> 22 0 R]>>>>"
    ),
  );
  data = data.replace("trailer", concat!(
    "20 0 obj <</Type/Annot/Subtype/Text/NM(note)/Rect[30 40 50 60]>> endobj\n",
    "21 0 obj <</Limits[(a&b)(proof)]/Names[(a&b)[4 0 R /XYZ 20 100 0]",
    "(proof)[6 0 R /Fit]]>> endobj\n",
    "22 0 obj <</Type/StructElem/ID<ff>/K[<</Pg 6 0 R>>]>> endobj\ntrailer"
  ));
  std::fs::write(file.path(), data)?;
  Ok(file)
}
