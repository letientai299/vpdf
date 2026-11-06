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
      let vertical = line.wmode() == mupdf::WriteMode::Vertical;
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
        let q = glyph_quad(&glyph, vertical);
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

/// Quads taller than this many sizes get a font metric check.
const CHECK_EMS: f32 = 1.3;
/// Ascent and descent limits, in ems, for glyph quads.
const ASCENT: f32 = 1.0;
const DESCENT: f32 = -0.3;

fn glyph_quad(glyph: &mupdf::TextChar, vertical: bool) -> mupdf::Quad {
  let q = glyph.quad();
  let height = (q.ul.x - q.ll.x).hypot(q.ul.y - q.ll.y);
  // Skips the native font lookup for ordinary glyphs.
  if vertical || height <= CHECK_EMS * glyph.size() {
    return q;
  }
  let Some(font) = glyph.font() else { return q };
  clamp_quad(q, glyph.origin(), font.ascender(), font.descender())
}

/// Limits a glyph quad's ascent and descent around its baseline.
///
/// Glyph quads come from the font's ascender and descender. Producers copy
/// some CJK font bounding boxes there, such as 1.809 and -1.049 em, so each
/// quad spans two lines and highlights cover the line above.
fn clamp_quad(
  q: mupdf::Quad,
  origin: mupdf::Point,
  ascender: f32,
  descender: f32,
) -> mupdf::Quad {
  let total = ascender - descender;
  if total.is_nan()
    || total <= 0.0
    || (ascender <= ASCENT && descender >= DESCENT)
  {
    return q;
  }
  let (ascent, descent) = (ascender.min(ASCENT), descender.max(DESCENT));
  let (up_x, up_y) = ((q.ul.x - q.ll.x) / total, (q.ul.y - q.ll.y) / total);
  let (advance_x, advance_y) = (q.lr.x - q.ll.x, q.lr.y - q.ll.y);
  let at = |ems: f32, along: f32| {
    mupdf::Point::new(
      origin.x + up_x * ems + advance_x * along,
      origin.y + up_y * ems + advance_y * along,
    )
  };
  mupdf::Quad::new(
    at(ascent, 0.0),
    at(ascent, 1.0),
    at(descent, 0.0),
    at(descent, 1.0),
  )
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
  fn quads_clamp_to_font_limits() {
    let point = |(x, y): (f32, f32)| mupdf::Point::new(x, y);
    // Em vector up, advance, and descent per case; origin (100, 200).
    let quad = |up: (f32, f32), advance: (f32, f32), asc: f32, desc: f32| {
      let at = |ems: f32, along: f32| {
        point((
          100.0 + up.0 * ems + advance.0 * along,
          200.0 + up.1 * ems + advance.1 * along,
        ))
      };
      mupdf::Quad::new(at(asc, 0.0), at(asc, 1.0), at(desc, 0.0), at(desc, 1.0))
    };
    let corners = |q: mupdf::Quad| {
      [q.ul, q.ur, q.ll, q.lr]
        .map(|p| ((p.x * 10.0).round(), (p.y * 10.0).round()))
    };
    for (name, up, advance, (asc, desc), want) in [
      (
        "normal metrics stay",
        (0.0, -10.0),
        (6.0, 0.0),
        (0.9, -0.2),
        (0.9, -0.2),
      ),
      (
        "CJK bbox metrics clamp",
        (0.0, -10.5),
        (10.5, 0.0),
        (1.809, -1.049),
        (1.0, -0.3),
      ),
      (
        "deep math descent clamps",
        (0.0, -10.0),
        (8.0, 0.0),
        (0.775, -0.96),
        (0.775, -0.3),
      ),
      (
        "narrow text keeps its em",
        (0.0, -10.0),
        (4.0, 0.0),
        (1.809, -1.049),
        (1.0, -0.3),
      ),
      (
        "rotated text keeps its direction",
        (-10.0, 0.0),
        (0.0, -10.0),
        (1.809, -1.049),
        (1.0, -0.3),
      ),
    ] {
      let got = clamp_quad(
        quad(up, advance, asc, desc),
        point((100.0, 200.0)),
        asc,
        desc,
      );
      let want = quad(up, advance, want.0, want.1);
      assert_eq!(corners(got), corners(want), "{name}");
    }
  }

  #[test]
  fn tall_font_metrics_stay_on_their_line() -> Result<()> {
    let content =
      "BT /F 10.5 Tf 20 180 Td (Line one) Tj 0 -16 Td (Line two) Tj ET";
    let pdf = format!(
      "%PDF-1.4\n1 0 obj <</Type/Catalog/Pages 2 0 R>> endobj\n\
       2 0 obj <</Type/Pages/Kids[3 0 R]/Count 1>> endobj\n\
       3 0 obj <</Type/Page/Parent 2 0 R/MediaBox[0 0 300 200]\
       /Resources<</Font<</F 5 0 R>>>>/Contents 4 0 R>> endobj\n\
       4 0 obj <</Length {}>> stream\n{content}\nendstream endobj\n\
       5 0 obj <</Type/Font/Subtype/Type1/BaseFont/Helvetica\
       /FontDescriptor 6 0 R>> endobj\n\
       6 0 obj <</Type/FontDescriptor/FontName/Helvetica/Flags 32\
       /FontBBox[0 -1049 1000 1809]/Ascent 1809/Descent -1049\
       /ItalicAngle 0/StemV 80/CapHeight 700>> endobj\n\
       trailer <</Root 1 0 R>>\n%%EOF\n",
      content.len()
    );
    let file = tempfile::NamedTempFile::new()?;
    std::fs::write(file.path(), pdf)?;
    let doc = mupdf::Document::open(file.path().to_str().unwrap_or_default())?;
    let page = doc.load_page(0)?;
    let raw = page.to_text_page(TEXT_FLAGS)?;
    let mut tall = false;
    for block in raw.blocks() {
      for line in block.lines() {
        for glyph in line.chars() {
          let q = glyph.quad();
          tall |= q.ll.y - q.ul.y > 2.0 * glyph.size();
        }
      }
    }
    assert!(tall, "fixture must reproduce oversized quads");
    let text = extract(&page)?;
    let [one, two] = &text.lines[..] else {
      panic!("expected two lines, got {}", text.lines.len());
    };
    let bottom =
      one.glyphs.iter().map(|g| g.bounds.y1).fold(f32::MIN, f32::max);
    let top = two.glyphs.iter().map(|g| g.bounds.y0).fold(f32::MAX, f32::min);
    assert!(
      top > bottom,
      "line two top {top} overlaps line one bottom {bottom}"
    );
    Ok(())
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
