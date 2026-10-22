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
}
