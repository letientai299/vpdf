use crate::text::{Bounds, Text};
use vpdf_model::text::Quad;

const WORDS: usize = 32;
const TOLERANCE: f32 = 0.004;

pub(super) fn apply(text: &mut Text, words: &[Vec<Bounds>], page: Bounds) {
  let width = if page.width() > 0.0 {
    page.width()
  } else {
    words
      .iter()
      .flatten()
      .copied()
      .reduce(Bounds::union)
      .map_or(0.0, Bounds::width)
  };
  let mut estimates: Vec<_> =
    words.iter().filter_map(|line| estimate(line, width)).collect();
  let slope = consensus(&mut estimates);
  for line in &mut text.lines {
    if line.glyphs.iter().any(|g| g.bounds.x1 < g.bounds.x0)
      || line.glyphs.windows(2).any(|p| p[1].bounds.x0 < p[0].bounds.x0)
    {
      continue;
    }
    let Some(bounds) = line.span() else { continue };
    let mut top = f32::INFINITY;
    let mut bottom = f32::NEG_INFINITY;
    for glyph in &line.glyphs {
      let b = glyph.bounds;
      top = top.min(b.y0 - slope * b.x0).min(b.y0 - slope * b.x1);
      bottom = bottom.max(b.y1 - slope * b.x0).max(b.y1 - slope * b.x1);
    }
    line.ocr = Some(Quad([
      (bounds.x0, top + slope * bounds.x0),
      (bounds.x1, top + slope * bounds.x1),
      (bounds.x1, bottom + slope * bounds.x1),
      (bounds.x0, bottom + slope * bounds.x0),
    ]));
  }
}

fn estimate(words: &[Bounds], width: f32) -> Option<f32> {
  if words.len() < 6 || width <= 0.0 {
    return None;
  }
  let span = words.iter().copied().reduce(Bounds::union)?;
  if span.width() < width * 0.23 {
    return None;
  }
  let count = words.len().min(WORDS);
  let samples: Vec<_> =
    (0..count).map(|i| words[i * (words.len() - 1) / (count - 1)]).collect();
  let mut slopes = Vec::with_capacity(count * (count - 1) / 2);
  for (i, a) in samples.iter().enumerate() {
    for b in &samples[i + 1..] {
      let dx = b.centre().0 - a.centre().0;
      if dx > width * 0.13 {
        slopes.push((b.y1 - a.y1) / dx);
      }
    }
  }
  (!slopes.is_empty()).then(|| median(&mut slopes))
}

fn consensus(estimates: &mut [f32]) -> f32 {
  if estimates.len() < 3 {
    return 0.0;
  }
  let slope = median(estimates);
  let support =
    estimates.iter().filter(|&&s| (s - slope).abs() <= TOLERANCE).count();
  if slope.abs() > 0.05
    || slope.abs() < 0.001
    || support * 3 < estimates.len() * 2
  {
    return 0.0;
  }
  slope
}

fn median(values: &mut [f32]) -> f32 {
  let odd = values.len() % 2 == 1;
  let middle = values.len() / 2;
  let (left, middle, _) = values.select_nth_unstable_by(middle, f32::total_cmp);
  if odd {
    *middle
  } else {
    f32::midpoint(
      *left.iter().max_by(|a, b| a.total_cmp(b)).expect("OCR slopes"),
      *middle,
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Glyph, Line};

  fn rows(slopes: &[f32]) -> (Text, Vec<Vec<Bounds>>) {
    let words: Vec<Vec<_>> = slopes
      .iter()
      .enumerate()
      .map(|(row, slope)| {
        (0..10)
          .map(|i| {
            let x = 20.0 + i as f32 * 30.0;
            let y = 100.0 + row as f32 * 20.0 + slope * x;
            Bounds { x0: x, y0: y, x1: x + 20.0, y1: y + 10.0 }
          })
          .collect()
      })
      .collect();
    let text = Text {
      lines: words
        .iter()
        .map(|line| Line {
          glyphs: line
            .iter()
            .map(|&bounds| Glyph { ch: 'x', bounds, quad: None })
            .collect(),
          ..Line::default()
        })
        .collect(),
      truncated: false,
    };
    (text, words)
  }

  #[test]
  fn pools_line_skew() {
    let page = Bounds { x0: 0.0, y0: 0.0, x1: 612.0, y1: 792.0 };
    for slope in [-0.02, 0.02] {
      let (mut text, words) = rows(&[slope, slope, -slope, slope]);
      let before = text.plain();
      apply(&mut text, &words, page);
      assert_eq!(text.plain(), before);
      for (line, words) in text.lines.iter().zip(&words) {
        let q = line.ocr.unwrap();
        let actual = (q.0[1].1 - q.0[0].1) / (q.0[1].0 - q.0[0].0);
        assert!((actual - slope).abs() < 0.0001);
        assert!(line.glyphs.iter().all(|g| g.quad.is_none()));
        assert_eq!(
          line.glyphs.iter().map(|g| g.bounds).collect::<Vec<_>>(),
          *words
        );
      }
    }
  }

  #[test]
  fn weak_estimates_stay_horizontal() {
    for slopes in
      [vec![-0.02], vec![-0.02, 0.02], vec![-0.03, 0.0, 0.03], vec![0.2; 4]]
    {
      let (mut text, words) = rows(&slopes);
      apply(&mut text, &words, Bounds::default());
      assert!(text.lines.iter().all(|line| line.ocr.unwrap().is_rect()));
    }
  }

  #[test]
  fn sampling_covers_long_lines() {
    let words: Vec<_> = (0..10_000)
      .map(|i| {
        let x = i as f32;
        Bounds { x0: x, x1: x + 0.5, y0: 10.0 - 0.02 * x, y1: 20.0 - 0.02 * x }
      })
      .collect();
    assert!((estimate(&words, 10_000.0).unwrap() + 0.02).abs() < 0.0001);
    for (mut values, expected) in
      [(vec![9.0, 1.0, 5.0], 5.0), (vec![9.0, 1.0, 5.0, 3.0], 4.0)]
    {
      assert_eq!(median(&mut values), expected);
    }
  }
}
