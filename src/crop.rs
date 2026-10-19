use crate::Result;
use crate::app::App;
use crate::jobs::crop::Job;
use crate::layout::Crop;
use crate::text::{Bounds, PageId};
use std::sync::{Arc, mpsc};

pub use vpdf_pdf::crop::{Margins, margins};

const SAMPLES: usize = 24;
const BATCH: usize = crate::prefetch::PAGES;

fn samples(count: usize, nearby: impl Iterator<Item = PageId>) -> Vec<PageId> {
  let mut pages: Vec<_> = nearby.take(BATCH).collect();
  if count <= SAMPLES {
    pages.extend((0..count).map(PageId));
  } else {
    for pair in 0..(SAMPLES - BATCH) / 2 {
      let first = pair * count.saturating_sub(2) / ((SAMPLES - BATCH) / 2 - 1);
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

impl App {
  pub(crate) fn start_crop(&mut self) {
    let pages = samples(self.doc.pages.len(), self.nearby_pages().into_iter());
    self.crop_batch(pages);
    self.layout.crop = Crop([1, 0, 0, 0]);
    self.reader.message = "Detecting margins".into();
  }

  fn crop_batch(&mut self, pages: Vec<PageId>) {
    self.crop_job = Some(Job::start(
      &self.path,
      &self.password,
      self.doc.margins.clone(),
      self.doc.lists(),
      pages,
      self.nearby_pages(),
      &self.wake,
    ));
  }

  fn apply_crop(&mut self, crops: Arc<Margins>) -> Result<()> {
    let animation = self.animation.take().and_then(|animation| {
      let page = self.layout.page_at(animation.target);
      let point = self.layout.unproject(
        page,
        (self.layout.screen.width() / 2.0, 0.0),
        animation.target,
      )?;
      Some((animation, page, point))
    });
    self.layout.crop = Crop([1, 0, 0, 0]);
    self.layout.crops = crops;
    self.relayout()?;
    if let Some((mut animation, page, (x, y))) = animation {
      let bounds = Bounds { x0: x, y0: y, x1: x, y1: y };
      let rect = self.layout.project(page, bounds, 0.0);
      let shown = &self.layout.pages[page.0];
      animation.target = self
        .layout
        .clamp_scroll(rect.y0.clamp(shown.top, shown.top + shown.height));
      animation.updated = std::time::Instant::now();
      self.animation = Some(animation);
    }
    self.reader.message = "Margins cropped".into();
    Ok(())
  }

  pub(crate) fn request_crop(&mut self) {
    if self.layout.crop.is_none() {
      return;
    }
    let nearby = self.nearby_pages();
    if let Some(job) = &self.crop_job {
      let target_ready = nearby.first().is_some_and(|page| {
        job.pages.contains(page)
          || self
            .doc
            .margins
            .as_ref()
            .is_some_and(|cached| cached.contains(*page))
      });
      if target_ready && nearby.iter().any(|page| job.pages.contains(page)) {
        return;
      }
      job.cancel();
      return;
    }
    let Some(cached) = &self.doc.margins else {
      self.start_crop();
      return;
    };
    let pages: Vec<_> = self
      .nearby_pages()
      .into_iter()
      .filter(|page| !cached.contains(*page))
      .collect();
    if !pages.is_empty() {
      self.crop_batch(pages);
    }
  }

  pub fn crop_updates(&mut self) -> Result<bool> {
    self.request_crop();
    let Some(job) = &self.crop_job else { return Ok(false) };
    let result = match job.result.try_recv() {
      Ok(result) => result,
      Err(mpsc::TryRecvError::Empty) => return Ok(false),
      Err(mpsc::TryRecvError::Disconnected) => Err("Cropping stopped".into()),
    };
    let canceled = job.canceled();
    self.crop_job = None;
    if canceled {
      self.request_crop();
      return Ok(true);
    }
    let crops = match result {
      Ok(crops) => crops,
      Err(error) => {
        self.layout.crop = Crop::default();
        self.relayout()?;
        return Err(anyhow::Error::msg(error));
      }
    };
    self.doc.margins = Some(Arc::clone(&crops));
    self.apply_crop(crops)?;
    Ok(true)
  }

  pub(crate) fn cancel_crop(&mut self) -> Result<()> {
    if self.crop_job.take().is_some() {
      self.layout.crop = Crop::default();
      self.relayout()?;
    }
    Ok(())
  }

  /// `zc`: trims white margins, or restores them.
  pub fn toggle_crop(&mut self) -> Result<()> {
    if self.crop_job.take().is_some() || !self.layout.crop.is_none() {
      self.layout.crop = Crop::default();
      self.relayout()?;
      self.reader.message = "Margins shown".into();
    } else if let Some(crops) = &self.doc.margins {
      self.apply_crop(Arc::clone(crops))?;
    } else {
      self.start_crop();
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
