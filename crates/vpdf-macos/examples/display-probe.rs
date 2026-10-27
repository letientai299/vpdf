#[cfg(target_os = "macos")]
fn main() {
  use objc2_app_kit::{
    NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
  };
  use std::sync::Arc;
  use std::sync::atomic::{AtomicUsize, Ordering};
  let calls = Arc::new(AtomicUsize::new(0));
  let called = Arc::clone(&calls);
  let observer = vpdf_macos::display::Observer::new(move || {
    called.fetch_add(1, Ordering::Relaxed);
  });
  let initial = observer.take().expect("Initial display settings");
  assert!(observer.take().is_none());
  let center = NSWorkspace::sharedWorkspace().notificationCenter();
  let post = || {
    // The notification has no object or user info.
    unsafe {
      center.postNotificationName_object(
        NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
        None,
      )
    };
  };
  post();
  assert_eq!(calls.load(Ordering::Relaxed), 1);
  assert_eq!(observer.take(), Some(initial));
  assert!(observer.take().is_none());
  drop(observer);
  post();
  assert_eq!(calls.load(Ordering::Relaxed), 1);
  println!("Display notifications and teardown passed");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
