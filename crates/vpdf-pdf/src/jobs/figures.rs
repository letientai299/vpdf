use super::{OneShot, Outcome, Wake};
use crate::content_cache::{self, Kind, Record};
use crate::document::{Handle, extract_list};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vpdf_model::cache::Cache;
use vpdf_model::document::Figure;
use vpdf_model::text::{Bounds, PageId, Text};

pub type Entries = Vec<(PageId, Figure)>;

/// Most figures a stored record holds.
const LIMIT: usize = 100_000;

pub type Job = OneShot<Outcome<Entries>>;

impl Job {
  pub fn start(
    path: &Path,
    password: &str,
    pages: usize,
    seeds: Vec<(PageId, Arc<Text>)>,
    wake: &Wake,
  ) -> Self {
    Self::start_at(path, password, pages, seeds, wake, content_cache::root())
  }

  fn start_at(
    path: &Path,
    password: &str,
    pages: usize,
    seeds: Vec<(PageId, Arc<Text>)>,
    wake: &Wake,
    root: Option<PathBuf>,
  ) -> Self {
    let (path, password) = (path.to_owned(), password.to_owned());
    Self::try_spawn(wake, move |stopped| {
      let found = scan(&path, &password, pages, seeds, stopped, root);
      (!stopped.load(Ordering::Relaxed)).then_some(found)
    })
  }
}

fn scan(
  path: &Path,
  password: &str,
  pages: usize,
  seeds: Vec<(PageId, Arc<Text>)>,
  stop: &AtomicBool,
  root: Option<PathBuf>,
) -> crate::Result<Entries> {
  let cache = root.filter(|_| seeds.is_empty()).and_then(|root| {
    let code = env!("VPDF_FIGURE_CODE");
    Record::open(root, path, password, Kind::Figures, code, || {
      crate::scheduling::ready(stop)
    })
  });
  if let Some(entries) = cache.as_ref().and_then(|cache| cached(cache, pages)) {
    return Ok(entries);
  }
  let handle = Handle::open(path, password)?;
  anyhow::ensure!(handle.pages == pages, "PDF changed; reopen figures");
  let seeds: HashMap<_, _> = seeds.into_iter().collect();
  let lists = Mutex::new(Cache::new(0));
  let mut entries = Vec::new();
  for index in 0..handle.pages {
    anyhow::ensure!(crate::scheduling::ready(stop), "Figure scan stopped");
    let page = PageId(index);
    let list = handle.display(page, &lists)?;
    let text = seeds.get(&page).map_or_else(
      || extract_list(&list).map(Arc::new),
      |text| Ok(Arc::clone(text)),
    )?;
    let figures = crate::figures::detect(&list, &text, handle.bounds(page)?)?;
    entries.extend(figures.into_iter().map(|figure| (page, figure)));
  }
  if entries.iter().any(|(_, figure)| !figure.caption.is_empty()) {
    entries.retain(|(_, figure)| !figure.caption.is_empty());
  }
  if !stop.load(Ordering::Relaxed)
    && let Some(cache) = cache
    && entries.len() <= LIMIT
  {
    cache.store(&(pages, &entries));
  }
  Ok(entries)
}

/// Stored figures, when they fit a `pages`-page document.
fn cached(cache: &Record, pages: usize) -> Option<Entries> {
  let (count, entries): (usize, Entries) = cache.load()?;
  let valid = |(page, figure): &(PageId, Figure)| {
    let Bounds { x0, y0, x1, y1 } = figure.bounds;
    page.0 < pages
      && [x0, y0, x1, y1].iter().all(|n| n.is_finite())
      && x0 < x1
      && y0 < y1
  };
  (count == pages && entries.len() <= LIMIT && entries.iter().all(valid))
    .then_some(entries)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;
  use vpdf_model::document::FigureSource;

  fn record(root: &Path, source: &Path) -> Record {
    let code = env!("VPDF_FIGURE_CODE");
    Record::open(root.into(), source, "", Kind::Figures, code, || true).unwrap()
  }

  /// Figures read back whole; other page counts and bad bounds miss.
  #[test]
  fn stored_figures_validate() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"PDF bytes")?;
    let cache = record(root.path(), &source);
    cache.store(&(2, Entries::new()));
    assert!(cached(&cache, 2).unwrap().is_empty());
    let figure = |bounds: [f32; 4]| Figure {
      bounds: Bounds {
        x0: bounds[0],
        y0: bounds[1],
        x1: bounds[2],
        y1: bounds[3],
      },
      caption: "FIGURE 4.1 Architecture of a CUDA-capable GPU.".into(),
      source: FigureSource::Graphic,
    };
    let entries = vec![(PageId(1), figure([20.0, 30.0, 200.0, 150.0]))];
    cache.store(&(2, &entries));
    let found = cached(&cache, 2).unwrap();
    assert_eq!(found[0].0, PageId(1));
    assert_eq!(found[0].1.caption, entries[0].1.caption);
    assert_eq!(found[0].1.bounds, entries[0].1.bounds);
    assert_eq!(found[0].1.source, FigureSource::Graphic);
    assert!(cached(&cache, 3).is_none());
    for (page, area) in [
      (2, [20.0, 30.0, 200.0, 150.0]),
      (0, [200.0, 30.0, 20.0, 150.0]),
      (0, [20.0, 30.0, 200.0, 30.0]),
      (0, [f32::NAN, 30.0, 200.0, 150.0]),
    ] {
      cache.store(&(2, vec![(PageId(page), figure(area))]));
      assert!(cached(&cache, 2).is_none());
    }
    Ok(())
  }

  #[test]
  fn cached_figures_skip_native() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"not a native PDF")?;
    let cache = record(root.path(), &source);
    cache.store(&(1, Entries::new()));
    let job = Job::start_at(
      &source,
      "",
      1,
      Vec::new(),
      &Wake::none(),
      Some(root.path().into()),
    );
    let entries = job
      .result
      .recv_timeout(Duration::from_secs(5))?
      .map_err(anyhow::Error::msg)?;
    assert!(entries.is_empty());
    Ok(())
  }

  #[test]
  fn seeded_figures_bypass_cache() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["Figure 1: native"], "")?;
    let stop = AtomicBool::new(false);
    let entries =
      scan(file.path(), "", 1, Vec::new(), &stop, Some(root.path().into()))?;
    assert_eq!(entries.len(), 1);
    let seeds = vec![(PageId(0), Arc::new(Text::default()))];
    assert!(
      scan(file.path(), "", 1, seeds, &stop, Some(root.path().into()))?
        .is_empty()
    );
    let cache = record(root.path(), file.path());
    assert_eq!(cached(&cache, 1).unwrap().len(), 1);
    Ok(())
  }

  #[test]
  fn stopped_scan_stays_uncached() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let file = tempfile::NamedTempFile::new()?;
    crate::fixtures::pdf(file.path(), &["Figure 1: native"], "")?;
    let stop = AtomicBool::new(true);
    assert!(
      scan(file.path(), "", 1, Vec::new(), &stop, Some(root.path().into()))
        .is_err()
    );
    assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
    Ok(())
  }

  #[test]
  fn missing_document_reports_error() {
    let directory = tempfile::tempdir().unwrap();
    let job = Job::start(
      &directory.path().join("missing.pdf"),
      "",
      1,
      Vec::new(),
      &Wake::none(),
    );
    assert!(job.result.recv_timeout(Duration::from_secs(5)).unwrap().is_err());
  }
}
