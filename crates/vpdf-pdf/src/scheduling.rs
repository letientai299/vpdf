use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

/// On-screen work in flight across documents: building a view's lists
/// and drawing its bands.
static URGENT: AtomicUsize = AtomicUsize::new(0);

/// Counts as on-screen work while alive.
pub struct Urgent(());

impl Urgent {
  pub fn new() -> Self {
    URGENT.fetch_add(1, Ordering::Relaxed);
    Self(())
  }
}

impl Drop for Urgent {
  fn drop(&mut self) {
    URGENT.fetch_sub(1, Ordering::Relaxed);
  }
}

/// Waits while any view's tiles render, as text scans share `MuPDF`'s
/// global allocation and glyph locks with the raster pool; false once
/// `stop` holds.
pub fn yield_to_views(stop: impl Fn() -> bool) -> bool {
  while URGENT.load(Ordering::Relaxed) > 0 {
    if stop() {
      return false;
    }
    thread::sleep(Duration::from_millis(1));
  }
  !stop()
}

impl Default for Urgent {
  fn default() -> Self {
    Self::new()
  }
}
