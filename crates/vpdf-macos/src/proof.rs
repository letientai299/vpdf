//! Native integration probes without window activation.
use super::Integration;
use objc2::rc::Retained;
use objc2::{ClassType, MainThreadOnly, msg_send};
use objc2_app_kit::{
  NSApplication, NSEvent, NSEventType, NSMenu, NSMenuItem, NSWindow,
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
    for root in &self.roots {
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
    let Some(item) =
      self.items.iter().find(|item| item.title().to_string() == title)
    else {
      return false;
    };
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
}
