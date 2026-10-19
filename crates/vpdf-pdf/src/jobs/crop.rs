use super::Wake;
use crate::Result;
use crate::crop::Margins;
use crate::document::{Document, Handle};
use crate::text::PageId;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

const SAMPLES: usize = 24;

fn samples(count: usize, nearby: impl Iterator<Item = PageId>) -> Vec<PageId> {
  let batch = super::Config::default().prefetch_pages;
  let mut pages: Vec<_> = nearby.take(batch).collect();
  if count <= SAMPLES {
    pages.extend((0..count).map(PageId));
  } else {
    for pair in 0..(SAMPLES - batch) / 2 {
      let first = pair * count.saturating_sub(2) / ((SAMPLES - batch) / 2 - 1);
      pages.extend([PageId(first), PageId((first + 1).min(count - 1))]);
    }
  }
  let mut unique = Vec::new();
  for page in pages {
    if !unique.contains(&page) {
      unique.push(page);
    }
  }
  unique
}

#[derive(Clone, Copy)]
pub struct Request<'a> {
  pub path: &'a Path,
  pub password: &'a str,
  pub doc: &'a Document,
  pub wake: &'a Wake,
}

pub enum Update {
  Canceled,
  Ready(std::result::Result<Arc<Margins>, String>),
}

#[derive(Default)]
pub struct Scan {
  job: Option<Job>,
}

impl Scan {
  pub fn active(&self) -> bool {
    self.job.is_some()
  }

  pub fn clear(&mut self) -> bool {
    self.job.take().is_some()
  }

  pub fn start(&mut self, request: Request<'_>, nearby: Vec<PageId>) {
    let pages = samples(request.doc.pages.len(), nearby.iter().copied());
    self.job = Some(Job::start(request, pages, nearby));
  }

  /// Returns whether initial calibration started.
  pub fn request(&mut self, request: Request<'_>, nearby: Vec<PageId>) -> bool {
    let cached = request.doc.margins.as_deref();
    if let Some(job) = &self.job {
      if !job.needed(&nearby, cached) {
        job.cancel();
      }
      return false;
    }
    let Some(cached) = cached else {
      self.start(request, nearby);
      return true;
    };
    let pages: Vec<_> =
      nearby.iter().copied().filter(|page| !cached.contains(*page)).collect();
    if !pages.is_empty() {
      self.job = Some(Job::start(request, pages, nearby));
    }
    false
  }

  pub fn poll(&mut self) -> Option<Update> {
    let job = self.job.as_ref()?;
    let result = match job.result.try_recv() {
      Ok(result) => result,
      Err(mpsc::TryRecvError::Empty) => return None,
      Err(mpsc::TryRecvError::Disconnected) => Err("Cropping stopped".into()),
    };
    let canceled = job.canceled();
    self.clear();
    Some(if canceled { Update::Canceled } else { Update::Ready(result) })
  }
}

/// Margin detection on a separate document; dropping it stops the scan.
struct Job {
  result: mpsc::Receiver<std::result::Result<Arc<Margins>, String>>,
  stop: Arc<AtomicBool>,
  pages: Vec<PageId>,
}

impl Job {
  /// Measures `pages`, extending `cached`; `nearby` pages go first.
  fn start(
    request: Request<'_>,
    pages: Vec<PageId>,
    nearby: Vec<PageId>,
  ) -> Self {
    let Request { path, password, doc, wake } = request;
    let cached = doc.margins.clone();
    let lists = doc.lists();
    let (send, result) = mpsc::channel();
    let send = wake.sender(send);
    let stop = Arc::new(AtomicBool::new(false));
    let canceled = Arc::clone(&stop);
    let (path, password) = (path.to_owned(), password.to_owned());
    let wanted = pages.clone();
    std::thread::spawn(move || {
      let found = (|| -> Result<Arc<Margins>> {
        let handle = Handle::open(&path, &password)?;
        let mut margins = cached.as_deref().cloned().unwrap_or_default();
        margins.update(&handle, &pages, &nearby, &lists, &canceled)?;
        Ok(Arc::new(margins))
      })()
      .map_err(|error| error.to_string());
      let _ = send.send(found);
    });
    Self { result, stop, pages: wanted }
  }
}

impl Job {
  fn needed(&self, nearby: &[PageId], cached: Option<&Margins>) -> bool {
    let target_ready = nearby.first().is_some_and(|page| {
      self.pages.contains(page)
        || cached.is_some_and(|cached| cached.contains(*page))
    });
    target_ready && nearby.iter().any(|page| self.pages.contains(page))
  }

  fn cancel(&self) {
    self.stop.store(true, Ordering::Relaxed);
  }

  fn canceled(&self) -> bool {
    self.stop.load(Ordering::Relaxed)
  }
}

impl Drop for Job {
  fn drop(&mut self) {
    self.cancel();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  type Sender = mpsc::Sender<std::result::Result<Arc<Margins>, String>>;

  fn pending() -> (Scan, Sender, Arc<AtomicBool>) {
    let (send, result) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let job = Job {
      result,
      stop: Arc::clone(&stop),
      pages: vec![PageId(2), PageId(3)],
    };
    (Scan { job: Some(job) }, send, stop)
  }

  #[test]
  fn polling_preserves_ready_results() {
    let (mut scan, send, stop) = pending();
    assert!(scan.poll().is_none());
    assert!(scan.active());
    let margins = Arc::new(Margins::default());
    send.send(Ok(Arc::clone(&margins))).unwrap();
    let Some(Update::Ready(Ok(found))) = scan.poll() else {
      panic!("Missing margins")
    };
    assert!(Arc::ptr_eq(&found, &margins));
    assert!(!scan.active());
    assert!(stop.load(Ordering::Relaxed));
    assert!(scan.poll().is_none());
  }

  #[test]
  fn canceled_results_are_discarded() {
    for disconnected in [false, true] {
      let (mut scan, send, stop) = pending();
      stop.store(true, Ordering::Relaxed);
      if !disconnected {
        send.send(Err("Failed crop".into())).unwrap();
      }
      drop(send);
      assert!(matches!(scan.poll(), Some(Update::Canceled)));
      assert!(!scan.active());
    }
    let (mut scan, send, _) = pending();
    drop(send);
    let Some(Update::Ready(Err(message))) = scan.poll() else {
      panic!("Missing error")
    };
    assert_eq!(message, "Cropping stopped");
  }

  #[test]
  fn clearing_cancels_delivery() {
    let (mut scan, send, stop) = pending();
    assert!(scan.clear());
    assert!(!scan.clear());
    assert!(stop.load(Ordering::Relaxed));
    assert!(send.send(Err("Late result".into())).is_err());
  }

  #[test]
  fn active_scan_covers_target() {
    let (scan, _send, _) = pending();
    let job = scan.job.as_ref().unwrap();
    for (pages, wanted) in [
      (vec![2, 3], true),
      (vec![5, 2], false),
      (vec![2], true),
      (vec![5], false),
      (vec![], false),
    ] {
      let nearby: Vec<_> = pages.into_iter().map(PageId).collect();
      assert_eq!(job.needed(&nearby, None), wanted);
    }
  }

  #[test]
  fn sampling_is_bounded() {
    for count in [1, 2, 15, 17, 24, 244, 3329, 100_000] {
      let pages = samples(count, (0..count).map(PageId));
      assert!(pages.len() <= SAMPLES);
      if count <= SAMPLES {
        assert_eq!(pages.len(), count);
      }
      assert!(pages.iter().all(|page| page.0 < count));
      assert!(pages.contains(&PageId(0)));
      assert!(pages.contains(&PageId(count - 1)));
    }
    assert!(Margins::default().get(PageId(100)).is_none());
  }
}
