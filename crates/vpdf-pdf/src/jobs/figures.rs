use super::Wake;
use crate::document::{Handle, extract_list};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use vpdf_model::cache::Cache;
use vpdf_model::document::Figure;
use vpdf_model::text::{PageId, Text};

pub type Entries = Vec<(PageId, Figure)>;

pub struct Job {
  pub result: mpsc::Receiver<Result<Entries, String>>,
  stop: Arc<AtomicBool>,
}

impl Job {
  pub fn start(
    path: &Path,
    password: &str,
    pages: usize,
    seeds: Vec<(PageId, Arc<Text>)>,
    wake: &Wake,
  ) -> Self {
    let (send, result) = mpsc::channel();
    let send = wake.sender(send);
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    let (path, password) = (path.to_owned(), password.to_owned());
    std::thread::spawn(move || {
      let found = scan(&path, &password, pages, seeds, &stopped);
      if !stopped.load(Ordering::Relaxed) {
        let _ = send.send(found.map_err(|error| error.to_string()));
      }
    });
    Self { result, stop }
  }
}

impl Drop for Job {
  fn drop(&mut self) {
    self.stop.store(true, Ordering::Relaxed);
  }
}

fn scan(
  path: &Path,
  password: &str,
  pages: usize,
  seeds: Vec<(PageId, Arc<Text>)>,
  stop: &AtomicBool,
) -> crate::Result<Entries> {
  let handle = Handle::open(path, password)?;
  anyhow::ensure!(handle.pages == pages, "PDF changed; reopen figures");
  let seeds: HashMap<_, _> = seeds.into_iter().collect();
  let lists = Mutex::new(Cache::new(0));
  let mut entries = Vec::new();
  for index in 0..handle.pages {
    if !crate::scheduling::yield_to_views(|| stop.load(Ordering::Relaxed)) {
      break;
    }
    let page = PageId(index);
    let list = handle.display(page, &lists)?;
    let text = seeds.get(&page).map_or_else(
      || extract_list(&list).map(Arc::new),
      |text| Ok(Arc::clone(text)),
    )?;
    let figures = crate::figures::detect(&list, &text, handle.bounds(page)?)?;
    entries.extend(figures.into_iter().map(|figure| (page, figure)));
  }
  Ok(entries)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::time::Duration;

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
