//! Native integration probes without window activation.
use super::Integration;
use objc2::rc::Retained;
use objc2::{ClassType, DefinedClass, MainThreadOnly, msg_send};
use objc2_app_kit::{
  NSApplication, NSEvent, NSEventType, NSMenu, NSMenuItem,
  NSStandardKeyBindingResponding, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSPoint};

pub(super) struct Identity {
  app: usize,
  window: usize,
  menu: Option<Retained<NSMenuItem>>,
}

impl Identity {
  pub(super) fn capture(
    window: &NSWindow,
    menu: &NSMenu,
    mtm: MainThreadMarker,
  ) -> Self {
    Self {
      app: NSApplication::sharedApplication(mtm)
        .delegate()
        .map_or(0, |d| Retained::as_ptr(&d).cast::<()>() as usize),
      window: window
        .delegate()
        .map_or(0, |d| Retained::as_ptr(&d).cast::<()>() as usize),
      menu: menu.itemAtIndex(0),
    }
  }
}

impl Integration {
  pub fn invoke_menu(&self, title: &str) {
    let application = self.menu.itemAtIndex(0);
    for root in application.iter().chain(&self.roots) {
      let Some(menu) = root.submenu() else { continue };
      let found = (0..menu.numberOfItems()).find(|&index| {
        menu
          .itemAtIndex(index)
          .is_some_and(|item| item.title().to_string() == title)
      });
      if let Some(index) = found {
        menu.performActionForItemAtIndex(index);
        return;
      }
    }
  }

  pub fn invoke_key(&self, title: &str) -> bool {
    let entries = self.handler.ivars().entries.borrow();
    let Some(item) = entries
      .iter()
      .map(|entry| &entry.item)
      .chain(&self.items)
      .find(|item| item.title().to_string() == title)
      .cloned()
    else {
      return false;
    };
    drop(entries);
    let characters = item.keyEquivalent();
    // SAFETY: Arguments match AppKit's key-event factory.
    let event: Option<Retained<NSEvent>> = unsafe {
      msg_send![NSEvent::class(),
        keyEventWithType: NSEventType::KeyDown,
        location: NSPoint::new(0.0, 0.0),
        modifierFlags: item.keyEquivalentModifierMask(),
        timestamp: 0.0_f64,
        windowNumber: self.window.windowNumber(),
        context: std::ptr::null::<objc2_app_kit::NSGraphicsContext>(),
        characters: &*characters,
        charactersIgnoringModifiers: &*characters,
        isARepeat: false,
        keyCode: 0_u16,
      ]
    };
    let Some(event) = event else { return false };
    self.menu.performKeyEquivalent(&event)
  }

  /// Opens the first Open Recent file.
  pub fn invoke_recent(&self) {
    super::fill_recent(&self.handler, &self.recent);
    if self.recent.numberOfItems() > 1 {
      self.recent.performActionForItemAtIndex(0);
    }
  }

  pub fn represented(&self) -> Option<std::path::PathBuf> {
    self.window.representedURL()?.to_file_path()
  }

  pub fn minimize(&self) {
    self.window.miniaturize(None);
  }

  pub fn minimized(&self) -> bool {
    self.window.isMiniaturized()
  }

  /// Delegates, application menu, sheet parent, command availability.
  pub fn inspect(&self) -> (bool, bool, bool, bool) {
    let current =
      Identity::capture(&self.window, &self.menu, self.handler.mtm());
    let delegates = current.app != 0
      && current.app == self.identity.app
      && current.window == self.identity.window;
    let menu = current
      .menu
      .as_ref()
      .zip(self.identity.menu.as_ref())
      .is_some_and(|(a, b)| std::ptr::eq(&**a, &**b));
    let sheet = self
      .window
      .attachedSheet()
      .and_then(|sheet| sheet.sheetParent())
      .is_some_and(|parent| std::ptr::eq(&*parent, &*self.window));
    (delegates, menu, sheet, self.open.isEnabled())
  }

  /// Clicks the toolbar button with `label`.
  pub fn invoke_tool(&self, label: &str) {
    let Some(bar) = self.handler.ivars().bar.get() else { return };
    let app = NSApplication::sharedApplication(self.handler.mtm());
    for super::toolbar::Control { item, .. } in bar.items.iter().flatten() {
      if item.label().to_string() == label
        && let Some(action) = item.action()
      {
        // SAFETY: Toolbar actions take their item.
        unsafe {
          app.sendAction_to_from(action, item.target().as_deref(), Some(item))
        };
      }
    }
  }

  /// Types `text` into the page field, then presses Return or Escape.
  pub fn submit_page(&self, text: &str, cancel: bool) {
    let Some(bar) = self.handler.ivars().bar.get() else { return };
    self.window.makeFirstResponder(Some(&bar.field));
    let Some(editor) = bar.field.currentEditor() else { return };
    editor.setString(&objc2_foundation::NSString::from_str(text));
    let key = if cancel {
      objc2::sel!(cancelOperation:)
    } else {
      objc2::sel!(insertNewline:)
    };
    // SAFETY: Field editors handle these standard commands.
    unsafe { editor.doCommandBySelector(key) };
  }

  /// Starts editing the page field.
  pub fn edit_page(&self) -> bool {
    let Some(bar) = self.handler.ivars().bar.get() else { return false };
    self.window.makeFirstResponder(Some(&bar.field))
  }

  /// Visibility, editing, page text, and enabled labels.
  pub fn toolbar_state(&self) -> String {
    let Some(bar) = self.handler.ivars().bar.get() else {
      return "none".into();
    };
    let enabled: Vec<_> = bar
      .items
      .iter()
      .flatten()
      .filter(|control| control.item.isEnabled())
      .map(|control| control.item.label().to_string())
      .collect();
    let content = self.window.firstResponder().is_some_and(|responder| {
      bar.view.load().is_some_and(|view| {
        std::ptr::eq(
          Retained::as_ptr(&responder).cast::<()>(),
          Retained::as_ptr(&view).cast(),
        )
      })
    });
    format!(
      "visible={} editing={} content={content} page={:?} count={:?} \
       enabled={enabled:?}",
      bar.toolbar.isVisible(),
      bar.editing(),
      bar.field.stringValue().to_string(),
      bar.count.stringValue().to_string(),
    )
  }

  /// Application menu titles, enabled ones marked with `*`.
  pub fn application_menu(&self) -> Vec<String> {
    let Some(app) = self.menu.itemAtIndex(0).and_then(|root| root.submenu())
    else {
      return Vec::new();
    };
    app.update();
    (0..app.numberOfItems())
      .filter_map(|index| app.itemAtIndex(index))
      .map(|item| {
        let mark = if item.isEnabled() { "*" } else { "" };
        format!("{}{mark}", item.title())
      })
      .collect()
  }
}
