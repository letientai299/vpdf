//! Outline inferred from text layout for PDFs that ship none.
//!
//! Headings are short blocks that start with section numbering
//! (`3.2 Title`), or that are larger than nearby body text or bold at body
//! size and precede body text. Running heads, folios, contents pages, numbered
//! exercises, code, and figure labels are ignored.

use super::{OneShot, Wake};
use crate::content_cache::{self, Kind, Record};
use crate::document::{Handle, Heading, LineStyle, Target};
use crate::text::{Line, PageId, Text};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// Larger than body text by this factor counts as a heading size.
const LARGER: f32 = 1.15;
/// Numbered headings may use a font that measures smaller than body text.
const SMALLER: f32 = 0.85;
const MAX_CHARS: usize = 120;
const MAX_WORDS: usize = 16;
/// Wrapped headings span at most this many lines.
const MAX_LINES: usize = 3;
/// Unnumbered headings of one style on a page; more form a list.
const MAX_CROWD: usize = 6;
/// Pages on each side that set a page's body size.
const NEIGHBORS: usize = 3;
/// Wrapped lines within this size ratio join.
const SAME_SIZE: f32 = 0.08;
/// Lines spanning this share of the column are prose.
const FULL_WIDTH: f32 = 0.92;
/// Bold fonts may measure slightly under body size.
const BOLD_BODY: f32 = 0.95;
/// Unnumbered sizes within this ratio share a rank.
const RANK_SPREAD: f32 = 0.92;
/// Largest folio read as a roman numeral.
const ROMAN_MAX: i64 = 100;

struct Candidate {
  page: PageId,
  y: f32,
  size: f32,
  title: String,
  /// Dots in leading numbering plus one; `None` without numbering.
  level: Option<usize>,
  bold: bool,
}

/// Median glyph height of the visible characters.
fn size(line: &Line) -> Option<f32> {
  let mut heights: Vec<f32> = line
    .glyphs
    .iter()
    .filter(|glyph| !glyph.ch.is_whitespace())
    .map(|glyph| glyph.bounds.height())
    .collect();
  if heights.is_empty() {
    return None;
  }
  heights.sort_by(f32::total_cmp);
  Some(heights[heights.len() / 2])
}

/// Section depth from a title's `1`, `2.3`, `A.1`, or `IV.` numbering.
fn numbering(title: &str) -> Option<usize> {
  let (number, rest) = title.split_once(' ')?;
  let rest = rest.trim_start();
  if rest.chars().next().is_none_or(char::is_lowercase) {
    return None;
  }
  // Initials start names: "J. C. Brewer".
  let initial = |w: &str| {
    w.len() == 2 && w.ends_with('.') && w.starts_with(char::is_uppercase)
  };
  if initial(number) && rest.split(' ').next().is_some_and(initial) {
    return None;
  }
  depth(number)
}

/// Depth of one section label.
fn depth(number: &str) -> Option<usize> {
  // A lone undotted letter reads as a word: "A Bayesian approach".
  if number.len() == 1 && number.chars().all(char::is_alphabetic) {
    return None;
  }
  let number = number.trim_end_matches('.');
  let parts: Vec<_> = number.split('.').collect();
  let arabic = |part: &&str| part.chars().all(|c| c.is_ascii_digit());
  let valid = parts.iter().all(|p| !p.is_empty() && p.len() <= 3)
    && (parts.iter().all(arabic) && parts[0].len() <= 2
      || parts[0].len() == 1
        && parts[0].chars().all(|c| c.is_ascii_uppercase())
        && parts[1..].iter().all(arabic)
      || parts.len() == 1 && parts[0].chars().all(|c| "IVX".contains(c)));
  valid.then_some(parts.len())
}

/// A canonical roman numeral up to [`ROMAN_MAX`] in one case: "xii",
/// "IV", not "civil" or "mix".
fn roman(word: &str) -> bool {
  const DIGITS: [(i64, &str); 9] = [
    (100, "c"),
    (90, "xc"),
    (50, "l"),
    (40, "xl"),
    (10, "x"),
    (9, "ix"),
    (5, "v"),
    (4, "iv"),
    (1, "i"),
  ];
  let lower = word.to_lowercase();
  let cased = word == lower || word == word.to_uppercase();
  let value = |c| match c {
    'i' => Some(1),
    'v' => Some(5),
    'x' => Some(10),
    'l' => Some(50),
    'c' => Some(100),
    'd' => Some(500),
    'm' => Some(1000),
    _ => None,
  };
  let Some(values) = lower.chars().map(value).collect::<Option<Vec<i64>>>()
  else {
    return false;
  };
  let total = values.iter().enumerate().fold(0, |total, (at, v)| {
    if values.get(at + 1).is_some_and(|next| next > v) {
      total - v
    } else {
      total + v
    }
  });
  let mut rest = total;
  let mut canonical = String::new();
  for (amount, digits) in DIGITS {
    while rest >= amount {
      canonical.push_str(digits);
      rest -= amount;
    }
  }
  cased && (1..=ROMAN_MAX).contains(&total) && canonical == lower
}

/// Half-point size classes shared by body, running-head, and crowd counts.
fn bucket(size: f32) -> u32 {
  (size * 2.0).round() as u32
}

fn plausible(title: &str, pages: usize) -> bool {
  let letters = title.chars().filter(|c| c.is_alphabetic()).count();
  // Figure labels and folios: "A B", "xii".
  let word = title
    .split_whitespace()
    .any(|w| w.chars().filter(|c| c.is_alphabetic()).count() >= 2);
  let code = title.contains(['{', '}', ';', '=', '<', '>'])
    || title.starts_with('.')
    || ["//", "::", "()"].iter().any(|s| title.contains(s))
    // Identifiers stand alone: "launch__grid_size", not "The __str__ method".
    || !title.contains(' ') && title.contains("__")
    // Command-line flags, unlike dashes: "--metrics".
    || title
      .match_indices("--")
      .any(|(at, _)| title[at + 2..].starts_with(char::is_alphabetic));
  let caption = title.split_once(' ').is_some_and(|(first, rest)| {
    ["Figure", "Fig.", "Table", "Algorithm", "Listing"].contains(&first)
      && rest.starts_with(|c: char| c.is_ascii_alphanumeric())
  });
  // Numbered exercises run on into sentences: "constraint. Express".
  let parts: Vec<_> = title.split(". ").collect();
  let sentence = parts[..parts.len() - 1].iter().any(|before| {
    before.chars().rev().take(3).filter(|c| c.is_lowercase()).count() == 3
  });
  title.chars().count() <= MAX_CHARS
    && title.split_whitespace().count() <= MAX_WORDS
    && letters * 2 >= title.chars().filter(|c| !c.is_whitespace()).count()
    && word
    && !roman(title)
    && !code
    && !caption
    && !sentence
    // Footnote marks end author lines: "Aidan N. Gomez∗†".
    && !title.ends_with(['.', ',', ';', ':', '-', '∗', '†', '‡', '§', '*'])
    // Contents pages list titles with trailing page numbers; TeX sets
    // "Chapter 1" above the title it names.
    && !title.rsplit_once(' ').is_some_and(|(_, last)| {
      last.parse::<usize>().is_ok_and(|n| n <= pages)
        || roman(last) && last == last.to_lowercase()
    })
}

/// What inference needs from one extracted block of same-size lines.
#[derive(Debug)]
pub struct Summary {
  title: String,
  style: LineStyle,
  y: f32,
  /// Widest line, in points.
  width: f32,
  paragraph: bool,
  glyphs: usize,
  lines: usize,
  /// Among the first or last two lines of its page.
  edge: bool,
}

/// Per-page block summaries, so whole-document inference need not keep
/// every page's glyphs. Wrapped lines of one size join; a bare section
/// number joins the title beside it.
pub fn summarize(text: &Text, styles: &[LineStyle]) -> Vec<Summary> {
  let words =
    |line: &Line| line.text().split_whitespace().collect::<Vec<_>>().join(" ");
  let count = text.lines.len();
  let mut out: Vec<Summary> = Vec::new();
  let mut lines = text.lines.iter().enumerate().peekable();
  while let Some((index, line)) = lines.next() {
    // Glyph heights stand in when fonts report no size.
    let mut style = styles.get(index).copied().unwrap_or_default();
    if !(style.size.is_finite() && style.size > 0.0) {
      let Some(size) = size(line) else { continue };
      style.size = size;
    }
    let edge = index < 2 || index + 2 >= count;
    if !line.paragraph
      && let Some(last) = out.last_mut()
      && (style.size / last.style.size - 1.0).abs() < SAME_SIZE
      && (last.style.bold, last.style.mono) == (style.bold, style.mono)
    {
      if last.title.chars().count() <= MAX_CHARS {
        last.title.push(' ');
        last.title.push_str(&words(line));
      }
      last.style.mixed |= style.mixed;
      last.glyphs += line.glyphs.len();
      last.width = last.width.max(width(line));
      last.lines += 1;
      last.edge |= edge;
      continue;
    }
    let mut title = words(line);
    let mut glyphs = line.glyphs.len();
    let mut span = width(line);
    if !title.contains(' ')
      && depth(&title).is_some()
      && let Some((next_index, next)) = lines.peek()
      && let (Some(a), Some(b)) = (line.glyphs.first(), next.glyphs.first())
      && (a.bounds.y1 - b.bounds.y1).abs() < a.bounds.height() / 2.0
    {
      title = format!("{title} {}", words(next));
      glyphs += next.glyphs.len();
      span = span.max(width(next));
      // A bold exercise number beside regular text: "**18** (a) For".
      let joined = styles.get(*next_index).copied().unwrap_or_default();
      style.mixed |= joined.mixed || !joined.mono && joined.bold != style.bold;
      lines.next();
    }
    out.push(Summary {
      title,
      style,
      y: line.glyphs.first().map_or(0.0, |g| g.bounds.y0),
      width: span,
      paragraph: line.paragraph,
      glyphs,
      lines: 1,
      edge,
    });
  }
  out
}

fn width(line: &Line) -> f32 {
  let x0 =
    line.glyphs.iter().map(|g| g.bounds.x0).fold(f32::INFINITY, f32::min);
  let x1 =
    line.glyphs.iter().map(|g| g.bounds.x1).fold(f32::NEG_INFINITY, f32::max);
  (x1 - x0).max(0.0)
}

/// Median width of a page's text lines, the column measure.
fn measure(page: &[Summary]) -> Option<f32> {
  let mut widths: Vec<f32> = page
    .iter()
    .filter(|line| line.glyphs >= 40)
    .map(|line| line.width)
    .collect();
  widths.sort_by(f32::total_cmp);
  widths.get(widths.len() / 2).copied()
}

fn letters(title: &str) -> String {
  title.chars().filter(|c| c.is_alphabetic()).collect()
}

/// Title without its numbering, for matching contents entries.
fn key(title: &str) -> String {
  let title = match title.split_once(' ') {
    Some((_, rest)) if numbering(title).is_some() => rest,
    _ => title,
  };
  title
    .chars()
    .filter(|c| c.is_alphanumeric())
    .collect::<String>()
    .to_lowercase()
}

/// Running heads keep their words while folios change: "CONTENTS xiii".
fn head_key(line: &Summary) -> (String, u32) {
  let words = line
    .title
    .split_whitespace()
    .filter(|w| !w.chars().all(|c| c.is_ascii_digit()) && !roman(w))
    .map(letters)
    .collect();
  (words, bucket(line.style.size))
}

/// Most common size near each page, so appended documents with another
/// body size keep their own headings.
fn bodies(pages: &[Vec<Summary>]) -> Vec<Option<f32>> {
  let histograms: Vec<HashMap<u32, usize>> = pages
    .iter()
    .map(|page| {
      let mut sizes = HashMap::new();
      for line in page {
        *sizes.entry(bucket(line.style.size)).or_default() += line.glyphs;
      }
      sizes
    })
    .collect();
  (0..pages.len())
    .map(|index| {
      let near = index.saturating_sub(NEIGHBORS)
        ..(index + NEIGHBORS + 1).min(pages.len());
      let mut sizes: HashMap<u32, usize> = HashMap::new();
      for histogram in &histograms[near] {
        for (size, count) in histogram {
          *sizes.entry(*size).or_default() += count;
        }
      }
      sizes
        .into_iter()
        .max_by_key(|(size, count)| (*count, *size))
        .map(|(size, _)| size as f32 / 2.0)
    })
    .collect()
}

/// Running heads repeat in one size at page edges, often or on nearby
/// pages.
fn running(pages: &[Vec<Summary>]) -> HashSet<(String, u32)> {
  let mut seen: HashMap<(String, u32), Vec<usize>> = HashMap::new();
  for (index, page) in pages.iter().enumerate() {
    for line in page.iter().filter(|line| line.edge) {
      let at = seen.entry(head_key(line)).or_default();
      if at.last() != Some(&index) {
        at.push(index);
      }
    }
  }
  let often = (pages.len() / 3).max(2);
  seen
    .into_iter()
    .filter(|(_, at)| {
      at.len() >= often
        || at.len() >= 3
          && at.windows(2).any(|w| (1..=2).contains(&(w[1] - w[0])))
    })
    .map(|(key, _)| key)
    .collect()
}

/// Headings precede body text soon; figure labels precede more labels.
fn introduces(rest: &[Summary], body: f32) -> bool {
  let mut rest = rest
    .iter()
    .filter(|line| !(line.edge && line.glyphs < 8))
    .take(3)
    .peekable();
  rest.peek().is_none()
    || rest.any(|line| line.style.size < body * LARGER && line.glyphs >= 20)
}

pub fn infer(pages: &[Vec<Summary>]) -> Vec<Heading> {
  let bodies = bodies(pages);
  let running = running(pages);
  // Body-size numbered lines whose labels repeat with other titles are
  // exercises; proceedings repeat "1 Introduction".
  let mut labels: HashMap<&str, HashSet<&str>> = HashMap::new();
  for (index, page) in pages.iter().enumerate() {
    let Some(body) = bodies[index] else { continue };
    for line in page.iter().filter(|line| {
      line.paragraph
        && (body * SMALLER..body * LARGER).contains(&line.style.size)
    }) {
      if numbering(&line.title).is_some()
        && let Some((label, rest)) = line.title.split_once(' ')
      {
        labels.entry(label).or_default().insert(rest);
      }
    }
  }
  let mut candidates = Vec::new();
  for (index, page) in pages.iter().enumerate() {
    let Some(body) = bodies[index] else { continue };
    let measure = measure(page);
    for (at, line) in page.iter().enumerate() {
      // Headings keep one style; labels run into body text.
      if !line.title.contains(char::is_alphabetic)
        || line.style.mixed
        || line.lines > MAX_LINES
        || !plausible(&line.title, pages.len())
        || line.edge && running.contains(&head_key(line))
      {
        continue;
      }
      let larger = line.style.size >= body * LARGER;
      // Long body-text lines filling the column are prose.
      let prose = !larger
        && !line.style.bold
        && line.title.split_whitespace().count() >= 8
        && measure.is_some_and(|measure| line.width >= measure * FULL_WIDTH);
      if prose {
        continue;
      }
      let repeated = |label: &str| {
        !larger && labels.get(label).is_some_and(|titles| titles.len() >= 3)
      };
      let level = numbering(&line.title).filter(|_| {
        line.paragraph
          && line.style.size >= body * SMALLER
          && !line
            .title
            .split_once(' ')
            .is_some_and(|(label, _)| repeated(label))
      });
      let bold = line.style.bold
        && !line.style.mono
        && line.paragraph
        && line.style.size >= body * BOLD_BODY;
      // Unnumbered lowercase words label figures and tables: "idx".
      let named = line.title.starts_with(|c: char| c.is_ascii_digit())
        || line
          .title
          .chars()
          .any(|c| c.is_uppercase() || c.is_alphabetic() && !c.is_ascii());
      if level.is_none()
        && !(named && (larger || bold) && introduces(&page[at + 1..], body))
      {
        continue;
      }
      candidates.push(Candidate {
        page: PageId(index),
        y: line.y,
        size: line.style.size,
        title: line.title.clone(),
        level,
        bold: line.style.bold,
      });
    }
  }
  depths(distinct(candidates, pages.len()))
}

/// Drops contents entries that reappear as headings later, titles
/// repeated on adjacent pages, and lists or figure labels sharing one
/// style many times on a page.
fn distinct(candidates: Vec<Candidate>, pages: usize) -> Vec<Candidate> {
  let crowd = |c: &Candidate| (c.page.0, bucket(c.size), c.bold);
  let mut crowds: HashMap<(usize, u32, bool), usize> = HashMap::new();
  for c in candidates.iter().filter(|c| c.level.is_none()) {
    *crowds.entry(crowd(c)).or_default() += 1;
  }
  let keys: Vec<String> = candidates.iter().map(|c| key(&c.title)).collect();
  let mut last: HashMap<&str, usize> = HashMap::new();
  for (c, key) in candidates.iter().zip(&keys) {
    last.insert(key, c.page.0);
  }
  // Per page: candidates, and those reappearing on later pages.
  let mut ahead: HashMap<usize, (usize, usize)> = HashMap::new();
  for (c, key) in candidates.iter().zip(&keys) {
    let entry = ahead.entry(c.page.0).or_default();
    entry.0 += 1;
    if last[key.as_str()] > c.page.0 {
      entry.1 += 1;
    }
  }
  let contents = |page: usize| {
    let (all, later) = ahead[&page];
    later >= 5 && later * 5 >= all * 4
  };
  // Callout labels repeat throughout: "Note", "See Also".
  let mut uses: HashMap<&str, usize> = HashMap::new();
  for (c, key) in candidates.iter().zip(&keys) {
    if c.level.is_none() {
      *uses.entry(key).or_default() += 1;
    }
  }
  let callout =
    |key: &str| uses.get(key).is_some_and(|n| *n >= 10 && *n * 20 > pages);
  // The last kept title, extended by its repeats on following pages.
  let mut previous: Option<(&str, usize)> = None;
  let mut kept = Vec::new();
  for (c, key) in candidates.into_iter().zip(&keys) {
    if previous.is_some_and(|(k, page)| k == key && c.page.0 <= page + 1) {
      previous = Some((key, c.page.0));
      continue;
    }
    let unnumbered = c.level.is_none();
    if last[key.as_str()] > c.page.0 && contents(c.page.0)
      || unnumbered && callout(key)
      || unnumbered && crowds[&crowd(&c)] > MAX_CROWD
    {
      continue;
    }
    previous = Some((key, c.page.0));
    kept.push(c);
  }
  kept
}

/// Pages read when inferring; later pages rarely start new parts.
const PAGES: usize = 3000;

/// Outline loading off the input thread; dropping it ends the scan, as a
/// reload does before starting another. The flag marks an inferred one.
pub type OutlineJob = OneShot<(Vec<Heading>, bool)>;

/// Reads the authored outline on a separate document, or infers one when
/// there is none.
pub fn spawn(path: &Path, password: &str, wake: &Wake) -> OutlineJob {
  spawn_at(path, password, wake, content_cache::root())
}

fn spawn_at(
  path: &Path,
  password: &str,
  wake: &Wake,
  root: Option<PathBuf>,
) -> OutlineJob {
  let (path, password) = (path.to_owned(), password.to_owned());
  OneShot::spawn(wake, move |stopped| {
    let cache = root.and_then(|root| {
      let code = env!("VPDF_OUTLINE_CODE");
      Record::open(root, &path, &password, Kind::Outline, code, || {
        crate::scheduling::ready(stopped)
      })
    });
    let found = cache.as_ref().and_then(cached);
    if stopped.load(Ordering::Relaxed) {
      return None;
    }
    if let Some(found) = found {
      return Some(found);
    }
    let Ok(doc) = Handle::open(&path, &password) else {
      return Some((Vec::new(), false));
    };
    let pages = (0..doc.pages.min(PAGES)).map(|page| {
      crate::scheduling::ready(stopped).then(|| doc.styled_text(PageId(page)))
    });
    load_outline(doc.outline(), pages, cache.as_ref())
  })
}

/// A stored outline and whether it was inferred.
fn cached(cache: &Record) -> Option<(Vec<Heading>, bool)> {
  let (outline, inferred): (Vec<Heading>, bool) = cache.load()?;
  let valid = outline.len() <= 10_000 && outline.iter().all(|h| h.depth <= 64);
  valid.then_some((outline, inferred))
}

/// One page's text with its line styles.
type Styled = (Text, Vec<LineStyle>);

fn load_outline(
  authored: crate::Result<Vec<Heading>>,
  pages: impl IntoIterator<Item = Option<crate::Result<Styled>>>,
  cache: Option<&Record>,
) -> Option<(Vec<Heading>, bool)> {
  let mut complete = authored.is_ok();
  if let Ok(outline) = authored
    && !outline.is_empty()
  {
    if let Some(cache) = cache {
      cache.store(&(&outline, false));
    }
    return Some((outline, false));
  }
  let mut summaries = Vec::new();
  for text in pages {
    let text = text?;
    complete &= text.is_ok();
    summaries.push(
      text.map_or_else(
        |_| Vec::new(),
        |(text, styles)| summarize(&text, &styles),
      ),
    );
  }
  let outline = infer(&summaries);
  if complete && let Some(cache) = cache {
    cache.store(&(&outline, true));
  }
  Some((outline, true))
}

/// Numbered headings nest by their dots; unnumbered ones by size rank,
/// where sizes within a few percent share a rank. Nesting then closes
/// gaps, so a heading sits at most one level below its parent.
fn depths(candidates: Vec<Candidate>) -> Vec<Heading> {
  let mut sizes: Vec<(f32, bool)> = candidates
    .iter()
    .filter(|c| c.level.is_none())
    .map(|c| (c.size, c.bold))
    .collect();
  sizes.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
  // Bold and plain sizes cluster separately; ranks stay largest first.
  let mut ranks: Vec<(f32, bool)> = Vec::new();
  for (size, bold) in sizes {
    if !ranks.iter().any(|(top, b)| *b == bold && size >= top * RANK_SPREAD) {
      ranks.push((size, bold));
    }
  }
  let mut parents: Vec<usize> = Vec::new();
  candidates
    .into_iter()
    .map(|candidate| {
      let rank = candidate.level.map_or_else(
        || {
          ranks
            .iter()
            .position(|(size, bold)| {
              *bold == candidate.bold
                && candidate.size <= *size
                && candidate.size >= size * RANK_SPREAD
            })
            .unwrap_or(0)
        },
        |level| level - 1,
      );
      while parents.last().is_some_and(|parent| *parent >= rank) {
        parents.pop();
      }
      let depth = parents.len();
      parents.push(rank);
      Heading {
        title: candidate.title,
        depth: depth.min(6),
        target: Target::Point { page: candidate.page, x: 0.0, y: candidate.y },
      }
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::sync::{Arc, mpsc};

  fn record(root: &Path, source: &Path) -> Record {
    let code = env!("VPDF_OUTLINE_CODE");
    Record::open(root.into(), source, "", Kind::Outline, code, || true).unwrap()
  }

  #[test]
  fn stored_outlines_round_trip() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"PDF bytes")?;
    let cache = record(root.path(), &source);
    let headings: Vec<_> = [
      Target::Point { page: PageId(7), x: 2.125, y: -3.5 },
      Target::Page(PageId(3)),
      Target::Uri("chapter.pdf".into()),
    ]
    .into_iter()
    .enumerate()
    .map(|(depth, target)| Heading {
      title: format!("Chapter 章 {depth}"),
      depth,
      target,
    })
    .collect();
    cache.store(&(&headings, false));
    let (found, inferred) = cached(&cache).unwrap();
    assert!(!inferred);
    assert_eq!(found.len(), headings.len());
    for (actual, expected) in found.iter().zip(&headings) {
      assert_eq!(actual.title, expected.title);
      assert_eq!(actual.target, expected.target);
      assert_eq!(actual.depth, expected.depth);
    }
    cache.store(&(Vec::<Heading>::new(), true));
    assert_eq!(cached(&cache).map(|(_, inferred)| inferred), Some(true));
    let deep = Heading { depth: 65, ..headings[0].clone() };
    cache.store(&(vec![deep], true));
    assert!(cached(&cache).is_none());
    Ok(())
  }

  #[test]
  fn results_wake_the_loop() -> crate::Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["1 Introduction", "body"], "")?;
    let (wake, woken) = crate::fixtures::wake();
    let job = spawn(file.path(), "", &wake);
    woken.recv_timeout(std::time::Duration::from_secs(10))?;
    let (_, inferred) = job.result.try_recv()?;
    assert!(inferred);
    assert_eq!(woken.try_iter().count(), 0);
    Ok(())
  }

  #[test]
  fn failed_extraction_retries() -> crate::Result<()> {
    let source = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(source.path(), &["body"], "")?;
    let doc = Handle::open(source.path(), "")?;
    for authored_failure in [true, false] {
      let root = tempfile::tempdir()?;
      let cache = record(root.path(), source.path());
      let error = || anyhow::anyhow!("Extraction failed");
      let authored =
        if authored_failure { Err(error()) } else { Ok(Vec::new()) };
      let text = if authored_failure {
        doc.styled_text(PageId(0))
      } else {
        Err(error())
      };
      assert!(load_outline(authored, [Some(text)], Some(&cache)).is_some());
      assert!(cached(&cache).is_none());
      assert!(!cache.path().exists());
      let retried = load_outline(
        Ok(Vec::new()),
        [Some(doc.styled_text(PageId(0)))],
        Some(&cache),
      );
      assert!(retried.is_some());
      assert!(cached(&cache).is_some());
    }
    Ok(())
  }

  #[test]
  fn encrypted_job_stays_private() -> crate::Result<()> {
    let source = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(source.path(), &["Secret title"], "")?;
    let mut doc = Handle::open(source.path(), "")?;
    let file = crate::encryption::encrypt(&mut doc.native)?;
    let before = content_cache::alias(&mut std::fs::File::open(file.path())?)?;
    let root = tempfile::tempdir()?;
    let (wake, _) = crate::fixtures::wake();
    let job =
      spawn_at(file.path(), "test-reader", &wake, Some(root.path().to_owned()));
    let (_, inferred) =
      job.result.recv_timeout(std::time::Duration::from_secs(10))?;
    assert!(inferred);
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    assert_eq!(
      content_cache::alias(&mut std::fs::File::open(file.path())?)?,
      before
    );
    Ok(())
  }

  #[test]
  fn dropping_cancels_cache_lookup() -> crate::Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["body"], "")?;
    let pdf = std::fs::read_to_string(file.path())?
      .replace("/Type/Catalog", "/Type/Catalog/Outlines 6 0 R")
      .replace(
        "trailer",
        "6 0 obj <</Type/Outlines/First 7 0 R/Last 7 0 R/Count 1>> endobj\n\
         7 0 obj <</Title (Chapter)/Parent 6 0 R/Dest [4 0 R /Fit]>> endobj\n\
         trailer",
      );
    std::fs::write(file.path(), pdf)?;
    assert_eq!(Handle::open(file.path(), "")?.outline()?.len(), 1);
    let root = tempfile::tempdir()?;
    let drawing = crate::scheduling::Urgent::new();
    let (wake, woken) = crate::fixtures::wake();
    let mut job = spawn_at(file.path(), "", &wake, Some(root.path().into()));
    let (_, empty) = mpsc::channel();
    let result = std::mem::replace(&mut job.result, empty);
    drop(job);
    assert!(matches!(
      result.recv_timeout(std::time::Duration::from_secs(10)),
      Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    assert_eq!(woken.try_iter().count(), 0);
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    drop(drawing);
    Ok(())
  }

  #[test]
  fn dropping_stops_the_scan() -> crate::Result<()> {
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["body"; 30], "")?;
    let drawing = crate::scheduling::Urgent::new();
    let (wake, woken) = crate::fixtures::wake();
    let mut job = spawn(file.path(), "", &wake);
    let stopped = Arc::clone(&job.stop);
    let (_, empty) = mpsc::channel();
    let result = std::mem::replace(&mut job.result, empty);
    drop(job);
    assert!(stopped.load(Ordering::Relaxed));
    assert!(matches!(
      result.recv_timeout(std::time::Duration::from_secs(10)),
      Err(mpsc::RecvTimeoutError::Disconnected)
    ));
    assert_eq!(woken.try_iter().count(), 0);
    drop(drawing);
    Ok(())
  }

  #[test]
  fn numbering_levels() {
    let cases = [
      ("1 Introduction", Some(1)),
      ("3.2.1 Scaled Dot-Product Attention", Some(3)),
      ("A.1 Proofs", Some(2)),
      ("A. Proofs", Some(1)),
      ("A Bayesian approach", None),
      ("IV. Results", Some(1)),
      ("2 cups of flour", None),
      ("1999 was a year", None),
      ("Introduction", None),
      ("J. C. Brewer, Compiler", None),
      ("100 Cycles", None),
      ("3.1 ∆NOx ratios", Some(2)),
    ];
    for (title, level) in cases {
      assert_eq!(numbering(title), level, "{title}");
    }
  }

  #[test]
  fn depths_close_gaps() {
    let candidate =
      |title: &str, size: f32, level: Option<usize>, bold: bool| Candidate {
        page: PageId(0),
        y: 0.0,
        size,
        title: title.into(),
        level,
        bold,
      };
    let plain = |title, size, level| candidate(title, size, level, false);
    let cases = [
      (
        vec![
          plain("Book", 30.0, None),
          plain("Project", 12.0, None),
          plain("Preface", 30.0, None),
          plain("Features", 11.0, None),
          plain("1 Chapter", 14.0, Some(1)),
          plain("1.1.1 Deep", 11.0, Some(3)),
          plain("Aside", 12.0, None),
          plain("2 Chapter", 14.0, Some(1)),
        ],
        vec![0, 1, 0, 1, 0, 1, 1, 0],
      ),
      (
        vec![
          candidate("Bold Part", 12.0, None, true),
          plain("Plain Part", 12.0, None),
          candidate("Bold Again", 12.0, None, true),
        ],
        vec![0, 1, 0],
      ),
    ];
    for (candidates, expected) in cases {
      let found: Vec<_> = depths(candidates).iter().map(|h| h.depth).collect();
      assert_eq!(found, expected);
    }
  }

  #[test]
  fn plausible_titles() {
    let cases = [
      ("Methods", true),
      ("8.9.5. macaddr8", true),
      ("CPUID.06H -- Thermal and Power Management", true),
      ("Figure 3 Results by year", false),
      ("for (i=0; i<n; i++) {", false),
      ("--metrics name", false),
      (".peak_sustained", false),
      ("launch__grid_size", false),
      ("The __str__ method", true),
      ("Preface vii", false),
      ("Law and civil", true),
      ("Audio mix", true),
      ("DIV", true),
      ("Chapter 12", false),
      ("Methods 12", false),
      ("XIV", false),
      ("iv", false),
      ("Note ic", true),
      ("xii", false),
      ("A B", false),
      ("Aidan N. Gomez∗†", false),
      ("5.3 Problems with one constraint. Express the dual", false),
      ("10 Another gambling system is the martingale. Sup-", false),
    ];
    for (title, expected) in cases {
      assert_eq!(plausible(title, 100), expected, "{title}");
    }
  }

  #[test]
  fn summaries_join_lines() {
    use crate::text::{Bounds, Glyph};
    let line = |text: &str, x: f32, y: f32, paragraph: bool| Line {
      paragraph,
      glyphs: text
        .chars()
        .enumerate()
        .map(|(at, ch)| {
          let x0 = x + at as f32 * 5.0;
          let bounds = Bounds { x0, y0: y, x1: x0 + 5.0, y1: y + 10.0 };
          Glyph { ch, bounds, quad: None }
        })
        .collect(),
      ocr: None,
    };
    let style =
      |size: f32, bold: bool| LineStyle { size, bold, ..LineStyle::default() };
    let long = "Word ".repeat(25);
    let cases = [
      (
        "wrapped heading",
        vec![
          line("Wrapped heading", 0.0, 0.0, true),
          line("goes on", 0.0, 12.0, false),
        ],
        vec![style(14.0, true); 2],
        vec![("Wrapped heading goes on", 14.0, false, 2, 75.0)],
      ),
      (
        "bold number beside regular text",
        vec![line("4", 0.0, 0.0, true), line("Prove that", 20.0, 0.0, true)],
        vec![style(10.0, true), style(10.0, false)],
        vec![("4 Prove that", 10.0, true, 1, 50.0)],
      ),
      (
        "glyph heights without font sizes",
        vec![line("Results", 0.0, 0.0, true)],
        vec![style(0.0, false)],
        vec![("Results", 10.0, false, 1, 35.0)],
      ),
      (
        "join stops past the title limit",
        vec![line(long.trim(), 0.0, 0.0, true), line("Tail", 0.0, 12.0, false)],
        vec![style(10.0, false); 2],
        vec![(long.trim(), 10.0, false, 2, long.trim().len() as f32 * 5.0)],
      ),
    ];
    for (name, lines, styles, expected) in cases {
      let text = Text { lines, truncated: false };
      let found: Vec<_> = summarize(&text, &styles)
        .into_iter()
        .map(|s| (s.title, s.style.size, s.style.mixed, s.lines, s.width))
        .collect();
      let expected: Vec<_> = expected
        .into_iter()
        .map(|(title, size, mixed, lines, width)| {
          (title.to_owned(), size, mixed, lines, width)
        })
        .collect();
      assert_eq!(found, expected, "{name}");
    }
  }

  type Pages = Vec<Vec<Summary>>;

  const BODY: &str = "Body text runs across the whole column of the page here.";

  /// Lines as (title, size, bold); edges follow their position.
  fn page(lines: &[(&str, f32, bool)]) -> Vec<Summary> {
    let count = lines.len();
    lines
      .iter()
      .enumerate()
      .map(|(index, (title, size, bold))| Summary {
        title: (*title).into(),
        style: LineStyle { size: *size, bold: *bold, ..LineStyle::default() },
        y: index as f32,
        width: title.chars().count() as f32 * 5.0,
        paragraph: true,
        glyphs: title.chars().count(),
        lines: 1,
        edge: index < 2 || index + 2 >= count,
      })
      .collect()
  }

  fn body(size: f32) -> [(&'static str, f32, bool); 4] {
    [(BODY, size, false); 4]
  }

  fn titled(head: &[(&str, f32, bool)], size: f32) -> Vec<Summary> {
    let mut lines = head.to_vec();
    lines.extend(body(size));
    page(&lines)
  }

  #[test]
  fn inference_skips_layout_noise() {
    let fires = vec![
      titled(&[("F i r e s", 8.0, false), ("Fires", 20.0, false)], 10.0),
      titled(&[("F i r e s", 8.0, false)], 10.0),
      titled(&[("F i r e s", 8.0, false)], 10.0),
    ];
    let heads =
      (0..4).map(|_| titled(&[("Running Head", 12.0, false)], 10.0)).collect();
    let mut appended: Vec<_> =
      (0..8).map(|_| titled(&[], 10.0)).collect::<Vec<_>>();
    appended[1] = titled(&[("Early Part", 12.0, false)], 10.0);
    appended.extend((0..10).map(|_| titled(&[], 12.0)));
    appended[12] = titled(&[("Later Part", 14.0, false)], 12.0);
    let mut exercises = vec![titled(&[("1 Introduction", 14.0, false)], 10.0)];
    exercises.extend(["Show", "Prove", "Find"].map(|verb| {
      titled(
        &[
          (format!("1 {verb} that").as_str(), 10.0, false),
          (format!("2 {verb} this").as_str(), 10.0, false),
        ],
        10.0,
      )
    }));
    exercises.push(titled(&[("3 Methods", 10.0, false)], 10.0));
    let names = ["Alpha", "Beta", "Gamma", "Delta", "Epsilon"];
    let entries: Vec<_> =
      (0..5).map(|i| format!("{} {}", i + 1, names[i])).collect();
    let mut contents = vec![page(
      &entries.iter().map(|e| (e.as_str(), 10.0, false)).collect::<Vec<_>>(),
    )];
    contents.extend(
      entries.iter().map(|e| titled(&[(e.as_str(), 14.0, false)], 10.0)),
    );
    let crowd: Vec<_> = std::iter::once(("Overview", 18.0, false))
      .chain((0..8).flat_map(|_| [("Label", 14.0, false), (BODY, 10.0, false)]))
      .collect();
    let bold = vec![titled(
      &[
        ("Random Numbers", 10.0, true),
        (BODY, 10.0, false),
        ("Plain Words", 10.0, false),
      ],
      10.0,
    )];
    let prose = vec![titled(
      &[("4 This numbered line runs on across the column width", 10.0, false)],
      10.0,
    )];
    let cases: [(&str, Pages, &[&str]); 8] = [
      ("larger twin of a running head", fires, &["Fires"]),
      ("running head", heads, &[]),
      ("appended body size", appended, &["Early Part", "Later Part"]),
      ("exercises", exercises, &["1 Introduction", "3 Methods"]),
      (
        "contents",
        contents,
        &["1 Alpha", "2 Beta", "3 Gamma", "4 Delta", "5 Epsilon"],
      ),
      ("crowded labels", vec![page(&crowd)], &["Overview"]),
      ("bold heading", bold, &["Random Numbers"]),
      ("full-width prose", prose, &[]),
    ];
    assert_titles(cases);
  }

  #[test]
  fn inference_skips_labels() {
    let mut proceedings = Vec::new();
    for _ in 0..3 {
      proceedings.push(titled(&[("1 Introduction", 10.0, false)], 10.0));
      proceedings.extend((0..3).map(|_| titled(&[], 10.0)));
    }
    let callouts = (0..20)
      .map(|i| match i % 2 {
        0 => titled(&[("Note", 14.0, false)], 10.0),
        _ => titled(&[], 10.0),
      })
      .collect();
    let mut long = titled(&[("Long Lead", 14.0, false)], 10.0);
    long[0].lines = MAX_LINES + 1;
    let ticks = vec![titled(
      &[
        (BODY, 10.0, false),
        ("Chart Alpha", 14.0, false),
        ("0.5 1.0 1.5", 8.0, false),
        ("Year Axis", 8.0, false),
        ("2020 2021", 8.0, false),
      ],
      10.0,
    )];
    let folios = ["xiii", "xiv", "xv"]
      .map(|folio| {
        titled(&[(format!("CONTENTS {folio}").as_str(), 12.0, false)], 10.0)
      })
      .into();
    let cases: [(&str, Pages, &[&str]); 6] = [
      (
        "restarted numbering",
        proceedings,
        &["1 Introduction", "1 Introduction", "1 Introduction"],
      ),
      ("callouts", callouts, &[]),
      ("too many lines", vec![long], &[]),
      ("figure label", ticks, &[]),
      ("running head with folios", folios, &[]),
      (
        "one page",
        vec![titled(&[("Abstract", 12.0, false)], 10.0)],
        &["Abstract"],
      ),
    ];
    assert_titles(cases);
  }

  fn assert_titles<const N: usize>(cases: [(&str, Pages, &[&str]); N]) {
    for (name, pages, expected) in cases {
      let titles: Vec<_> = infer(&pages).into_iter().map(|h| h.title).collect();
      assert_eq!(titles, expected, "{name}");
    }
  }
}
