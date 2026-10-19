use crate::Result;
use crate::document::TEXT_FLAGS;
use crate::layout::clip;
use crate::text::{Bounds, Text};
use mupdf::text_page::TextBlockType;
use mupdf::{DisplayList, TextPageFlags};

#[derive(Clone, Debug)]
pub struct Figure {
  pub bounds: Bounds,
  pub caption: String,
  pub source: Source,
}

/// What a figure was found from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
  /// Image or vector blocks.
  Graphic,
  /// A caption line alone.
  Caption,
}

pub(crate) fn detect(
  list: &DisplayList,
  text: &Text,
  page: Bounds,
) -> Result<Vec<Figure>> {
  let flags = TEXT_FLAGS
    | TextPageFlags::PRESERVE_IMAGES
    | TextPageFlags::COLLECT_VECTORS;
  let raw = list.to_text_page(flags)?;
  let primitives = raw.blocks().filter_map(|block| {
    primitive(block.r#type(), crate::document::rect(block.bounds()), page)
  });
  Ok(group(primitives, text, page))
}

fn primitive(
  kind: TextBlockType,
  bounds: Bounds,
  page: Bounds,
) -> Option<Bounds> {
  let bounds = clip(bounds, page)?;
  match kind {
    TextBlockType::Image => Some(bounds),
    TextBlockType::Vector
      if bounds.width() * bounds.height()
        < page.width() * page.height() * 0.95 =>
    {
      Some(bounds)
    }
    _ => None,
  }
}

fn group(
  primitives: impl Iterator<Item = Bounds>,
  text: &Text,
  page: Bounds,
) -> Vec<Figure> {
  let mut figures: Vec<Figure> = Vec::new();
  for mut bounds in primitives {
    let mut index = 0;
    while index < figures.len() {
      if near(bounds, figures[index].bounds, 3.0) {
        bounds = bounds.union(figures.swap_remove(index).bounds);
        index = 0;
      } else {
        index += 1;
      }
    }
    figures.push(Figure {
      bounds,
      caption: String::new(),
      source: Source::Graphic,
    });
  }
  for line in &text.lines {
    let caption = line.text();
    if !is_caption(&caption) {
      continue;
    }
    let bounds = line.glyphs.iter().filter_map(|g| clip(g.bounds, page));
    let Some(bounds) = bounds.reduce(Bounds::union) else { continue };
    let nearest = figures
      .iter()
      .enumerate()
      .filter(|(_, figure)| figure.caption.is_empty())
      .filter_map(|(index, figure)| {
        let b = figure.bounds;
        if b.x0 >= bounds.x1 || b.x1 <= bounds.x0 {
          return None;
        }
        let gap = (b.y0 - bounds.y1).max(bounds.y0 - b.y1).max(0.0);
        (gap <= 48.0).then_some((index, gap))
      })
      .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((index, _)) = nearest {
      figures[index].bounds = figures[index].bounds.union(bounds);
      figures[index].caption = caption;
    } else {
      figures.push(Figure { bounds, caption, source: Source::Caption });
    }
  }
  figures.retain(|figure| {
    !figure.caption.is_empty()
      || (figure.bounds.width() >= 4.0 && figure.bounds.height() >= 4.0)
  });
  figures.sort_by(|a, b| {
    a.bounds
      .y0
      .total_cmp(&b.bounds.y0)
      .then(a.bounds.x0.total_cmp(&b.bounds.x0))
  });
  figures
}

fn is_caption(text: &str) -> bool {
  let mut words = text.split_whitespace();
  let Some(prefix) = words.next() else { return false };
  (prefix.eq_ignore_ascii_case("figure")
    || prefix.eq_ignore_ascii_case("table"))
    && words
      .next()
      .is_some_and(|word| word.starts_with(|c: char| c.is_ascii_digit()))
}

fn near(a: Bounds, b: Bounds, gap: f32) -> bool {
  a.x0 <= b.x1 + gap
    && a.x1 + gap >= b.x0
    && a.y0 <= b.y1 + gap
    && a.y1 + gap >= b.y0
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Glyph, Line};

  fn bounds(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds {
    Bounds { x0, y0, x1, y1 }
  }

  #[test]
  fn isolated_rules_are_filtered() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    for (height, expected) in [(1.0, 0), (3.985, 0), (4.0, 1)] {
      let figures = group(
        [bounds(20.0, 20.0, 500.0, 20.0 + height)].into_iter(),
        &Text::default(),
        page,
      );
      assert_eq!(figures.len(), expected);
    }
    let connected =
      [bounds(20.0, 20.0, 500.0, 22.5), bounds(20.0, 20.0, 22.5, 100.0)];
    assert_eq!(group(connected.into_iter(), &Text::default(), page).len(), 1);
  }

  #[test]
  fn primitives_attach_captions() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let text = Text {
      lines: ["Figure 1: chart", "Table 2. caption"]
        .into_iter()
        .enumerate()
        .map(|(i, s)| Line {
          paragraph: true,
          glyphs: s
            .chars()
            .enumerate()
            .map(|(j, ch)| Glyph {
              ch,
              bounds: bounds(
                20.0 + j as f32 * 5.0,
                150.0 + i as f32 * 300.0,
                25.0 + j as f32 * 5.0,
                160.0 + i as f32 * 300.0,
              ),
            })
            .collect(),
        })
        .collect(),
      truncated: false,
    };
    let primitives = [
      page,
      bounds(0.0, 0.0, 500.0, 1.0),
      bounds(20.0, 20.0, 50.0, 120.0),
      bounds(52.0, 20.0, 150.0, 120.0),
      bounds(300.0, 20.0, 400.0, 120.0),
    ];
    let figures = group(
      primitives
        .into_iter()
        .filter_map(|bounds| primitive(TextBlockType::Vector, bounds, page)),
      &text,
      page,
    );
    assert!(primitive(TextBlockType::Image, page, page).is_some());
    let lines = [bounds(0.0, 0.0, 100.0, 1.0), bounds(0.0, 1.0, 1.0, 80.0)];
    let grouped = group(lines.into_iter(), &Text::default(), page);
    assert_eq!(grouped.len(), 1);
    assert_eq!(grouped[0].bounds.height(), 80.0);
    assert_eq!(figures.len(), 3);
    assert_eq!(figures[0].caption, "Figure 1: chart");
    assert_eq!(figures[0].bounds.y1, 160.0);
    assert_eq!(figures[1].caption, "");
    assert_eq!(figures[2].caption, "Table 2. caption");
    for (s, want) in [
      ("Figure 12(a)", true),
      ("table 2", true),
      ("Figurehead 2", false),
      ("Figure text", false),
    ] {
      assert_eq!(is_caption(s), want, "{s}");
    }
  }
}
