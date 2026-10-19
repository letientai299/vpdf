use crate::Result;
use crate::app::App;
use std::sync::Arc;
use vpdf_pdf::jobs::crop::{Request, Update};

impl App {
  pub(crate) fn start_crop(&mut self) {
    self.crop_scan.start(
      Request {
        path: &self.path,
        password: &self.password,
        doc: &self.doc,
        wake: &self.wake,
      },
      self.nearby_pages(),
    );
    self.layout.request_crop();
    self.reader.notice("Detecting margins");
  }

  fn apply_crop(
    &mut self,
    crops: Arc<vpdf_model::geometry::Crops>,
  ) -> Result<()> {
    let animation = self.reading_view().suspend_scroll();
    self.layout.resolve_crops(crops);
    self.relayout()?;
    if let Some(point) = animation {
      self.reading_view().resume_scroll(point);
    }
    self.reader.notice("Margins cropped");
    Ok(())
  }

  pub(crate) fn request_crop(&mut self) {
    if self.layout.crop().is_none() {
      return;
    }
    let initial = self.crop_scan.request(
      Request {
        path: &self.path,
        password: &self.password,
        doc: &self.doc,
        wake: &self.wake,
      },
      self.nearby_pages(),
    );
    if initial {
      self.layout.request_crop();
      self.reader.notice("Detecting margins");
    }
  }

  pub fn crop_updates(&mut self) -> Result<bool> {
    self.request_crop();
    let Some(update) = self.crop_scan.poll() else { return Ok(false) };
    let result = match update {
      Update::Canceled => {
        self.request_crop();
        return Ok(true);
      }
      Update::Ready(result) => result,
    };
    let crops = match result {
      Ok(crops) => crops,
      Err(error) => {
        self.layout.show_margins();
        self.relayout()?;
        return Err(anyhow::Error::msg(error));
      }
    };
    self.doc.margins = Some(Arc::clone(&crops));
    self.apply_crop(crops.resolved())?;
    Ok(true)
  }

  pub(crate) fn cancel_crop(&mut self) -> Result<()> {
    if self.crop_scan.clear() {
      self.layout.show_margins();
      self.relayout()?;
    }
    Ok(())
  }

  /// `zc`: trims white margins, or restores them.
  pub fn toggle_crop(&mut self) -> Result<()> {
    if self.crop_scan.clear() || !self.layout.crop().is_none() {
      self.layout.show_margins();
      self.relayout()?;
      self.reader.notice("Margins shown");
    } else if let Some(crops) = &self.doc.margins {
      self.apply_crop(crops.resolved())?;
    } else {
      self.start_crop();
    }
    Ok(())
  }
}
