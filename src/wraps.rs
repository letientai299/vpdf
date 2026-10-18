//! Rejoins addresses split across lines, columns, pages.

use crate::document::{Document, Link, Target};
use crate::text::{Bounds, PageId, Text};

/// The address a part may spell.
fn address(target: &Target) -> Option<&str> {
  match target {
    Target::Uri(uri) => Some(uri),
    // Repeated citations would look wrapped.
    Target::Page(_) | Target::Point { .. } => None,
  }
}

/// A part's text without spaces.
fn spelled(text: &Text, page: PageId, link: &Link) -> String {
  link.anchor(text, page).split_whitespace().collect()
}

/// Whether `after` follows `before` in `uri`.
fn runs_on(uri: &str, before: &str, after: &str) -> bool {
  !before.is_empty()
    && !after.is_empty()
    && uri.contains(&[before, after].concat())
}

/// Whether `run` opens `uri`, scheme optional.
fn opens(uri: &str, run: &str) -> bool {
  let bare = uri.split_once(':').map_or(uri, |(_, rest)| rest);
  uri.starts_with(run) || bare.trim_start_matches('/').starts_with(run)
}

/// Gap from `before`'s end to `after`'s start.
fn gap(before: Bounds, after: Bounds) -> f32 {
  (after.y0 - before.y1).abs() + (after.x0 - before.x1).abs()
}

/// Joins each address's parts on one page.
pub(crate) fn join(links: Vec<Link>, text: &Text, page: PageId) -> Vec<Link> {
  // Only repeated addresses can join.
  let repeated = |link: &Link| {
    address(&link.target).is_some()
      && links
        .iter()
        .filter(|other| other.target == link.target)
        .nth(1)
        .is_some()
  };
  let runs: Vec<_> =
    links
      .iter()
      .map(|link| {
        if repeated(link) { spelled(text, page, link) } else { String::new() }
      })
      .collect();
  let mut parts: Vec<_> = links.into_iter().zip(runs).map(Some).collect();
  let mut joined = Vec::with_capacity(parts.len());
  for at in 0..parts.len() {
    let Some((mut link, mut run)) = parts[at].take() else { continue };
    let Some(uri) = address(&link.target).map(str::to_owned) else {
      joined.push(link);
      continue;
    };
    // Any order; the nearest part joins.
    while let Some((_, index, after)) = (parts.iter().enumerate())
      .filter_map(|(index, part)| {
        let (other, more) = part.as_ref()?;
        if other.target != link.target {
          return None;
        }
        if runs_on(&uri, &run, more) {
          return Some((gap(link.last(), other.first()), index, true));
        }
        let before = runs_on(&uri, more, &run);
        before.then(|| (gap(other.last(), link.first()), index, false))
      })
      .min_by(|a, b| a.0.total_cmp(&b.0))
    {
      let (part, more) = parts[index].take().expect("a found part");
      if after {
        run += &more;
        link.append(part);
      } else {
        run = more + &run;
        link.prepend(part);
      }
    }
    joined.push(link);
  }
  joined
}

/// `link`'s parts on the pages beside `page`.
pub(crate) fn page_parts(
  doc: &mut Document,
  page: PageId,
  link: &Link,
) -> Vec<(PageId, Bounds)> {
  let Some(uri) = address(&link.target) else { return Vec::new() };
  // Hover is advisory; failures find nothing.
  let Ok(text) = doc.text(page) else { return Vec::new() };
  let run = spelled(&text, page, link);
  // Prose and whole addresses stop here.
  if run.is_empty() || !uri.contains(&run) {
    return Vec::new();
  }
  let next = (page.0 + 1 < doc.pages.len() && !uri.ends_with(&run))
    .then(|| PageId(page.0 + 1));
  let previous =
    page.0.checked_sub(1).filter(|_| !opens(uri, &run)).map(PageId);
  let mut found = Vec::new();
  for other in [previous, next].into_iter().flatten() {
    let (Ok(text), Ok(links)) = (doc.text(other), doc.links(other)) else {
      continue;
    };
    let after = Some(other) == next;
    let parts = links.iter().filter(|part| {
      if part.target != link.target {
        return false;
      }
      let more = spelled(&text, other, part);
      let (before, then) = if after { (&run, &more) } else { (&more, &run) };
      runs_on(uri, before, then)
    });
    found.extend(parts.flat_map(Link::boxes).map(|bounds| (other, bounds)));
  }
  found
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::text::{Glyph, Line};

  /// `word` in 10 pt glyphs at `(x, y)`.
  fn line(word: &str, x: f32, y: f32) -> Line {
    let glyphs = word.chars().enumerate().map(|(index, ch)| {
      let x0 = x + 10.0 * index as f32;
      let bounds = Bounds { x0, y0: y, x1: x0 + 10.0, y1: y + 12.0 };
      Glyph { ch, bounds }
    });
    Line { paragraph: false, glyphs: glyphs.collect() }
  }

  /// Parts (text, x, y), then joined tops.
  type Case<'a> = (&'a [(&'a str, f32, f32)], &'a [&'a [f32]]);

  #[test]
  fn parts_join_when_their_text_runs_on() {
    let uri = Target::Uri("https://a.org/long/path".into());
    let cases: [Case; 8] = [
      // A line wrap, in either order.
      (
        &[("https://a.org/", 300.0, 100.0), ("long/path", 20.0, 120.0)],
        &[&[100.0, 120.0]],
      ),
      (
        &[("long/path", 20.0, 120.0), ("https://a.org/", 300.0, 100.0)],
        &[&[100.0, 120.0]],
      ),
      // Bottom left, then top right.
      (
        &[("https://a.org/lo", 20.0, 500.0), ("ng/path", 300.0, 40.0)],
        &[&[500.0, 40.0]],
      ),
      // Separate mentions stay apart.
      (
        &[
          ("https://a.org/long/path", 20.0, 300.0),
          ("https://a.org/long/path", 300.0, 40.0),
        ],
        &[&[300.0], &[40.0]],
      ),
      // Table rows of one address.
      (
        &[
          ("a.org", 20.0, 100.0),
          ("a.org", 20.0, 113.0),
          ("a.org", 20.0, 126.0),
        ],
        &[&[100.0], &[113.0], &[126.0]],
      ),
      // Prose anchors stay apart.
      (
        &[("theproject", 300.0, 100.0), ("website", 20.0, 120.0)],
        &[&[100.0], &[120.0]],
      ),
      (
        &[
          ("https://a.org/", 300.0, 100.0),
          ("long/", 20.0, 120.0),
          ("path", 20.0, 140.0),
        ],
        &[&[100.0, 120.0, 140.0]],
      ),
      // Two equal splits, unsorted: nearest parts pair.
      (
        &[
          ("long/path", 20.0, 320.0),
          ("https://a.org/", 300.0, 100.0),
          ("long/path", 20.0, 120.0),
          ("https://a.org/", 300.0, 300.0),
        ],
        &[&[300.0, 320.0], &[100.0, 120.0]],
      ),
    ];
    for (step, (parts, want)) in cases.into_iter().enumerate() {
      let lines = parts.iter().map(|(word, x, y)| line(word, *x, *y));
      let text = Text { lines: lines.collect(), truncated: false };
      let made = text.lines.iter().map(|line| {
        let bounds = line.glyphs.iter().map(|glyph| glyph.bounds);
        Link::new(bounds.reduce(Bounds::union).expect("glyphs"), uri.clone())
      });
      let joined = join(made.collect(), &text, PageId(0));
      let tops: Vec<Vec<f32>> = joined
        .iter()
        .map(|link| link.boxes().map(|bounds| bounds.y0).collect())
        .collect();
      assert_eq!(tops, want, "case {step}");
    }
  }

  #[test]
  fn runs_open_addresses_without_schemes() {
    // (address, run) -> opens
    let cases = [
      ("mailto:a@b.org", "a@b.org", true),
      ("https://x.org/p", "x.org/", true),
      ("https://x.org/p", "https://x.", true),
      ("https://x.org/p", "org/p", false),
    ];
    for (uri, run, want) in cases {
      assert_eq!(opens(uri, run), want, "{uri} {run}");
    }
  }

  #[test]
  fn citations_never_join() {
    let cite = Target::Page(PageId(3));
    let text = Text { lines: vec![line("[1]", 20.0, 100.0)], truncated: false };
    let link = Link::new(text.lines[0].glyphs[0].bounds, cite);
    let joined = join(vec![link.clone(), link], &text, PageId(0));
    assert_eq!(joined.len(), 2);
  }
}
