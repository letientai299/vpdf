//! Text objects over native page text.
use super::common::keys;
use vpdf::Result;
use vpdf::pdf_text::PdfText;
use vpdf::solo::Solo;
use vpdf::text::{Line, PageId, Position};
use vpdf_reader::objects::Object;
use vpdf_reader::reader::{Reader, SelectionKind};

#[test]
#[ignore = "requires verified public PDF corpus"]
fn object_raw_keys_preserve_normal_and_copy_blocks() -> Result<()> {
  use vpdf_ui::layout::Screen;
  let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
  for text_mode in [false, true] {
    let mut app = Solo::open(
      &root.join("paper.pdf"),
      "",
      text_mode,
      Screen {
        columns: 100,
        rows: 30,
        cell_width: 8,
        cell_height: 16,
        origin: (0, 0),
      },
    )?;
    app.doc.trim();
    let cursor = app.reader.cursor;
    keys(&mut app, b"WBEgE()aWapab[]")?;
    assert_eq!(app.reader.cursor, cursor);
    assert!(app.reader.selection.is_none() && app.reader.operator.is_none());
    assert_eq!(app.doc.cache_bytes(), 0);
    let text = app.doc.text(PageId(1))?;
    let starts: Vec<_> = text
      .lines
      .iter()
      .enumerate()
      .filter_map(|(i, line)| line.paragraph.then_some(i))
      .collect();
    let pair = starts.windows(2).find(|pair| pair[1] - pair[0] > 2).unwrap();
    let (first, next) = (pair[0], pair[1]);
    let expected = text.lines[first..next]
      .iter()
      .map(Line::text)
      .collect::<Vec<_>>()
      .join("\n");
    let glyph = text.lines[first + 1]
      .glyphs
      .iter()
      .position(|g| !g.ch.is_whitespace())
      .unwrap();
    let position = Position { page: PageId(1), line: first + 1, glyph };
    app.go_to(position, true)?;
    let scroll = app.reader.scroll;
    keys(&mut app, b"yip")?;
    assert_eq!(app.reader.take_yank().unwrap().text, expected.trim_end());
    assert_eq!(app.reader.scroll, scroll);
    assert!(app.reader.selection.is_none() && app.reader.operator.is_none());
    keys(&mut app, b"vip")?;
    assert_eq!(app.reader.selection.unwrap().anchor.line, first);
    assert_eq!(app.reader.cursor.line, next - 1);
    let mut frame = Vec::new();
    app.draw(&mut frame)?;
    assert_ne!(frame, [] as [u8; 0]);
    keys(&mut app, b"\x1b[27u")?;
    assert!(app.reader.selection.is_none());
    app.go_to(position, true)?;
    let row = &text.lines[position.line];
    let start = row.glyphs[..position.glyph]
      .iter()
      .rposition(|g| g.ch.is_whitespace())
      .map_or(0, |i| i + 1);
    let end = row.glyphs[position.glyph..]
      .iter()
      .position(|g| g.ch.is_whitespace())
      .map_or(row.glyphs.len(), |i| position.glyph + i);
    let expected: String =
      row.glyphs[start..end].iter().map(|g| g.ch).collect();
    keys(&mut app, b"yiW")?;
    assert_eq!(app.reader.take_yank().unwrap().text, expected);
  }
  Ok(())
}

#[test]
#[ignore = "requires verified public PDF corpus"]
fn object_ranges_cross_real_pages() -> Result<()> {
  use vpdf::document::Document;
  let root = std::path::PathBuf::from(std::env::var("VPDF_CORPUS")?);
  let mut doc = Document::open(&root.join("paper.pdf"), "")?;
  let first = doc.text(PageId(0))?;
  let line = first.lines.len() - 1;
  let glyph = first.lines[line]
    .glyphs
    .iter()
    .rposition(|g| g.ch.is_whitespace())
    .map_or(0, |i| i + 1);
  let start = Position { page: PageId(0), line, glyph };
  let mut reader = Reader::default();
  reader.cursor = start;
  reader.visual(SelectionKind::Character);
  reader.select_object(Object::BigWord, false, 2, &mut PdfText(&mut doc))?;
  assert_eq!(reader.selection.unwrap().anchor, start);
  assert_eq!(reader.cursor.page, PageId(1));
  let next = doc.text(PageId(1))?;
  let last = reader.cursor;
  let expected =
    first.lines[line].glyphs[glyph..].iter().map(|g| g.ch).collect::<String>()
      + "\n"
      + &next.lines[0].glyphs[..=last.glyph]
        .iter()
        .map(|g| g.ch)
        .collect::<String>();
  reader.yank(1, 'a', &mut PdfText(&mut doc))?;
  assert_eq!(reader.pending_yank.take().unwrap().text, expected);
  Ok(())
}
