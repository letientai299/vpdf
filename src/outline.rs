//! Outline inferred from text layout for PDFs that ship none.
//!
//! Extracted glyph boxes carry no font weight, so headings are lines that
//! are either clearly larger than body text, or start with section
//! numbering (`3.2 Title`) at a paragraph start. Repeated lines at the
//! page edges (running heads, folios) are ignored.

use crate::app::App;
use crate::document::{Handle, Heading, Target};
use crate::keys::PanelKind;
use crate::text::{Line, PageId, Text};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

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
pub struct OutlineJob {
  result: mpsc::Receiver<(Vec<Heading>, bool)>,
  stop: Arc<AtomicBool>,
}

impl Drop for OutlineJob {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
  }
}

/// Reads the authored outline on a separate document, or infers one when
/// there is none.
pub fn spawn(path: &Path, password: &str) -> OutlineJob {
  let (send, result) = mpsc::channel();
  let stop = Arc::new(AtomicBool::new(false));
  let stopped = Arc::clone(&stop);
  let (path, password) = (path.to_owned(), password.to_owned());
  std::thread::spawn(move || {
    let Ok(doc) = Handle::open(&path, &password) else { return };
    if let Ok(outline) = doc.outline()
      && !outline.is_empty()
    {
      let _ = crate::input::notify(send.send((outline, false)));
      return;
    }
    let mut pages = Vec::new();
    for page in 0..doc.pages.min(PAGES) {
      if stopped.load(Ordering::Relaxed) {
        return;
      }
      // An unreadable page adds no headings instead of failing them all.
      let text = doc.text(PageId(page));
      pages.push(text.map_or_else(|_| Vec::new(), |text| summarize(&text)));
    }
    let _ = crate::input::notify(send.send((infer(&pages), true)));
  });
  OutlineJob { result, stop }
}

impl App {
  /// Starts loading the outline, which `Document::open` skips.
  pub fn load_outline(&mut self) {
    self.outline_job = Some(spawn(&self.path, &self.password));
  }

  pub fn outline_updates(&mut self) -> crate::Result<bool> {
    let Some(job) = &self.outline_job else { return Ok(false) };
    let Ok((outline, inferred)) = job.result.try_recv() else {
      return Ok(false);
    };
    self.outline_job = None;
    self.doc.outline = outline;
    self.doc.inferred = inferred;
    // Link labels name sections.
    self.hover_link = None;
    self.refill_panel(PanelKind::Outline)?;
    self.track_outline();
    Ok(true)
  }
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
