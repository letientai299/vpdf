use super::Wake;
use crate::Result;
use crate::crop::Margins;
use crate::document::{Handle, Lists};
use crate::text::PageId;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

/// Margin detection on a separate document; dropping it stops the scan.
pub struct Job {
  pub result: mpsc::Receiver<std::result::Result<Arc<Margins>, String>>,
  stop: Arc<AtomicBool>,
  pub pages: Vec<PageId>,
}

impl Job {
  /// Measures `pages`, extending `cached`; `nearby` pages go first.
  pub fn start(
    path: &Path,
    password: &str,
    cached: Option<Arc<Margins>>,
    lists: Arc<Lists>,
    pages: Vec<PageId>,
    nearby: Vec<PageId>,
    wake: &Wake,
  ) -> Self {
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
  pub fn cancel(&self) {
    self.stop.store(true, Ordering::Relaxed);
  }

  pub fn canceled(&self) -> bool {
    self.stop.load(Ordering::Relaxed)
  }
}

impl Drop for Job {
  fn drop(&mut self) {
    self.cancel();
  }
}
