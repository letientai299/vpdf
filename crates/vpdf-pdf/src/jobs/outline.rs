//! Outline inferred from text layout for PDFs that ship none.
//!
//! Extracted glyph boxes carry no font weight, so headings are lines that
//! are either clearly larger than body text, or start with section
//! numbering (`3.2 Title`) at a paragraph start. Repeated lines at the
//! page edges (running heads, folios) are ignored.

use super::{OneShot, Wake};
use crate::content_cache::{self, OutlineCache};
use crate::document::{Handle, Heading, Target};
use crate::text::{Line, PageId, Text};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

/// Larger than body text by this factor counts as a heading size.
const LARGER: f32 = 1.15;
const MAX_CHARS: usize = 90;
const MAX_WORDS: usize = 14;

struct Candidate {
  page: PageId,
  y: f32,
  size: f32,
  title: String,
  /// Dots in leading numbering plus one; `None` without numbering.
  level: Option<usize>,
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

/// Section depth from `1`, `2.3`, `A.1`, or `IV.` numbering.
fn numbering(title: &str) -> Option<usize> {
  let (number, rest) = title.split_once(' ')?;
  let rest = rest.trim_start();
  if !rest.chars().next().is_some_and(char::is_uppercase) {
    return None;
  }
  // A lone undotted letter reads as a word: "A Bayesian approach".
  let word = number.len() == 1 && number.chars().all(char::is_alphabetic);
  if word {
    return None;
  }
  let number = number.trim_end_matches('.');
  let parts: Vec<_> = number.split('.').collect();
  let arabic = |part: &&str| part.chars().all(|c| c.is_ascii_digit());
  let valid = parts.iter().all(|p| !p.is_empty() && p.len() <= 3)
    && (parts.iter().all(arabic)
      || parts[0].len() == 1
        && parts[0].chars().all(|c| c.is_ascii_uppercase())
        && parts[1..].iter().all(arabic)
      || parts.len() == 1 && parts[0].chars().all(|c| "IVX".contains(c)));
  valid.then_some(parts.len())
}

fn plausible(title: &str) -> bool {
  let letters = title.chars().filter(|c| c.is_alphabetic()).count();
  title.chars().count() <= MAX_CHARS
    && title.split_whitespace().count() <= MAX_WORDS
    && letters * 2 >= title.chars().filter(|c| !c.is_whitespace()).count()
    && !title.ends_with(['.', ',', ';', ':'])
    // Contents pages list titles with trailing page numbers.
    && !title
      .rsplit_once(' ')
      .is_some_and(|(_, last)| last.chars().all(|c| c.is_ascii_digit()))
}

/// What inference needs from one extracted line.
pub struct Summary {
  title: String,
  size: f32,
  y: f32,
  paragraph: bool,
  glyphs: usize,
  /// Among the first or last two lines of its page.
  edge: bool,
}

/// Per-page line summaries, so whole-document inference need not keep
/// every page's glyphs; a bare section number joins the title beside it.
pub fn summarize(text: &Text) -> Vec<Summary> {
  let words =
    |line: &Line| line.text().split_whitespace().collect::<Vec<_>>().join(" ");
  let count = text.lines.len();
  let mut out = Vec::new();
  let mut lines = text.lines.iter().enumerate().peekable();
  while let Some((index, line)) = lines.next() {
    let Some(size) = size(line) else { continue };
    let mut title = words(line);
    if numbering(&format!("{title} X")).is_some()
      && let Some((_, next)) = lines.peek()
      && let (Some(a), Some(b)) = (line.glyphs.first(), next.glyphs.first())
      && (a.bounds.y1 - b.bounds.y1).abs() < a.bounds.height() / 2.0
    {
      title = format!("{title} {}", words(next));
      lines.next();
    }
    out.push(Summary {
      title,
      size,
      y: line.glyphs.first().map_or(0.0, |g| g.bounds.y0),
      paragraph: line.paragraph,
      glyphs: line.glyphs.len(),
      edge: index < 2 || index + 2 >= count,
    });
  }
  out
}

fn letters(title: &str) -> String {
  title.chars().filter(|c| c.is_alphabetic()).collect()
}

pub fn infer(pages: &[Vec<Summary>]) -> Vec<Heading> {
  let mut sizes: HashMap<u32, usize> = HashMap::new();
  let mut repeated: HashMap<String, usize> = HashMap::new();
  for line in pages.iter().flatten() {
    *sizes.entry((line.size * 2.0).round() as u32).or_default() += line.glyphs;
    if line.edge {
      *repeated.entry(letters(&line.title)).or_default() += 1;
    }
  }
  let Some(body) =
    sizes.iter().max_by_key(|(_, count)| **count).map(|(s, _)| *s as f32 / 2.0)
  else {
    return Vec::new();
  };
  // Running heads and folios repeat at page edges.
  let threshold = (pages.len() / 3).max(2);
  let mut candidates = Vec::new();
  for (index, page) in pages.iter().enumerate() {
    for line in page {
      let key = letters(&line.title);
      if key.is_empty()
        || !plausible(&line.title)
        || line.edge && repeated.get(&key).is_some_and(|n| *n >= threshold)
      {
        continue;
      }
      let level = numbering(&line.title)
        .filter(|_| line.paragraph && line.size >= body * 0.95);
      if line.size < body * LARGER && level.is_none() {
        continue;
      }
      candidates.push(Candidate {
        page: PageId(index),
        y: line.y,
        size: line.size,
        title: line.title.clone(),
        level,
      });
    }
  }
  depths(candidates, body)
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
      OutlineCache::open(root, &path, &password, || {
        crate::scheduling::yield_to_views(|| stopped.load(Ordering::Relaxed))
      })
    });
    let found = cache.as_ref().and_then(OutlineCache::read);
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
      crate::scheduling::yield_to_views(|| stopped.load(Ordering::Relaxed))
        .then(|| doc.text(PageId(page)))
    });
    load_outline(doc.outline(), pages, cache.as_ref())
  })
}

fn load_outline(
  authored: crate::Result<Vec<Heading>>,
  pages: impl IntoIterator<Item = Option<crate::Result<Text>>>,
  cache: Option<&OutlineCache>,
) -> Option<(Vec<Heading>, bool)> {
  let mut complete = authored.is_ok();
  if let Ok(outline) = authored
    && !outline.is_empty()
  {
    if let Some(cache) = cache {
      cache.save(&outline, false);
    }
    return Some((outline, false));
  }
  let mut summaries = Vec::new();
  for text in pages {
    let text = text?;
    complete &= text.is_ok();
    summaries.push(text.map_or_else(|_| Vec::new(), |text| summarize(&text)));
  }
  let outline = infer(&summaries);
  if complete && let Some(cache) = cache {
    cache.save(&outline, true);
  }
  Some((outline, true))
}

/// Numbered headings nest by their dots; unnumbered ones by size rank.
fn depths(candidates: Vec<Candidate>, body: f32) -> Vec<Heading> {
  let mut ranks: Vec<u32> = candidates
    .iter()
    .filter(|c| c.level.is_none() && c.size >= body * LARGER)
    .map(|c| (c.size * 2.0).round() as u32)
    .collect();
  ranks.sort_unstable_by(|a, b| b.cmp(a));
  ranks.dedup();
  candidates
    .into_iter()
    .map(|candidate| {
      let depth = candidate.level.map_or_else(
        || {
          let key = (candidate.size * 2.0).round() as u32;
          ranks.iter().position(|rank| *rank == key).unwrap_or(0)
        },
        |level| level - 1,
      );
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
      let cache =
        OutlineCache::open(root.path().into(), source.path(), "", || true)
          .unwrap();
      let error = || anyhow::anyhow!("Extraction failed");
      let authored =
        if authored_failure { Err(error()) } else { Ok(Vec::new()) };
      let text =
        if authored_failure { doc.text(PageId(0)) } else { Err(error()) };
      assert!(load_outline(authored, [Some(text)], Some(&cache)).is_some());
      assert!(cache.read().is_none());
      assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
      let retried =
        load_outline(Ok(Vec::new()), [Some(doc.text(PageId(0)))], Some(&cache));
      assert!(retried.is_some());
      assert!(cache.read().is_some());
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
    ];
    for (title, level) in cases {
      assert_eq!(numbering(title), level, "{title}");
    }
  }
}
