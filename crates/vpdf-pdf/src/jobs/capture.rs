use super::{OneShot, Outcome, Wake};
use crate::Result;
use crate::document::Document;
use crate::raster::Capture;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
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
  work: OneShot<Outcome<T>>,
}

impl<T: Send + 'static> Job<T> {
  /// Captures off the input thread, then runs `finish` there too, with
  /// the cancellation flag.
  pub fn start(
    request: Request,
    wake: &Wake,
    finish: impl FnOnce(Capture, &AtomicBool) -> Result<T> + Send + 'static,
  ) -> Self {
    let work = OneShot::try_spawn(wake, move |stop| {
      let Request { path, password, page, bounds, rotation } = request;
      let result = capture(&path, &password, page, bounds, rotation, stop);
      Some(result.and_then(|capture| finish(capture, stop)))
    });
    Self { work }
  }

  /// The finished result, if any.
  pub fn poll(&self) -> Option<Outcome<T>> {
    self.work.poll("Region worker disconnected")
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
  use std::sync::{Arc, mpsc};
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
    drop(Job { work: OneShot { result, stop: Arc::clone(&cancel) } });
    assert!(cancel.load(Ordering::Relaxed));
    Ok(())
  }
}
