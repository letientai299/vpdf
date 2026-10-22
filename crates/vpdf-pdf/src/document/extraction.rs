use super::{
  DisplayList, GLYPH_LIMIT, Glyph, Line, Result, Text, TextPageFlags,
  quad_bounds,
};

pub const TEXT_FLAGS: TextPageFlags = TextPageFlags::PRESERVE_LIGATURES
  .union(TextPageFlags::PRESERVE_WHITESPACE)
  .union(TextPageFlags::CLIP)
  .union(TextPageFlags::USE_CID_FOR_UNKNOWN_UNICODE);

pub fn extract(page: &mupdf::Page) -> Result<Text> {
  Ok(read(&page.to_text_page(TEXT_FLAGS)?))
}

/// Text from a display list, which skips interpreting the page again.
pub fn extract_list(list: &DisplayList) -> Result<Text> {
  Ok(read(&list.to_text_page(TEXT_FLAGS)?))
}

fn read(raw: &mupdf::TextPage) -> Text {
  let mut text = Text::default();
  let mut count = 0;
  for block in raw.blocks() {
    let mut paragraph = true;
    for line in block.lines() {
      let mut row = Line::default();
      for glyph in line.chars() {
        let Some(ch) = glyph.char() else { continue };
        if ch.is_control() {
          continue;
        }
        if count >= GLYPH_LIMIT {
          text.truncated = true;
          break;
        }
        let q = glyph.quad();
        let quad = vpdf_model::text::Quad([
          (q.ul.x, q.ul.y),
          (q.ur.x, q.ur.y),
          (q.lr.x, q.lr.y),
          (q.ll.x, q.ll.y),
        ]);
        row.glyphs.push(Glyph {
          quad: Some(quad),
          ch,
          bounds: quad_bounds(&q),
        });
        count += 1;
      }
      if row.glyphs.iter().any(|g| !g.ch.is_whitespace()) {
        row.paragraph = paragraph;
        paragraph = false;
        text.lines.push(row);
      }
      if text.truncated {
        return text;
      }
    }
  }
  text
}
