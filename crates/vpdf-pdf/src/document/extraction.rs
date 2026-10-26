use super::style::{LineStyle, style};
use super::{
  DisplayList, GLYPH_LIMIT, Glyph, Line, Result, Text, TextPageFlags,
  quad_bounds,
};

pub const TEXT_FLAGS: TextPageFlags = TextPageFlags::PRESERVE_LIGATURES
  .union(TextPageFlags::PRESERVE_WHITESPACE)
  .union(TextPageFlags::CLIP)
  .union(TextPageFlags::USE_CID_FOR_UNKNOWN_UNICODE);

pub fn extract(page: &mupdf::Page) -> Result<Text> {
  Ok(read(&page.to_text_page(TEXT_FLAGS)?, None))
}

/// Text with one style per line.
pub fn extract_styled(page: &mupdf::Page) -> Result<(Text, Vec<LineStyle>)> {
  let mut styles = Vec::new();
  let text = read(&page.to_text_page(TEXT_FLAGS)?, Some(&mut styles));
  Ok((text, styles))
}

/// Text from a display list, which skips interpreting the page again.
pub fn extract_list(list: &DisplayList) -> Result<Text> {
  Ok(read(&list.to_text_page(TEXT_FLAGS)?, None))
}

fn read(
  raw: &mupdf::TextPage,
  mut styles: Option<&mut Vec<LineStyle>>,
) -> Text {
  let mut text = Text::default();
  let mut count = 0;
  let blocks: Vec<_> =
    raw.blocks().filter(|block| block.lines().next().is_some()).collect();
  let bounds: Vec<_> = blocks.iter().map(mupdf::TextBlock::bounds).collect();
  for at in block_order(&bounds) {
    let block = &blocks[at];
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
        if let Some(styles) = styles.as_deref_mut() {
          styles.push(style(&line));
        }
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

fn block_order(bounds: &[mupdf::Rect]) -> Vec<usize> {
  let mut order: Vec<_> = (0..bounds.len()).collect();
  order.sort_by(|&a, &b| bounds[a].y0.total_cmp(&bounds[b].y0));
  let mut bands = vec![0; bounds.len()];
  let mut band = 0;
  let mut bottom = f32::NEG_INFINITY;
  for &at in &order {
    if bounds[at].y0 >= bottom {
      band += 1;
    }
    bands[at] = band;
    bottom = bottom.max(bounds[at].y1);
  }
  // Preserve flow within overlapping columns.
  order.sort_by_key(|&at| (bands[at], at));
  order
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn blocks_follow_vertical_bands() {
    for (spans, expected) in [
      (vec![(390.0, 600.0), (350.0, 365.0), (30.0, 42.0)], vec![2, 1, 0]),
      (vec![(100.0, 500.0), (80.0, 480.0), (30.0, 42.0)], vec![2, 0, 1]),
      (vec![(300.0, 500.0), (100.0, 310.0), (80.0, 110.0)], vec![0, 1, 2]),
      (vec![(50.0, 60.0), (30.0, 50.0)], vec![1, 0]),
      (vec![], vec![]),
    ] {
      let bounds: Vec<_> = spans
        .into_iter()
        .map(|(top, bottom)| mupdf::Rect::new(0.0, top, 100.0, bottom))
        .collect();
      assert_eq!(block_order(&bounds), expected);
    }
  }

  #[test]
  fn styles_report_font_facts() -> Result<()> {
    let content = "BT /R 10 Tf 20 180 Td (Body text) Tj ET \
       BT /R 10 Tf 20 160 Td (Data) Tj /R 6 Tf (11) Tj ET \
       BT /B 14 Tf 20 140 Td (Heading) Tj ET \
       BT /B 10 Tf 20 100 Td (Lead) Tj /R 10 Tf ( in body) Tj ET \
       BT /C 10 Tf 20 60 Td (let x = 1) Tj ET \
       BT /B 10 Tf 20 20 Td (Example) Tj /R 10 Tf ( runs into body ) Tj \
       /B 10 Tf (Names) Tj ET";
    let font =
      |name: &str| format!("<</Type/Font/Subtype/Type1/BaseFont/{name}>>");
    let pdf = format!(
      "%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n\
       2 0 obj <</Type/Pages/Kids[3 0 R]/Count 1>> endobj\n\
       3 0 obj <</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]\
       /Resources<</Font<</R {}/B {}/C {}>>>>/Contents 4 0 R>> endobj\n\
       4 0 obj <</Length {}>> stream\n{content}\nendstream endobj\n\
       trailer <</Root 1 0 R>>\n%%EOF\n",
      font("Helvetica"),
      font("Helvetica-Bold"),
      font("Courier"),
      content.len()
    );
    let file = tempfile::NamedTempFile::new()?;
    std::fs::write(file.path(), pdf)?;
    let doc = mupdf::Document::open(file.path().to_str().unwrap_or_default())?;
    let (text, styles) = extract_styled(&doc.load_page(0)?)?;
    let facts: Vec<_> = text
      .lines
      .iter()
      .zip(&styles)
      .map(|(line, s)| (line.text(), s.size.round(), s.bold, s.mono, s.mixed))
      .collect();
    assert_eq!(
      facts,
      [
        ("Body text".into(), 10.0, false, false, false),
        ("Data11".into(), 10.0, false, false, true),
        ("Heading".into(), 14.0, true, false, false),
        ("Lead in body".into(), 10.0, false, false, true),
        ("let x = 1".into(), 10.0, false, true, false),
        ("Example runs into body Names".into(), 10.0, false, false, true),
      ]
    );
    Ok(())
  }
}
