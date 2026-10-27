//! Main-thread menu and window-sheet adapter.
#![cfg(target_os = "macos")]
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
  NSApplication, NSMenu, NSMenuItem, NSModalResponseOK, NSOpenPanel, NSView,
  NSWindow,
};
use objc2_foundation::{
  MainThreadMarker, NSObject, NSObjectProtocol, ns_string,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

struct Callback {
  enabled: Cell<bool>,
  invoke: Box<dyn Fn()>,
}

define_class!(
  // SAFETY: NSObject permits main-thread subclasses.
  #[unsafe(super = NSObject)]
  #[thread_kind = MainThreadOnly]
  #[ivars = Callback]
  struct Handler;
  // SAFETY: NSObjectProtocol adds no subclass invariants.
  unsafe impl NSObjectProtocol for Handler {}
  impl Handler {
    // SAFETY: AppKit actions accept a nullable sender.
    #[unsafe(method(openReader:))]
    fn open(&self, _: Option<&NSObject>) {
      if self.ivars().enabled.get() {
        (self.ivars().invoke)();
      }
    }
  }
);

pub struct Integration {
  handler: Retained<Handler>,
  window: Retained<NSWindow>,
  menu: Retained<NSMenu>,
  root: Retained<NSMenuItem>,
  open: Retained<NSMenuItem>,
  panel: Option<Retained<NSOpenPanel>>,
  alive: Rc<Cell<bool>>,
  #[cfg(feature = "bench-events")]
  identity: proof::Identity,
}

impl Integration {
  pub fn install(
    parent: &impl HasWindowHandle,
    invoke: impl Fn() + 'static,
  ) -> Option<Self> {
    let mtm = MainThreadMarker::new()?;
    let borrowed = parent.window_handle().ok()?;
    let RawWindowHandle::AppKit(raw) = borrowed.as_raw() else { return None };
    // SAFETY: Borrowed AppKit handles guarantee live NSViews.
    let view = unsafe { raw.ns_view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    let menu = NSApplication::sharedApplication(mtm).mainMenu()?;
    #[cfg(feature = "bench-events")]
    let identity = proof::Identity::capture(&window, &menu, mtm);
    let allocated = Handler::alloc(mtm).set_ivars(Callback {
      enabled: Cell::new(false),
      invoke: Box::new(invoke),
    });
    // SAFETY: NSObject initializes our retained subclass.
    let handler: Retained<Handler> =
      unsafe { msg_send![super(allocated), init] };
    let root = NSMenuItem::new(mtm);
    root.setTitle(ns_string!("File"));
    let submenu = NSMenu::new(mtm);
    submenu.setAutoenablesItems(false);
    // SAFETY: Handler implements this one-argument action.
    let open = unsafe {
      NSMenuItem::initWithTitle_action_keyEquivalent(
        NSMenuItem::alloc(mtm),
        ns_string!("Open…"),
        Some(sel!(openReader:)),
        ns_string!("o"),
      )
    };
    // SAFETY: Integration retains Handler until removal.
    unsafe { open.setTarget(Some(&handler)) };
    open.setEnabled(false);
    submenu.addItem(&open);
    root.setSubmenu(Some(&submenu));
    menu.addItem(&root);
    Some(Self {
      handler,
      window,
      menu,
      root,
      open,
      panel: None,
      alive: Rc::new(Cell::new(true)),
      #[cfg(feature = "bench-events")]
      identity,
    })
  }

  pub fn enable(&self, enabled: bool) {
    if self.handler.ivars().enabled.replace(enabled) != enabled {
      self.open.setEnabled(enabled);
    }
  }

  pub fn pick(&mut self, complete: impl Fn(Vec<PathBuf>) + 'static) {
    self.cancel();
    let panel = NSOpenPanel::openPanel(self.handler.mtm());
    panel.setTitle(Some(ns_string!("Open PDF")));
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(false);
    let selected = objc2::rc::Weak::from_retained(&panel);
    let alive = Rc::clone(&self.alive);
    let handler = RcBlock::new(move |result| {
      if !alive.get() {
        return;
      }
      let Some(selected) = selected.load() else { return };
      let paths = if result == NSModalResponseOK {
        selected
          .URLs()
          .iter()
          .filter_map(|url| {
            url
              .isFileURL()
              .then(|| url.path())
              .flatten()
              .map(|path| PathBuf::from(path.to_string()))
          })
          .collect()
      } else {
        Vec::new()
      };
      complete(paths);
    });
    panel.beginSheetModalForWindow_completionHandler(&self.window, &handler);
    self.panel = Some(panel);
  }

  pub fn cancel(&mut self) {
    if let Some(panel) = self.panel.take() {
      // SAFETY: AppKit permits nil action senders.
      unsafe { panel.cancel(None) };
    }
  }
}

impl Drop for Integration {
  fn drop(&mut self) {
    self.alive.set(false);
    self.cancel();
    self.menu.removeItem(&self.root);
    // SAFETY: Disconnect before releasing the retained target.
    unsafe { self.open.setTarget(None) };
  }
}

#[cfg(feature = "bench-events")]
mod proof;
