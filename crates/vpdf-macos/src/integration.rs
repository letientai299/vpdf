//! Main-thread menu and window-sheet adapter.
use crate::placement;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{
  ClassType, DefinedClass, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
  NSApplication, NSDocumentController, NSEventModifierFlags, NSMenu,
  NSMenuDelegate, NSMenuItem, NSModalResponseOK, NSOpenPanel, NSPasteboard,
  NSPasteboardTypeString, NSScreen, NSView, NSWindow, NSWorkspace,
};
use objc2_foundation::{
  MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect,
  NSSize, NSString, NSURL, NSUserDefaults, ns_string,
};
use objc2_uniform_type_identifiers::UTTypePDF;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// A menu choice for the worker.
pub enum Request {
  /// Index into the installed entries.
  Menu(usize),
  /// An Open Recent file.
  Open(PathBuf),
  /// Work scheduled by `deferrer`, outside the scheduling callback.
  Deferred,
}

struct Callback {
  invoke: Box<dyn Fn(Request)>,
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
        (self.ivars().invoke)(Request::Menu(sender.tag() as usize));
      }
    }

    // SAFETY: These items supply their sender.
    #[unsafe(method(openRecent:))]
    fn open_recent(&self, sender: &NSMenuItem) {
      let path = sender.representedObject().and_then(|object| {
        object.downcast_ref::<NSURL>().and_then(NSURL::to_file_path)
      });
      if let Some(path) = path {
        (self.ivars().invoke)(Request::Open(path));
      }
    }

    // SAFETY: Delayed performs pass a nil object.
    #[unsafe(method(deferred:))]
    fn deferred(&self, _object: Option<&AnyObject>) {
      (self.ivars().invoke)(Request::Deferred);
    }

    // SAFETY: These items supply their sender.
    #[unsafe(method(clearRecent:))]
    fn clear_recent(&self, _sender: &NSMenuItem) {
      let documents =
        NSDocumentController::sharedDocumentController(self.mtm());
      // SAFETY: AppKit permits nil action senders.
      unsafe { documents.clearRecentDocuments(None) };
    }
  }

  // SAFETY: Only the Open Recent menu uses this delegate.
  unsafe impl NSMenuDelegate for Handler {
    #[unsafe(method(menuNeedsUpdate:))]
    fn menu_needs_update(&self, menu: &NSMenu) {
      fill_recent(self, menu);
    }
  }
);

// Our own selectors keep AppKit from managing this menu.
fn fill_recent(handler: &Handler, menu: &NSMenu) {
  let mtm = handler.mtm();
  menu.removeAllItems();
  let urls =
    NSDocumentController::sharedDocumentController(mtm).recentDocumentURLs();
  for url in &urls {
    let Some(name) = url.lastPathComponent() else { continue };
    let item = menu_item(mtm, &name, sel!(openRecent:), handler);
    // SAFETY: Menu items retain any represented object.
    unsafe { item.setRepresentedObject(Some(&url)) };
    menu.addItem(&item);
  }
  if !urls.is_empty() {
    menu.addItem(&NSMenuItem::separatorItem(mtm));
  }
  let clear =
    menu_item(mtm, ns_string!("Clear Menu"), sel!(clearRecent:), handler);
  clear.setEnabled(!urls.is_empty());
  menu.addItem(&clear);
}

fn menu_item(
  mtm: MainThreadMarker,
  title: &NSString,
  action: Sel,
  handler: &Handler,
) -> Retained<NSMenuItem> {
  // SAFETY: Handler implements each one-argument action.
  let item = unsafe {
    NSMenuItem::initWithTitle_action_keyEquivalent(
      NSMenuItem::alloc(mtm),
      title,
      Some(action),
      ns_string!(""),
    )
  };
  // SAFETY: Integration retains Handler until removal.
  unsafe { item.setTarget(Some(handler)) };
  item
}

pub struct Integration {
  handler: Retained<Handler>,
  window: Retained<NSWindow>,
  menu: Retained<NSMenu>,
  roots: Vec<Retained<NSMenuItem>>,
  items: Vec<Retained<NSMenuItem>>,
  recent: Retained<NSMenu>,
  recent_root: Retained<NSMenuItem>,
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
    invoke: impl Fn(Request) + 'static,
  ) -> Option<Self> {
    let mtm = MainThreadMarker::new()?;
    let borrowed = parent.window_handle().ok()?;
    let RawWindowHandle::AppKit(raw) = borrowed.as_raw() else { return None };
    // SAFETY: Borrowed AppKit handles guarantee live NSViews.
    let view = unsafe { raw.ns_view.cast::<NSView>().as_ref() };
    let window = view.window()?;
    restore_frame(&window, mtm);
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
      let item = menu_item(
        mtm,
        &NSString::from_str(entry.title),
        sel!(openReader:),
        &handler,
      );
      item.setKeyEquivalent(&NSString::from_str(entry.key));
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
      item.setEnabled(false);
      submenu.addItem(&item);
      items.push(item);
    }
    #[cfg(feature = "bench-events")]
    let open = items.first()?.clone();
    // Open Recent follows the first entry.
    let recent = NSMenu::new(mtm);
    recent.setAutoenablesItems(false);
    recent.setDelegate(Some(ProtocolObject::from_ref(&*handler)));
    let recent_root = NSMenuItem::new(mtm);
    recent_root.setTitle(ns_string!("Open Recent"));
    recent_root.setSubmenu(Some(&recent));
    recent_root.setEnabled(false);
    roots.first()?.submenu()?.insertItem_atIndex(&recent_root, 1);
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
      recent,
      recent_root,
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

  /// Open Recent follows the Open command.
  pub fn enable_recent(&self, enabled: bool) {
    self.recent_root.setEnabled(enabled);
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
    panel.setAllowsMultipleSelection(true);
    // SAFETY: UniformTypeIdentifiers owns this constant.
    panel.setAllowedContentTypes(&NSArray::from_slice(&[unsafe { UTTypePDF }]));
    let selected = objc2::rc::Weak::from_retained(&panel);
    let alive = Rc::clone(&self.alive);
    let handler = RcBlock::new(move |result| {
      if !alive.get() {
        return;
      }
      let Some(selected) = selected.load() else { return };
      let paths = if result == NSModalResponseOK {
        selected.URLs().iter().filter_map(|url| url.to_file_path()).collect()
      } else {
        Vec::new()
      };
      complete(paths);
    });
    panel.beginSheetModalForWindow_completionHandler(&self.window, &handler);
    self.panel = Some(panel);
  }

  /// Sets the title-bar document; None clears it.
  pub fn represent(&self, path: Option<&Path>) {
    let url = path.and_then(NSURL::from_file_path);
    self.window.setRepresentedURL(url.as_deref());
  }

  pub fn note_recent(&self, path: &Path) {
    if let Some(url) = NSURL::from_file_path(path) {
      NSDocumentController::sharedDocumentController(self.handler.mtm())
        .noteNewRecentDocumentURL(&url);
    }
  }

  pub fn reveal(&self, path: &Path) {
    if let Some(url) = NSURL::from_file_path(path) {
      NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
    }
  }

  /// Shows a minimized window only for an active app.
  pub fn restorer(&self) -> impl Fn() + 'static {
    let window = objc2::rc::Weak::from_retained(&self.window);
    let mtm = self.handler.mtm();
    move || {
      let Some(window) = window.load() else { return };
      if NSApplication::sharedApplication(mtm).isActive()
        && window.isMiniaturized()
      {
        window.deminiaturize(None);
      }
    }
  }

  /// Schedules `Request::Deferred` on the main run loop; minimized
  /// windows still run it.
  pub fn deferrer(&self) -> impl Fn() + 'static {
    let handler = objc2::rc::Weak::from_retained(&self.handler);
    move || {
      let Some(handler) = handler.load() else { return };
      // SAFETY: Handler implements `deferred:`, which takes a nullable object.
      let () = unsafe {
        msg_send![&*handler, performSelector: sel!(deferred:),
          withObject: None::<&AnyObject>, afterDelay: 0.0f64]
      };
    }
  }

  pub fn cancel(&mut self) {
    if let Some(panel) = self.panel.take() {
      // SAFETY: AppKit permits nil action senders.
      unsafe { panel.cancel(None) };
    }
  }
}

/// Quits through AppKit, which ends the event loop without a frame.
pub fn terminate() {
  if let Some(mtm) = MainThreadMarker::new() {
    NSApplication::sharedApplication(mtm).terminate(None);
  }
}

/// Whether the system reopens windows on relaunch; unset means yes.
pub fn reopens_windows() -> bool {
  let defaults = NSUserDefaults::standardUserDefaults();
  let key = ns_string!("NSQuitAlwaysKeepsWindows");
  defaults.objectForKey(key).is_none() || defaults.boolForKey(key)
}

/// Reapplies the saved frame where a display can show it.
fn restore_frame(window: &NSWindow, mtm: MainThreadMarker) {
  let name = ns_string!("Vpdf");
  window.setFrameUsingName(name);
  window.setFrameAutosaveName(name);
  let rect = |frame: NSRect| {
    [frame.origin.x, frame.origin.y, frame.size.width, frame.size.height]
  };
  let areas: Vec<_> = NSScreen::screens(mtm)
    .iter()
    .map(|screen| rect(screen.visibleFrame()))
    .collect();
  let Some(main) = NSScreen::mainScreen(mtm) else { return };
  // AppKit's frame setters skip the minimum size.
  let min = window.minSize();
  let fitted = placement::fit(
    rect(window.frame()),
    [min.width, min.height],
    &areas,
    rect(main.visibleFrame()),
  );
  if let Some([x, y, width, height]) = fitted {
    let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
    window.setFrame_display(frame, false);
  }
}

impl Drop for Integration {
  fn drop(&mut self) {
    self.alive.set(false);
    // SAFETY: NSObject's class method takes any target.
    let () = unsafe {
      msg_send![NSObject::class(),
        cancelPreviousPerformRequestsWithTarget: &*self.handler]
    };
    self.cancel();
    self.recent.setDelegate(None);
    self.recent.removeAllItems();
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
#[path = "proof.rs"]
mod proof;
