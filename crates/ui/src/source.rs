use crate::text::{Bounds, PageId};

/// Metadata for a nonempty document.
pub trait Pages {
  fn count(&self) -> usize;
  fn bounds(&self, page: PageId) -> Bounds;
  fn label(&self, page: PageId) -> &str;
}

impl<S: Pages + ?Sized> Pages for &mut S {
  fn count(&self) -> usize {
    (**self).count()
  }
  fn bounds(&self, page: PageId) -> Bounds {
    (**self).bounds(page)
  }
  fn label(&self, page: PageId) -> &str {
    (**self).label(page)
  }
}

/// Figure lists in page reading order.
pub trait Figures: Pages {
  fn figures(
    &mut self,
    page: PageId,
  ) -> crate::Result<std::sync::Arc<Vec<vpdf_model::document::Figure>>>;
}
