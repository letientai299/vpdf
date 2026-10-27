#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Display {
  pub contrast: bool,
  pub motion: bool,
  pub transparency: bool,
  pub differentiate: bool,
}

#[cfg(target_os = "macos")]
mod macos {
  use super::Display;
  use objc2::rc::Retained;
  use objc2::runtime::ProtocolObject;
  use objc2_app_kit::{
    NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
  };
  use objc2_foundation::{NSNotificationCenter, NSObjectProtocol};
  use std::sync::Arc;
  use std::sync::atomic::{AtomicBool, Ordering};

  pub struct Observer {
    _main: objc2::MainThreadMarker,
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    dirty: Arc<AtomicBool>,
  }

  impl Observer {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
      // AppKit access stays on the interaction thread.
      let main = objc2::MainThreadMarker::new()
        .expect("Display observer requires main thread");
      let center = NSWorkspace::sharedWorkspace().notificationCenter();
      let dirty = Arc::new(AtomicBool::new(true));
      let changed = Arc::clone(&dirty);
      let block = block2::RcBlock::new(move |_| {
        changed.store(true, Ordering::Release);
        wake();
      });
      // SAFETY: Callbacks capture thread-safe state only.
      let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
          Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
          None,
          None,
          &block,
        )
      };
      Self { _main: main, center, token, dirty }
    }

    pub fn take(&self) -> Option<Display> {
      if !self.dirty.swap(false, Ordering::AcqRel) {
        return None;
      }
      let workspace = NSWorkspace::sharedWorkspace();
      Some(Display {
        contrast: workspace.accessibilityDisplayShouldIncreaseContrast(),
        motion: workspace.accessibilityDisplayShouldReduceMotion(),
        transparency: workspace.accessibilityDisplayShouldReduceTransparency(),
        differentiate: workspace
          .accessibilityDisplayShouldDifferentiateWithoutColor(),
      })
    }
  }

  impl Drop for Observer {
    fn drop(&mut self) {
      // SAFETY: Removal uses our retained token.
      unsafe { self.center.removeObserver((*self.token).as_ref()) };
    }
  }
}

#[cfg(target_os = "macos")]
pub use macos::Observer;
