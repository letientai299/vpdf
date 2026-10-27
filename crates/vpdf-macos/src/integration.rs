//! Main-thread menu and window-sheet adapter.
use crate::placement;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{
  ClassType, DefinedClass, MainThreadOnly, Message, define_class, msg_send, sel,
};
use objc2_app_kit::{
  NSApplication, NSControl, NSControlTextEditingDelegate, NSDocumentController,
  NSEventModifierFlags, NSMenu, NSMenuDelegate, NSMenuItem, NSModalResponseOK,
  NSOpenPanel, NSPasteboard, NSPasteboardTypeString, NSScreen,
  NSTextFieldDelegate, NSTextView, NSToolbar, NSToolbarDelegate, NSToolbarItem,
  NSView, NSWindow, NSWorkspace, NSWorkspaceOpenConfiguration,
};
use objc2_foundation::{
  MainThreadMarker, NSArray, NSNotification, NSObject, NSObjectProtocol,
  NSPoint, NSRect, NSSize, NSString, NSURL, NSUserDefaults, ns_string,
};
use objc2_uniform_type_identifiers::{UTTypePDF, UTTypePlainText};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, OnceCell, RefCell};
use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
pub use toolbar::Tool;

/// User default that keeps the toolbar hidden.
const HIDDEN: &str = "VpdfToolbarHidden";

/// A menu choice for the worker.
pub enum Request {
  /// Index into the installed entries.
  Menu(usize),
  /// An Open Recent file.
  Open(PathBuf),
  /// A one-based page from the toolbar.
  Page(usize),
  /// Work scheduled by `deferrer`, outside the scheduling callback.
  Deferred,
}

struct Callback {
  invoke: Box<dyn Fn(Request)>,
  entries: RefCell<Vec<Entry>>,
  bar: OnceCell<toolbar::Bar>,
  toggle: OnceCell<Retained<NSMenuItem>>,
}

struct Entry {
  item: Retained<NSMenuItem>,
  enabled: bool,
  /// The standard action while toolbar text is editing.
  responder: Option<Sel>,
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
      let index = sender.tag() as usize;
      if let Some(action) = self.forwarded(index) {
        let app = NSApplication::sharedApplication(self.mtm());
        // SAFETY: Standard edit actions take any sender.
        unsafe { app.sendAction_to_from(action, None, Some(sender)) };
        return;
      }
      self.choose(index, sender.isEnabled());
    }

    // SAFETY: These items supply their sender.
    #[unsafe(method(toolbarAction:))]
    fn toolbar_action(&self, sender: &NSToolbarItem) {
      self.choose(sender.tag() as usize, sender.isEnabled());
    }

    // SAFETY: Menus pass the item they validate.
    #[unsafe(method(validateMenuItem:))]
    fn validate(&self, item: &NSMenuItem) -> bool {
      let index = item.tag() as usize;
      let entries = self.ivars().entries.borrow();
      match self.forwarded(index) {
        Some(action) => self.ivars().bar.get().is_some_and(|b| b.can(action)),
        None => {
          item.action() != Some(sel!(openReader:))
            || entries.get(index).is_some_and(|entry| entry.enabled)
        }
      }
    }

    // SAFETY: These items supply their sender.
    #[unsafe(method(toggleToolbar:))]
    fn toggle_toolbar(&self, _sender: &NSMenuItem) {
      let Some(bar) = self.ivars().bar.get() else { return };
      bar.focus_content();
      let visible = !bar.toolbar.isVisible();
      bar.toolbar.setVisible(visible);
      NSUserDefaults::standardUserDefaults()
        .setBool_forKey(!visible, &NSString::from_str(HIDDEN));
      self.retitle_toggle();
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

  // SAFETY: Only the toolbar uses this delegate.
  unsafe impl NSToolbarDelegate for Handler {
    // Autoreleased: `method_id` exceeds 80 columns.
    #[unsafe(method(toolbar:itemForItemIdentifier:willBeInsertedIntoToolbar:))]
    fn toolbar_item(
      &self,
      _toolbar: &NSToolbar,
      id: &NSString,
      _inserted: bool,
    ) -> *mut NSToolbarItem {
      let item = self.ivars().bar.get().and_then(|bar| bar.item(id));
      item.map_or(std::ptr::null_mut(), Retained::autorelease_return)
    }

    #[unsafe(method_id(toolbarDefaultItemIdentifiers:))]
    fn defaults(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
      self.identifiers()
    }

    #[unsafe(method_id(toolbarAllowedItemIdentifiers:))]
    fn allowed(&self, _toolbar: &NSToolbar) -> Retained<NSArray<NSString>> {
      self.identifiers()
    }
  }

  // SAFETY: Only the toolbar page field uses this delegate.
  unsafe impl NSControlTextEditingDelegate for Handler {
    #[unsafe(method(controlTextDidEndEditing:))]
    fn ended(&self, _notification: &NSNotification) {
      if let Some(bar) = self.ivars().bar.get() {
        bar.revert();
      }
    }

    #[unsafe(method(control:textView:doCommandBySelector:))]
    unsafe fn command(
      &self,
      _control: &NSControl,
      view: &NSTextView,
      selector: Sel,
    ) -> bool {
      self.field_command(view, selector)
    }
  }

  // SAFETY: The protocol adds no required methods.
  unsafe impl NSTextFieldDelegate for Handler {}

  // SAFETY: Only the Open Recent menu uses this delegate.
  unsafe impl NSMenuDelegate for Handler {
    #[unsafe(method(menuNeedsUpdate:))]
    fn menu_needs_update(&self, menu: &NSMenu) {
      fill_recent(self, menu);
    }
  }
);

impl Handler {
  /// Return submits the page; Escape and Tab abandon it. All leave
  /// the field, whose end of editing restores the shown page.
  fn field_command(&self, view: &NSTextView, selector: Sel) -> bool {
    let Some(bar) = self.ivars().bar.get() else { return false };
    let submit = selector == sel!(insertNewline:);
    let leave =
      [sel!(cancelOperation:), sel!(insertTab:), sel!(insertBacktab:)];
    if !submit && !leave.contains(&selector) {
      return false;
    }
    let page = toolbar::requested_page(&view.string().to_string());
    bar.focus_content();
    if let Some(page) = page.filter(|_| submit) {
      (self.ivars().invoke)(Request::Page(page));
    }
    true
  }

  /// The standard action replacing `index` while the page field edits.
  fn forwarded(&self, index: usize) -> Option<Sel> {
    let editing = self.ivars().bar.get().is_some_and(toolbar::Bar::editing);
    let entries = self.ivars().entries.borrow();
    entries.get(index).and_then(|entry| entry.responder).filter(|_| editing)
  }

  /// Leaves page editing, then sends an enabled entry.
  fn choose(&self, index: usize, enabled: bool) {
    if let Some(bar) = self.ivars().bar.get() {
      bar.focus_content();
    }
    if enabled {
      (self.ivars().invoke)(Request::Menu(index));
    }
  }

  fn identifiers(&self) -> Retained<NSArray<NSString>> {
    self.ivars().bar.get().map_or_else(NSArray::new, toolbar::Bar::identifiers)
  }

  fn retitle_toggle(&self) {
    let Some(bar) = self.ivars().bar.get() else { return };
    let Some(toggle) = self.ivars().toggle.get() else { return };
    let visible = bar.toolbar.isVisible();
    toggle.setTitle(if visible {
      ns_string!("Hide Toolbar")
    } else {
      ns_string!("Show Toolbar")
    });
  }
}

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
  /// Items added to the application menu.
  extras: Vec<Retained<NSMenuItem>>,
  /// Window actions outside the entries.
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
  /// Empty for the application menu.
  pub menu: &'static str,
  pub title: &'static str,
  pub key: &'static str,
  pub shift: bool,
  pub control: bool,
  /// The standard action while toolbar text is editing.
  pub responder: Option<&'static CStr>,
}

impl Integration {
  pub fn install(
    parent: &impl HasWindowHandle,
    entries: &[Item],
    tools: &[Tool],
    invoke: impl Fn(Request) + 'static,
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
      invoke: Box::new(invoke),
      entries: RefCell::default(),
      bar: OnceCell::new(),
      toggle: OnceCell::new(),
    });
    // SAFETY: NSObject initializes our retained subclass.
    let handler: Retained<Handler> =
      unsafe { msg_send![super(allocated), init] };
    let mut roots: Vec<Retained<NSMenuItem>> = Vec::new();
    let mut items = Vec::new();
    let mut installed = Vec::new();
    let mut previous = "";
    let mut submenu = NSMenu::new(mtm);
    let mut extras = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
      if !entry.menu.is_empty() && entry.menu != previous {
        let root = NSMenuItem::new(mtm);
        root.setTitle(&NSString::from_str(entry.menu));
        submenu = NSMenu::new(mtm);
        // Validation enables standard edits during page editing.
        submenu.setAutoenablesItems(
          entries
            .iter()
            .any(|other| other.menu == entry.menu && other.responder.is_some()),
        );
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
      if entry.menu.is_empty() {
        add_application(&menu, &item, &mut extras);
      } else {
        submenu.addItem(&item);
      }
      installed.push(Entry {
        item,
        enabled: false,
        responder: entry.responder.map(Sel::register),
      });
    }
    #[cfg(feature = "bench-events")]
    let open = installed.first()?.item.clone();
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
    let toggle = menu_item(
      mtm,
      ns_string!("Hide Toolbar"),
      sel!(toggleToolbar:),
      &handler,
    );
    toggle.setKeyEquivalent(ns_string!("t"));
    toggle.setKeyEquivalentModifierMask(
      NSEventModifierFlags::Command | NSEventModifierFlags::Option,
    );
    if let Some(view_menu) = roots
      .iter()
      .find(|item| item.title().to_string() == "View")
      .and_then(|root| root.submenu())
    {
      view_menu.addItem(&NSMenuItem::separatorItem(mtm));
      view_menu.addItem(&toggle);
      items.push(toggle.clone());
      let _ = handler.ivars().toggle.set(toggle);
    }
    *handler.ivars().entries.borrow_mut() = installed;
    let bar = toolbar::Bar::new(&handler, tools, entries, &window, view);
    let hidden = NSUserDefaults::standardUserDefaults()
      .boolForKey(&NSString::from_str(HIDDEN));
    let _ = handler.ivars().bar.set(bar);
    if let Some(bar) = handler.ivars().bar.get() {
      bar.attach(&handler, &window, !hidden);
    }
    handler.retitle_toggle();
    restore_frame(&window, mtm);
    Some(Self {
      handler,
      window,
      menu,
      roots,
      extras,
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

  /// Enables each entry by index, with the controls that show it.
  pub fn enable(&self, states: &[bool]) {
    let items: Vec<_> = {
      let mut entries = self.handler.ivars().entries.borrow_mut();
      for (entry, &enabled) in entries.iter_mut().zip(states) {
        entry.enabled = enabled;
      }
      entries.iter().map(|entry| (entry.item.clone(), entry.enabled)).collect()
    };
    for (item, enabled) in items {
      item.setEnabled(enabled);
    }
    if let Some(bar) = self.handler.ivars().bar.get() {
      bar.enable(states);
    }
  }

  pub fn retitle(&self, index: usize, title: &str) {
    let item =
      self.handler.ivars().entries.borrow().get(index).map(|e| e.item.clone());
    let title = NSString::from_str(title);
    if let Some(item) = item {
      item.setTitle(&title);
    }
    if let Some(bar) = self.handler.ivars().bar.get() {
      bar.retitle(index, &title);
    }
  }

  /// Shows the one-based page of a count; `enabled` allows jumps.
  pub fn page(&self, location: Option<(usize, usize)>, enabled: bool) {
    if let Some(bar) = self.handler.ivars().bar.get() {
      bar.page(location, enabled);
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

  /// Opens `path` in the default text editor, else TextEdit.
  pub fn edit(&self, path: &Path) {
    let Some(url) = NSURL::from_file_path(path) else { return };
    let workspace = NSWorkspace::sharedWorkspace();
    // macOS 12 added the content-type lookup.
    let default = workspace
      .respondsToSelector(sel!(URLForApplicationToOpenContentType:))
      .then(|| {
        // SAFETY: UniformTypeIdentifiers owns this constant.
        workspace.URLForApplicationToOpenContentType(unsafe { UTTypePlainText })
      })
      .flatten();
    let Some(editor) = default.or_else(|| {
      workspace
        .URLForApplicationWithBundleIdentifier(ns_string!("com.apple.TextEdit"))
    }) else {
      eprintln!("No text editor for {}", path.display());
      return;
    };
    let shown = path.display().to_string();
    let done = RcBlock::new(move |_, error: *mut objc2_foundation::NSError| {
      if !error.is_null() {
        eprintln!("Editor failed to open {shown}");
      }
    });
    workspace.openURLs_withApplicationAtURL_configuration_completionHandler(
      &NSArray::from_retained_slice(&[url]),
      &editor,
      &NSWorkspaceOpenConfiguration::configuration(),
      Some(&done),
    );
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

/// Adds `item` and a separator after the application menu's About.
fn add_application(
  menu: &NSMenu,
  item: &NSMenuItem,
  added: &mut Vec<Retained<NSMenuItem>>,
) {
  let Some(app) = menu.itemAtIndex(0).and_then(|root| root.submenu()) else {
    return;
  };
  let at = app.numberOfItems().min(2);
  let separator = NSMenuItem::separatorItem(app.mtm());
  app.insertItem_atIndex(&separator, at);
  app.insertItem_atIndex(item, at);
  added.extend([item.retain(), separator]);
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
    if let Some(bar) = self.handler.ivars().bar.get() {
      bar.detach();
    }
    for root in &self.roots {
      self.menu.removeItem(root);
    }
    if let Some(app) = self.menu.itemAtIndex(0).and_then(|root| root.submenu())
    {
      for extra in &self.extras {
        if app.indexOfItem(extra) >= 0 {
          app.removeItem(extra);
        }
      }
    }
    let entries = self.handler.ivars().entries.borrow();
    for item in self.items.iter().chain(entries.iter().map(|e| &e.item)) {
      // SAFETY: Disconnect before releasing the retained target.
      unsafe { item.setTarget(None) };
    }
  }
}

#[cfg(feature = "bench-events")]
#[path = "proof.rs"]
mod proof;
#[path = "toolbar.rs"]
mod toolbar;
