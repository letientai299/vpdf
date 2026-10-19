use super::Wake;
use crate::Result;
use crate::document::Document;
use crate::raster::Capture;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use vpdf_model::text::{Bounds, PageId};

/// A page region to rasterize.
pub struct Request {
  pub path: PathBuf,
  pub password: String,
  pub page: PageId,
  pub bounds: Bounds,
  pub rotation: u16,
}

/// A region capture on its own document; dropping it cancels.
pub struct Job<T> {
  cancel: Arc<AtomicBool>,
  result: mpsc::Receiver<std::result::Result<T, String>>,
}

impl<T: Send + 'static> Job<T> {
  /// Captures off the input thread, then runs `finish` there too, with
  /// the cancellation flag.
  pub fn start(
    request: Request,
    wake: &Wake,
    finish: impl FnOnce(Capture, &AtomicBool) -> Result<T> + Send + 'static,
  ) -> Self {
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&cancel);
    let (send, result) = mpsc::channel();
    let send = wake.sender(send);
    std::thread::spawn(move || {
      let Request { path, password, page, bounds, rotation } = request;
      let result = capture(&path, &password, page, bounds, rotation, &stop)
        .and_then(|capture| finish(capture, &stop))
        .map_err(|error| error.to_string());
      let _ = send.send(result);
    });
    Self { cancel, result }
  }

  /// The finished result, if any.
  pub fn poll(&self) -> Option<std::result::Result<T, String>> {
    match self.result.try_recv() {
      Ok(result) => Some(result),
      Err(mpsc::TryRecvError::Empty) => None,
      Err(mpsc::TryRecvError::Disconnected) => {
        Some(Err("Region worker disconnected".into()))
      }
    }
  }
}

impl<T> Drop for Job<T> {
  fn drop(&mut self) {
    self.cancel.store(true, Ordering::Relaxed);
  }
}

fn capture(
  path: &Path,
  password: &str,
  page: PageId,
  bounds: Bounds,
  rotation: u16,
  cancel: &AtomicBool,
) -> Result<Capture> {
  let mut doc = Document::open(path, password)?;
  let capture = crate::raster::capture(&mut doc, page, bounds, rotation)?;
  anyhow::ensure!(
    !cancel.load(Ordering::Relaxed),
    "Region operation cancelled"
  );
  Ok(capture)
}

pub fn recognize(
  png: Vec<u8>,
  language: &str,
  cancel: &AtomicBool,
) -> Result<String> {
  let output = super::ocr::tesseract(png, language, cancel)?;
  let text = super::ocr::parse_tsv(&output, Bounds::default(), 1.0)?;
  Ok(
    text
      .lines
      .iter()
      .map(vpdf_model::text::Line::text)
      .collect::<Vec<_>>()
      .join("\n"),
  )
}

#[cfg(test)]
mod tests {
  use super::*;
  use anyhow::Context;
  use std::time::Duration;

  fn request(path: PathBuf) -> Request {
    Request {
      path,
      password: String::new(),
      page: PageId(0),
      bounds: Bounds { x0: 10.0, y0: 10.0, x1: 50.0, y1: 50.0 },
      rotation: 0,
    }
  }

  #[test]
  fn results_wake_the_loop() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("region.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let (wake, woken) = crate::fixtures::wake();
    let job = Job::start(request(path), &wake, |capture, _| Ok(capture.png));
    woken.recv_timeout(Duration::from_secs(10))?;
    let png = job.poll().context("Missing capture")?;
    assert!(png.map_err(anyhow::Error::msg)?.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(woken.try_iter().count(), 0);
    Ok(())
  }

  #[test]
  fn cancelled_captures_skip_finish() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("region.pdf");
    crate::fixtures::pdf(&path, &["body"], "")?;
    let request = request(path);
    let cancel = AtomicBool::new(true);
    let result = capture(
      &request.path,
      "",
      request.page,
      request.bounds,
      request.rotation,
      &cancel,
    );
    assert_eq!(result.err().unwrap().to_string(), "Region operation cancelled");
    let (_send, result) = mpsc::channel::<std::result::Result<(), String>>();
    let cancel = Arc::new(AtomicBool::new(false));
    drop(Job { cancel: Arc::clone(&cancel), result });
    assert!(cancel.load(Ordering::Relaxed));
    Ok(())
  }
}
