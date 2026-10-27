//! Main-thread menu and window-sheet adapter.
#![cfg(target_os = "macos")]
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
  NSApplication, NSEventModifierFlags, NSMenu, NSMenuItem, NSModalResponseOK,
  NSOpenPanel, NSPasteboard, NSPasteboardTypeString, NSView, NSWindow,
};
use objc2_foundation::{
  MainThreadMarker, NSObject, NSObjectProtocol, NSString, ns_string,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

struct Callback {
  invoke: Box<dyn Fn(usize)>,
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
    // SAFETY: These items supply their sender.
    #[unsafe(method(openReader:))]
    fn open(&self, sender: &NSMenuItem) {
      if sender.isEnabled() {
        (self.ivars().invoke)(sender.tag() as usize);
      }
    }
  }
);

pub struct Integration {
  handler: Retained<Handler>,
  window: Retained<NSWindow>,
  menu: Retained<NSMenu>,
  roots: Vec<Retained<NSMenuItem>>,
  items: Vec<Retained<NSMenuItem>>,
  #[cfg(feature = "bench-events")]
  open: Retained<NSMenuItem>,
  panel: Option<Retained<NSOpenPanel>>,
  alive: Rc<Cell<bool>>,
  #[cfg(feature = "bench-events")]
  identity: proof::Identity,
}

pub struct Item {
  pub menu: &'static str,
  pub title: &'static str,
  pub key: &'static str,
  pub shift: bool,
  pub control: bool,
}

impl Integration {
  pub fn install(
    parent: &impl HasWindowHandle,
    entries: &[Item],
    invoke: impl Fn(usize) + 'static,
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
    let allocated =
      Handler::alloc(mtm).set_ivars(Callback { invoke: Box::new(invoke) });
    // SAFETY: NSObject initializes our retained subclass.
    let handler: Retained<Handler> =
      unsafe { msg_send![super(allocated), init] };
    let mut roots: Vec<Retained<NSMenuItem>> = Vec::new();
    let mut items = Vec::new();
    let mut previous = "";
    let mut submenu = NSMenu::new(mtm);
    for (index, entry) in entries.iter().enumerate() {
      if entry.menu != previous {
        let root = NSMenuItem::new(mtm);
        root.setTitle(&NSString::from_str(entry.menu));
        submenu = NSMenu::new(mtm);
        submenu.setAutoenablesItems(false);
        root.setSubmenu(Some(&submenu));
        menu.addItem(&root);
        roots.push(root);
        previous = entry.menu;
      }
      // SAFETY: Handler implements this one-argument action.
      let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
          NSMenuItem::alloc(mtm),
          &NSString::from_str(entry.title),
          Some(sel!(openReader:)),
          &NSString::from_str(entry.key),
        )
      };
      item.setTag(index as isize);
      let mut modifiers = if entry.control {
        NSEventModifierFlags::Control
      } else {
        NSEventModifierFlags::Command
      };
      if entry.shift {
        modifiers |= NSEventModifierFlags::Shift;
      }
      item.setKeyEquivalentModifierMask(modifiers);
      // SAFETY: Integration retains Handler until removal.
      unsafe { item.setTarget(Some(&handler)) };
      item.setEnabled(false);
      submenu.addItem(&item);
      items.push(item);
    }
    #[cfg(feature = "bench-events")]
    let open = items.first()?.clone();
    for (title, selector) in [
      ("Minimize", sel!(performMiniaturize:)),
      ("Zoom Window", sel!(performZoom:)),
      ("Enter Full Screen", sel!(toggleFullScreen:)),
    ] {
      // SAFETY: NSWindow implements these AppKit actions.
      let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
          NSMenuItem::alloc(mtm),
          &NSString::from_str(title),
          Some(selector),
          ns_string!(""),
        )
      };
      // SAFETY: Integration retains the target window.
      unsafe { item.setTarget(Some(&window)) };
      let Some(submenu) = roots
        .iter()
        .find(|item| item.title().to_string() == "Window")
        .and_then(|root| root.submenu())
      else {
        continue;
      };
      submenu.addItem(&item);
      items.push(item);
    }
    Some(Self {
      handler,
      window,
      menu,
      roots,
      items,
      #[cfg(feature = "bench-events")]
      open,
      panel: None,
      alive: Rc::new(Cell::new(true)),
      #[cfg(feature = "bench-events")]
      identity,
    })
  }

  pub fn enable(&self, index: usize, enabled: bool) {
    if let Some(item) = self.items.get(index) {
      item.setEnabled(enabled);
    }
  }

  pub fn paste(&self) -> Option<String> {
    let board = NSPasteboard::generalPasteboard();
    // SAFETY: AppKit owns the string type constant.
    board
      .stringForType(unsafe { NSPasteboardTypeString })
      .map(|text| text.to_string())
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
    for root in &self.roots {
      self.menu.removeItem(root);
    }
    for item in &self.items {
      // SAFETY: Disconnect before releasing the retained target.
      unsafe { item.setTarget(None) };
    }
  }
}

#[cfg(feature = "bench-events")]
mod proof;
