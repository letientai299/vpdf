use std::path::PathBuf;
use vpdf::Result;
use vpdf::document::Document;
use vpdf::motion::Motion;
use vpdf::reader::Reader;
use vpdf::text::{PageId, Position};

#[test]
#[ignore = "requires verified public PDF corpus"]
fn paper_blocks_supply_paragraph_boundaries() -> Result<()> {
  let root = PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut doc = Document::open(&root.join("paper.pdf"), "")?;
  let text = doc.text(PageId(0))?;
  let starts: Vec<_> = text
    .lines
    .iter()
    .enumerate()
    .filter_map(|(i, line)| line.paragraph.then_some(i))
    .collect();
  assert_eq!(starts[0], 0);
  let pair = starts.windows(2).find(|p| p[1] - p[0] >= 3).unwrap();
  let (first, next) = (pair[0], pair[1]);
  let middle = Position { page: PageId(0), line: first + 1, glyph: 1 };
  let mut reader = Reader::default();
  reader.cursor = middle;
  reader.motion(Motion::NextParagraph, 1, false, &mut doc)?;
  assert_eq!(reader.cursor.line, next);
  assert_eq!(reader.cursor.glyph, 0);
  reader.motion(Motion::PreviousParagraph, 1, false, &mut doc)?;
  assert_eq!(reader.cursor.line, first);
  reader.cursor = middle;
  reader.motion(Motion::PreviousParagraph, 1, false, &mut doc)?;
  assert_eq!(reader.cursor.line, first);
  reader.yank(1, 'a', &mut doc)?;
  reader.motion(Motion::NextParagraph, 1, false, &mut doc)?;
  let expected = text.lines[first..next]
    .iter()
    .map(|line| line.text())
    .collect::<Vec<_>>()
    .join("\n");
  assert_eq!(reader.pending_yank.take().unwrap().text, expected);
  Ok(())
}
