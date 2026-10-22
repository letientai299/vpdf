use super::{OneShot, Wake};
use crate::content_cache::{self, FigureCache};
use crate::document::{Handle, extract_list};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use vpdf_model::cache::Cache;
use vpdf_model::document::Figure;
use vpdf_model::text::{PageId, Text};

pub type Entries = Vec<(PageId, Figure)>;

pub type Job = OneShot<Result<Entries, String>>;

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
    Self::spawn(wake, move |stopped| {
      let found = scan(&path, &password, pages, seeds, stopped, root);
      (!stopped.load(Ordering::Relaxed))
        .then(|| found.map_err(|error| error.to_string()))
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
    FigureCache::open(root, path, password, || {
      crate::scheduling::yield_to_views(|| stop.load(Ordering::Relaxed))
    })
  });
  if let Some(entries) = cache.as_ref().and_then(|cache| cache.read(pages)) {
    return Ok(entries);
  }
  let handle = Handle::open(path, password)?;
  anyhow::ensure!(handle.pages == pages, "PDF changed; reopen figures");
  let seeds: HashMap<_, _> = seeds.into_iter().collect();
  let lists = Mutex::new(Cache::new(0));
  let mut entries = Vec::new();
  for index in 0..handle.pages {
    anyhow::ensure!(
      crate::scheduling::yield_to_views(|| stop.load(Ordering::Relaxed)),
      "Figure scan stopped"
    );
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
  {
    cache.save(&entries, pages);
  }
  Ok(entries)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;

  #[test]
  fn cached_figures_skip_native() -> crate::Result<()> {
    let root = tempfile::tempdir()?;
    let source = root.path().join("source.pdf");
    std::fs::write(&source, b"not a native PDF")?;
    let cache =
      FigureCache::open(root.path().into(), &source, "", || true).unwrap();
    cache.save(&Vec::new(), 1);
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
    let cache =
      FigureCache::open(root.path().into(), file.path(), "", || true).unwrap();
    assert_eq!(cache.read(1).unwrap().len(), 1);
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
