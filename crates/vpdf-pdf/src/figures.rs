use crate::Result;
use crate::document::TEXT_FLAGS;
use crate::text::{Bounds, Text};
use mupdf::text_page::TextBlockType;
use mupdf::{DisplayList, TextPageFlags};
use vpdf_model::geometry::clip;

pub use vpdf_model::document::{Figure, FigureSource as Source};

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
  for bounds in primitives {
    let bounds = absorb(&mut figures, bounds, |bounds, figure| {
      near(bounds, figure.bounds, 3.0)
    });
    figures.push(Figure {
      bounds,
      caption: String::new(),
      source: Source::Graphic,
    });
  }
  figures.retain(|figure| {
    let b = figure.bounds;
    !(b.height() < page.height() * 0.03
      && (b.y1 < page.y0 + page.height() * 0.07
        || b.y0 > page.y1 - page.height() * 0.07))
  });
  let mut lines: Vec<_> = text
    .lines
    .iter()
    .filter_map(|line| {
      let bounds = line
        .glyphs
        .iter()
        .filter_map(|g| clip(g.bounds, page))
        .reduce(Bounds::union)?;
      Some((bounds, line.text()))
    })
    .collect();
  lines.sort_by(|a, b| a.0.y0.total_cmp(&b.0.y0));
  for (bounds, caption) in &lines {
    if !is_caption(caption)
      || !text
        .lines
        .iter()
        .any(|line| line.paragraph && line.text() == *caption)
    {
      continue;
    }
    attach_caption(&mut figures, *bounds, caption, &lines, page);
  }
  figures.retain(|figure| {
    !figure.caption.is_empty()
      || (figure.bounds.width() >= 4.0 && figure.bounds.height() >= 4.0)
  });
  for figure in &mut figures {
    for (bounds, _) in &lines {
      let gap = if bounds.y0 > figure.bounds.y1 {
        3.0
      } else {
        (bounds.height() * 1.5).min(12.0)
      };
      if near(figure.bounds, *bounds, gap) {
        figure.bounds = figure.bounds.union(*bounds);
      }
    }
  }
  figures.sort_by(|a, b| {
    a.bounds
      .y0
      .total_cmp(&b.bounds.y0)
      .then(a.bounds.x0.total_cmp(&b.bounds.x0))
  });
  figures
}

fn attach_caption(
  figures: &mut Vec<Figure>,
  bounds: Bounds,
  caption: &str,
  lines: &[(Bounds, String)],
  page: Bounds,
) {
  let Some((bounds, title)) = caption_title(bounds, caption, lines) else {
    return;
  };
  let preceding = lines
    .iter()
    .filter(|(line, value)| {
      line.y0 > page.y0 + page.height() * 0.07
        && line.y1 < bounds.y0
        && !is_caption(value)
        && connection(*line, bounds, lines).is_some()
    })
    .max_by(|a, b| a.0.y1.total_cmp(&b.0.y1));
  let nearest = figures
    .iter()
    .enumerate()
    .filter(|(_, figure)| figure.caption.is_empty())
    .filter_map(|(index, figure)| {
      connection(figure.bounds, bounds, lines).map(|gap| {
        let above = figure.bounds.y0 < bounds.y0;
        let rule = figure.bounds.width() < 4.0 || figure.bounds.height() < 4.0;
        (index, !above, rule, gap)
      })
    })
    .min_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)).then(a.3.total_cmp(&b.3)));
  let nearest =
    nearest.filter(|(_, below, _, _)| !below || preceding.is_none());
  if let Some((index, _, _, _)) = nearest {
    let mut figure = figures.swap_remove(index);
    let above = figure.bounds.y0 < bounds.y0;
    figure.bounds =
      absorb(figures, figure.bounds.union(bounds), |grown, other| {
        other.caption.is_empty()
          && if above {
            other.bounds.y0 < bounds.y0
          } else {
            other.bounds.y1 > bounds.y1
          }
          && connection(grown, other.bounds, lines).is_some()
      });
    figure.caption = title;
    figures.push(figure);
  } else {
    let mut area = preceding.map_or(bounds, |(line, _)| bounds.union(*line));
    for (line, value) in lines.iter().rev() {
      if line.y0 > page.y0 + page.height() * 0.07
        && line.y1 < bounds.y0
        && line.width() < bounds.width() * 0.5
        && !is_caption(value)
        && connection(area, *line, lines).is_some()
      {
        area = area.union(*line);
      }
    }
    figures.push(Figure {
      bounds: area,
      caption: title,
      source: Source::Caption,
    });
  }
}

/// `bounds` grown by each figure `joins` takes, until none joins.
fn absorb(
  figures: &mut Vec<Figure>,
  mut bounds: Bounds,
  joins: impl Fn(Bounds, &Figure) -> bool,
) -> Bounds {
  let mut index = 0;
  while index < figures.len() {
    if joins(bounds, &figures[index]) {
      bounds = bounds.union(figures.swap_remove(index).bounds);
      index = 0;
    } else {
      index += 1;
    }
  }
  bounds
}

fn caption_row(
  bounds: Bounds,
  caption: &str,
  lines: &[(Bounds, String)],
) -> (Bounds, String) {
  let mut row: Vec<_> = lines
    .iter()
    .filter(|(line, value)| {
      line.y0 < bounds.y1
        && line.y1 > bounds.y0
        && (line.y0 - bounds.y0).abs() < bounds.height() * 0.5
        && (value == caption || line.x0 > bounds.x0 + 3.0)
        && (!is_caption(value) || value == caption)
    })
    .collect();
  row.sort_by(|a, b| a.0.x0.total_cmp(&b.0.x0));
  let area = row.iter().fold(bounds, |area, (line, _)| area.union(*line));
  let title =
    row.iter().map(|(_, text)| text.as_str()).collect::<Vec<_>>().join(" ");
  (area, title)
}

fn caption_title(
  bounds: Bounds,
  caption: &str,
  lines: &[(Bounds, String)],
) -> Option<(Bounds, String)> {
  let (mut area, mut title) = caption_row(bounds, caption, lines);
  let contents = title.chars().filter(|&ch| ch == '.').count() >= 12
    && title.split_whitespace().last().is_some_and(|word| {
      let word = word.trim_matches('.');
      word.chars().any(|ch| ch.is_ascii_digit())
        && word.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    });
  if contents {
    return None;
  }
  if title.split_whitespace().count() > 2
    && title.trim_end().ends_with(['.', '!', '?'])
  {
    return Some((area, title));
  }
  let mut description = title.split_whitespace().count() > 2;
  let mut gap = if description { 1.0 } else { 2.0 };
  let mut height = bounds.height();
  for (line, value) in lines.iter().filter(|(line, _)| line.y0 >= bounds.y1) {
    if line.y0 < area.y1 {
      continue;
    }
    if line.y0 - area.y1 > bounds.height() * gap {
      break;
    }
    if (line.x0 - bounds.x0).abs() > 3.0 {
      continue;
    }
    let (row, value) = caption_row(*line, value, lines);
    if (row.x0 - bounds.x0).abs() > 3.0 {
      continue;
    }
    if value.split_whitespace().next().is_some_and(caption_prefix)
      || description
        && (row.height() < height * 0.7 || row.height() > height * 1.4)
    {
      break;
    }
    title.push(' ');
    title.push_str(&value);
    area = area.union(row);
    height = row.height();
    description = true;
    gap = 1.0;
    if value.trim_end().ends_with(['.', '!', '?']) {
      break;
    }
  }
  Some((area, title))
}

fn caption_prefix(word: &str) -> bool {
  word.eq_ignore_ascii_case("figure") || word.eq_ignore_ascii_case("table")
}

fn connection(a: Bounds, b: Bounds, lines: &[(Bounds, String)]) -> Option<f32> {
  if a.x0 >= b.x1 || a.x1 <= b.x0 {
    return None;
  }
  let (upper, lower) = if a.y0 < b.y0 { (a, b) } else { (b, a) };
  let gap = (lower.y0 - upper.y1).max(0.0);
  if gap > 96.0 {
    return None;
  }
  let blocked = lines.iter().any(|(line, text)| {
    line.y0 >= upper.y1
      && line.y1 <= lower.y0
      && line.x0 < a.x1.max(b.x1)
      && line.x1 > a.x0.min(b.x0)
      && (is_caption(text) || line.width() >= a.width().max(b.width()) * 0.5)
  });
  (!blocked).then_some(gap)
}

fn is_caption(text: &str) -> bool {
  let mut words = text.split_whitespace();
  let Some(prefix) = words.next() else { return false };
  if !caption_prefix(prefix) {
    return false;
  }
  let Some(number) = words.next() else { return false };
  let standalone = words.next().is_none();
  let numbered = |value: &str| {
    value.starts_with(|c: char| c.is_ascii_digit())
      && value.ends_with(|c: char| c.is_ascii_digit())
      && value.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-')
  };
  let label = standalone
    && number.ends_with(')')
    && number.split_once('(').is_some_and(|(base, _)| numbered(base));
  label
    || numbered(number.trim_end_matches([':', '.']))
      && (standalone
        || prefix.eq_ignore_ascii_case("table")
        || number.ends_with([':', '.']))
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
        [bounds(20.0, 100.0, 500.0, 100.0 + height)].into_iter(),
        &Text::default(),
        page,
      );
      assert_eq!(figures.len(), expected);
    }
    let connected =
      [bounds(20.0, 20.0, 500.0, 22.5), bounds(20.0, 20.0, 22.5, 100.0)];
    assert_eq!(group(connected.into_iter(), &Text::default(), page).len(), 1);
  }

  fn line(value: &str, area: Bounds) -> Line {
    let count = value.chars().count();
    let width = area.width() / count as f32;
    Line {
      ocr: None,
      paragraph: true,
      glyphs: value
        .chars()
        .enumerate()
        .map(|(index, ch)| Glyph {
          quad: None,
          ch,
          bounds: bounds(
            area.x0 + index as f32 * width,
            area.y0,
            if index + 1 == count {
              area.x1
            } else {
              area.x0 + (index + 1) as f32 * width
            },
            area.y1,
          ),
        })
        .collect(),
    }
  }

  /// Figures on `page` from paragraph `rows` and `primitives`.
  fn grouped(
    page: Bounds,
    rows: &[(&str, Bounds)],
    primitives: &[Bounds],
  ) -> Vec<Figure> {
    let lines = rows.iter().map(|&(value, area)| line(value, area)).collect();
    let text = Text { lines, truncated: false };
    group(primitives.iter().copied(), &text, page)
  }

  #[test]
  fn caption_description_joins_graphic() {
    let page = bounds(0.0, 0.0, 540.0, 666.0);
    let graphic = bounds(130.0, 414.0, 371.0, 521.0);
    let description = bounds(83.0, 544.0, 197.0, 551.0);
    let rows = [
      ("FIGURE 2.3", bounds(83.0, 530.0, 124.0, 537.0)),
      ("Execution of a CUDA program.", description),
    ];
    let figures =
      grouped(page, &rows, &[graphic, bounds(83.0, 540.0, 418.0, 540.4)]);
    assert_eq!(figures.len(), 1, "{figures:?}");
    assert_eq!(figures[0].caption, "FIGURE 2.3 Execution of a CUDA program.");
    assert_eq!(figures[0].bounds, graphic.union(description));
    assert_eq!(figures[0].source, Source::Graphic);
  }

  #[test]
  fn wrapped_descriptions_exclude_body() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let rows = [
      ("FIGURE 4.1", bounds(80.0, 200.0, 130.0, 207.0)),
      ("Architecture of a", bounds(80.0, 215.0, 190.0, 222.0)),
      ("CUDA-capable GPU.", bounds(80.0, 228.0, 190.0, 235.0)),
      ("Body paragraph.", bounds(80.0, 241.0, 500.0, 252.0)),
    ];
    let figures = grouped(page, &rows, &[bounds(150.0, 80.0, 400.0, 190.0)]);
    assert_eq!(figures.len(), 1, "{figures:?}");
    assert_eq!(
      figures[0].caption,
      "FIGURE 4.1 Architecture of a CUDA-capable GPU."
    );
    assert!(figures[0].bounds.y1 < 241.0, "{figures:?}");
  }

  #[test]
  fn split_caption_spans_join() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let rows = [
      ("Figure 4.17:", bounds(80.0, 200.0, 130.0, 210.0)),
      ("Samples from an Ising model with", bounds(135.0, 200.0, 420.0, 210.0)),
      ("varying J > 0.", bounds(80.0, 215.0, 210.0, 225.0)),
    ];
    let figures = grouped(page, &rows, &[bounds(100.0, 80.0, 400.0, 190.0)]);
    assert_eq!(figures.len(), 1, "{figures:?}");
    assert_eq!(
      figures[0].caption,
      "Figure 4.17: Samples from an Ising model with varying J > 0."
    );
  }

  #[test]
  fn contents_rows_are_ignored() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let rows = [
      ("Figure 21-1.", bounds(40.0, 80.0, 100.0, 90.0)),
      (
        "Processor brand . . . . . . . . . . . . . . 21-5",
        bounds(105.0, 80.0, 500.0, 90.0),
      ),
      (
        "Exception handling . . . . . . . . . . . . . . D-4",
        bounds(105.0, 89.0, 500.0, 99.0),
      ),
      ("Figure E-1.", bounds(40.0, 100.0, 100.0, 110.0)),
    ];
    assert!(grouped(page, &rows, &[]).is_empty());
  }

  #[test]
  fn captions_skip_rules() {
    let page = bounds(0.0, 0.0, 540.0, 666.0);
    let graphic = bounds(124.0, 80.0, 454.0, 467.0);
    let caption = bounds(122.0, 476.0, 167.0, 482.0);
    let rows = [("FIGURE 6.13", caption)];
    let figures = grouped(
      page,
      &rows,
      &[
        graphic,
        bounds(122.0, 486.0, 456.0, 486.4),
        bounds(0.0, 17.0, 540.0, 26.0),
        bounds(40.0, 630.0, 490.0, 644.0),
      ],
    );
    assert_eq!(figures.len(), 1, "{figures:?}");
    assert_eq!(figures[0].bounds, graphic.union(caption));
    assert_eq!(figures[0].caption, "FIGURE 6.13");
    assert_eq!(figures[0].source, Source::Graphic);
  }

  #[test]
  fn disconnected_rows_share_caption() {
    let page = bounds(0.0, 0.0, 612.0, 792.0);
    let top = bounds(178.0, 431.0, 475.0, 483.0);
    let bottom = bounds(178.0, 499.0, 475.0, 513.0);
    let rows = [
      ("Figure 1-17. Operation", bounds(226.0, 557.0, 378.0, 569.0)),
      ("SRC1", bounds(158.0, 435.0, 178.0, 443.0)),
      ("DEST", bounds(158.0, 501.0, 178.0, 509.0)),
    ];
    for primitives in [[top, bottom], [bottom, top]] {
      let figures = grouped(page, &rows, &primitives);
      assert_eq!(figures.len(), 1, "{figures:?}");
      assert_eq!(figures[0].bounds, bounds(158.0, 431.0, 475.0, 569.0));
    }
  }

  #[test]
  fn intervening_text_separates_graphics() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let rows = [
      ("Figure 1", bounds(100.0, 190.0, 180.0, 200.0)),
      ("Body text", bounds(100.0, 235.0, 400.0, 245.0)),
      ("Figure 2", bounds(100.0, 340.0, 180.0, 350.0)),
    ];
    let figures = grouped(
      page,
      &rows,
      &[bounds(100.0, 100.0, 400.0, 180.0), bounds(100.0, 220.0, 400.0, 330.0)],
    );
    assert_eq!(figures.len(), 2, "{figures:?}");
    assert_eq!(figures[0].bounds.y1, 200.0);
    assert_eq!(figures[1].bounds.y0, 220.0);
  }

  #[test]
  fn wrapped_references_are_ignored() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let mut reference = line(
      "Figure 9.1. We discuss this later.",
      bounds(100.0, 120.0, 500.0, 130.0),
    );
    reference.paragraph = false;
    let text = Text { lines: vec![reference], truncated: false };
    assert!(group(std::iter::empty(), &text, page).is_empty());
  }

  #[test]
  fn text_diagrams_precede_equations() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let rows = [
      ("Chapter header", bounds(100.0, 25.0, 300.0, 35.0)),
      ("alpha", bounds(120.0, 70.0, 150.0, 80.0)),
      ("beta", bounds(250.0, 120.0, 280.0, 130.0)),
      ("gamma", bounds(400.0, 170.0, 430.0, 180.0)),
      ("Figure 9.4: Messages", bounds(100.0, 200.0, 500.0, 210.0)),
      ("Body text after the diagram", bounds(100.0, 230.0, 500.0, 240.0)),
    ];
    let figures = grouped(page, &rows, &[bounds(200.0, 250.0, 400.0, 290.0)]);
    let figure = figures.iter().find(|f| !f.caption.is_empty()).unwrap();
    assert_eq!(figure.bounds, bounds(100.0, 70.0, 500.0, 210.0));
    assert_eq!(figure.source, Source::Caption);
  }

  #[test]
  fn caption_spans_join_panels() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let rows = [
      ("Figure 2.4: (a) Distribution.", bounds(80.0, 205.0, 260.0, 215.0)),
      ("(b) Distribution on a log scale.", bounds(260.0, 205.0, 500.0, 215.0)),
      ("(a) plot label", bounds(120.0, 175.0, 240.0, 185.0)),
      ("Figure 2.5: Another plot.", bounds(80.0, 350.0, 500.0, 360.0)),
    ];
    let primitives = [
      bounds(100.0, 80.0, 280.0, 150.0),
      bounds(300.0, 80.0, 480.0, 150.0),
      bounds(180.0, 230.0, 400.0, 330.0),
    ];
    let figures = grouped(page, &rows, &primitives);
    assert_eq!(figures.len(), 2, "{figures:?}");
    assert_eq!(figures[0].bounds, bounds(80.0, 80.0, 500.0, 215.0));
    assert!(figures[0].caption.starts_with("Figure 2.4:"));
    assert_eq!(figures[1].bounds, bounds(80.0, 230.0, 500.0, 360.0));
    assert!(figures[1].caption.starts_with("Figure 2.5:"));
  }

  #[test]
  fn primitives_attach_captions() {
    let page = bounds(0.0, 0.0, 600.0, 800.0);
    let text = Text {
      lines: vec![
        line("Figure 1: chart", bounds(20.0, 150.0, 95.0, 160.0)),
        line("Table 2. caption", bounds(20.0, 450.0, 100.0, 460.0)),
      ],
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
      ("Table 1 alone here", true),
      ("Figurehead 2", false),
      ("Figure text", false),
      ("Figure 2.6 shows some plots", false),
      ("Figure 2.10(a) gives an example", false),
      ("Figure 4.3b. This is called a network", false),
      ("Table 3.1).", false),
      ("Figure 2.4: A plot", true),
    ] {
      assert_eq!(is_caption(s), want, "{s}");
    }
  }
}
