use std::sync::Arc;
use vpdf_model::text::{Bounds, Glyph, Line, PageId, Position, Text};
use vpdf_reader::Result;
use vpdf_reader::motion::Motion;
use vpdf_reader::reader::{Reader, SelectionKind};

type Pages = Vec<Arc<Text>>;

fn pages() -> Pages {
  [vec!["one two", "café Straße"], vec![], vec!["last page"]]
    .into_iter()
    .map(|lines| {
      Arc::new(Text {
        lines: lines
          .iter()
          .map(|line| Line {
            ocr: None,
            paragraph: true,
            glyphs: line
              .chars()
              .map(|ch| Glyph { quad: None, ch, bounds: Bounds::default() })
              .collect(),
          })
          .collect(),
        truncated: false,
      })
    })
    .collect()
}

#[test]
fn words_operators_and_registers() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.yank(1, 'a', &mut source)?;
  reader.object(false, &mut source)?;
  assert_eq!(reader.registers.get(&'a').map(|text| &**text), Some("one"));
  reader.yank(1, '"', &mut source)?;
  reader.motion(Motion::Word, 1, false, &mut source)?;
  assert_eq!(reader.pending_yank.take().unwrap().text, "one ");
  assert_eq!(reader.cursor.glyph, 0);
  reader.motion(Motion::Word, 1, false, &mut source)?;
  assert_eq!(reader.cursor.glyph, 4);
  reader.motion(Motion::EndWord, 1, false, &mut source)?;
  assert_eq!(reader.cursor.glyph, 6);
  reader.motion(Motion::BackWord, 1, false, &mut source)?;
  assert_eq!(reader.cursor.glyph, 4);
  Ok(())
}

#[test]
fn cross_page_unicode_selection_and_reselect() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.cursor.line = 1;
  reader.visual(SelectionKind::Character);
  reader.motion(Motion::Last, 1, false, &mut source)?;
  reader.yank(1, '+', &mut source)?;
  let yank = reader.pending_yank.take().unwrap();
  assert_eq!(yank.text, "café Straße\nlast page");
  assert_eq!(yank.register, '+');
  assert!(reader.selection.is_none());
  reader.reselect();
  reader.swap();
  assert_eq!(reader.cursor.page, PageId(0));
  assert_eq!(reader.selection.unwrap().anchor.page, PageId(2));
  Ok(())
}

#[test]
fn vertical_motion_crosses_blank_pages() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.cursor.glyph = 5;
  reader.motion(Motion::Down, 3, false, &mut source)?;
  assert_eq!(reader.cursor.page, PageId(2));
  reader.motion(Motion::Up, 3, false, &mut source)?;
  assert_eq!(reader.cursor.page, PageId(0));
  assert_eq!(reader.cursor.line, 0);
  Ok(())
}

#[test]
fn count_page_and_jump_history() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.scroll = 12.0;
  reader.motion(Motion::First, 3, true, &mut source)?;
  assert_eq!(reader.cursor.page, PageId(2));
  reader.history(false);
  assert_eq!(reader.cursor, Position::default());
  assert_eq!(reader.scroll, 12.0);
  reader.history(true);
  assert_eq!(reader.cursor.page, PageId(2));
  reader.motion(Motion::Last, 900, true, &mut source)?;
  assert_eq!(reader.cursor.page, PageId(2));
  Ok(())
}

#[test]
fn linewise_yank_and_operator_counts() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.yank_lines(2, '"', &mut source)?;
  assert_eq!(
    reader.pending_yank.take().unwrap().text,
    "one two\n\ncafé Straße\n"
  );
  assert_eq!(reader.cursor, Position::default());
  // `Y` in visual mode yanks the selected lines and arms nothing.
  reader.visual(SelectionKind::Character);
  reader.motion(Motion::Right, 1, false, &mut source)?;
  // Switching kinds keeps the anchor.
  reader.visual(SelectionKind::Line);
  assert_eq!(reader.selection.map(|s| s.anchor), Some(Position::default()));
  reader.yank_lines(1, '"', &mut source)?;
  assert_eq!(reader.pending_yank.take().unwrap().text, "one two\n");
  assert!(reader.operator.is_none());
  reader.yank(1, '"', &mut source)?;
  reader.motion(Motion::Last, 1, false, &mut source)?;
  assert_eq!(
    reader.pending_yank.take().unwrap().text,
    "one two\ncafé Straße\nlast page\n"
  );
  Ok(())
}

#[test]
fn reverse_line_selection_has_newline() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.cursor.line = 1;
  reader.visual(SelectionKind::Line);
  reader.motion(Motion::Up, 1, false, &mut source)?;
  reader.yank(1, '"', &mut source)?;
  assert_eq!(
    reader.pending_yank.take().unwrap().text,
    "one two\ncafé Straße\n"
  );
  Ok(())
}

#[test]
fn paragraphs_skip_blank_pages_and_yank_exclusively() -> Result<()> {
  let mut source = pages();
  let mut reader = Reader::default();
  reader.motion(Motion::NextParagraph, 1, false, &mut source)?;
  assert_eq!(reader.cursor.line, 1);
  reader.motion(Motion::NextParagraph, 1, false, &mut source)?;
  assert_eq!(reader.cursor.page, PageId(2));
  reader.motion(Motion::PreviousParagraph, 2, false, &mut source)?;
  assert_eq!(reader.cursor, Position::default());
  reader.yank(1, 'a', &mut source)?;
  reader.motion(Motion::NextParagraph, 1, false, &mut source)?;
  assert_eq!(reader.pending_yank.take().unwrap().text, "one two");
  reader.cursor.line = 1;
  reader.cursor.glyph = 2;
  reader.yank(1, 'a', &mut source)?;
  reader.motion(Motion::PreviousParagraph, 1, false, &mut source)?;
  assert_eq!(reader.pending_yank.take().unwrap().text, "ca");
  assert_eq!(reader.cursor.glyph, 0);
  reader.motion(Motion::Last, 1, false, &mut source)?;
  reader.cursor.glyph = 0;
  reader.yank(1, 'a', &mut source)?;
  reader.motion(Motion::NextParagraph, 1, false, &mut source)?;
  assert_eq!(reader.pending_yank.take().unwrap().text, "last page");
  Ok(())
}

#[test]
fn line_end_counts_and_goal() -> Result<()> {
  let mut source = pages();
  for visual in [false, true] {
    let mut reader = Reader::default();
    if visual {
      reader.visual(SelectionKind::Character);
    }
    reader.motion(Motion::LineEnd, 2, true, &mut source)?;
    assert_eq!((reader.cursor.line, reader.cursor.glyph), (1, 10));
    reader.motion(Motion::Up, 1, false, &mut source)?;
    assert_eq!(reader.cursor.glyph, 6);
    reader.motion(Motion::Down, 1, false, &mut source)?;
    assert_eq!(reader.cursor.glyph, 10);
    if visual {
      assert_eq!(reader.selection.unwrap().anchor, Position::default());
    }
  }
  let mut reader = Reader::default();
  reader.yank(2, 'a', &mut source)?;
  reader.motion(Motion::LineEnd, 1, false, &mut source)?;
  assert_eq!(
    reader.registers.get(&'a').map(|value| &**value),
    Some("one two\ncafé Straße")
  );
  assert_eq!(reader.cursor, Position::default());
  assert_eq!(reader.desired, None);
  Ok(())
}
