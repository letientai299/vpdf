use crate::Result;
use crate::app::App;
use std::sync::Arc;
use vpdf_pdf::jobs::ocr::OcrJob;

impl App {
  pub fn start_ocr(&mut self, language: &str) -> Result<()> {
    if let Some(job) = self.ocr.take() {
      job.cancel();
    }
    self.ocr = Some(OcrJob::start(
      &self.path,
      &self.password,
      self.navigation_position().page,
      language,
      &self.wake,
    ));
    self.reader.notice("OCR running; :cancel stops");
    Ok(())
  }

  pub fn ocr_updates(&mut self) -> Result<bool> {
    let Some(job) = &self.ocr else { return Ok(false) };
    let Ok(result) = job.result.try_recv() else { return Ok(false) };
    let page = job.page;
    self.ocr = None;
    match result {
      Ok(text) => {
        if let Some(worker) = &self.search_worker {
          worker.seed(page, Arc::clone(&text));
        }
        self.doc.seed(page, text);
        self.reader.notice("OCR ready");
        self.reading_view().reclamp_text()?;
      }
      Err(error) => self.error = Some(error),
    }
    Ok(true)
  }
}
