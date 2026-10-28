//! Match Metal presentation to native live resize.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, Message};
use objc2_app_kit::NSView;
use objc2_quartz_core::{CALayer, CAMetalLayer};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub(crate) fn view(parent: &impl HasWindowHandle) -> Option<Retained<NSView>> {
  let _main = MainThreadMarker::new()?;
  let borrowed = parent.window_handle().ok()?;
  let RawWindowHandle::AppKit(raw) = borrowed.as_raw() else { return None };
  // SAFETY: Borrowed AppKit handles guarantee live NSViews.
  Some(unsafe { raw.ns_view.cast::<NSView>().as_ref() }.retain())
}

pub struct Presentation {
  view: Retained<NSView>,
}

impl Presentation {
  pub fn new(parent: &impl HasWindowHandle) -> Option<Self> {
    Some(Self { view: view(parent)? })
  }

  /// Corrects eframe's Accessibility resize presentation.
  pub fn prepare_frame(&self) {
    let Some(window) = self.view.window() else { return };
    let Some(layer) = self.view.layer() else { return };
    // Eframe misclassifies Accessibility-driven window resizes.
    sync_layers(&layer, window.inLiveResize());
  }
}

fn sync_layers(layer: &CALayer, resizing: bool) {
  if let Some(metal) = layer.downcast_ref::<CAMetalLayer>()
    && metal.presentsWithTransaction() != resizing
  {
    metal.setPresentsWithTransaction(resizing);
  }
  // SAFETY: AppKit owns this valid CALayer tree.
  if let Some(children) = unsafe { layer.sublayers() } {
    for child in children {
      sync_layers(&child, resizing);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn resize_tracks_nested_layers() {
    let root = CALayer::new();
    let container = CALayer::new();
    let metal = CAMetalLayer::new();
    root.addSublayer(&container);
    container.addSublayer(&metal);
    sync_layers(&root, true);
    assert!(metal.presentsWithTransaction());
    sync_layers(&root, false);
    assert!(!metal.presentsWithTransaction());
  }

  #[test]
  fn resize_preserves_other_trees() {
    let target = CAMetalLayer::new();
    let other = CAMetalLayer::new();
    target.setPresentsWithTransaction(true);
    other.setPresentsWithTransaction(true);
    sync_layers(&target, false);
    assert!(!target.presentsWithTransaction());
    assert!(other.presentsWithTransaction());
  }
}
