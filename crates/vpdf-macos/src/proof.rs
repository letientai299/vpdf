//! Native integration probes without window activation.
use super::Integration;
use objc2::rc::Retained;
use objc2::{ClassType, MainThreadOnly, msg_send};
use objc2_app_kit::{
  NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSMenu,
  NSMenuItem, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSPoint, ns_string};

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
  pub fn invoke_menu(&self) {
    if let Some(menu) = self.root.submenu() {
      menu.performActionForItemAtIndex(0);
    }
  }

  pub fn invoke_key(&self) -> bool {
    // SAFETY: Arguments match AppKit's key-event factory.
    let event: Option<Retained<NSEvent>> = unsafe {
      msg_send![NSEvent::class(),
        keyEventWithType: NSEventType::KeyDown,
        location: NSPoint::new(0.0, 0.0),
        modifierFlags: NSEventModifierFlags::Command,
        timestamp: 0.0_f64,
        windowNumber: self.window.windowNumber(),
        context: std::ptr::null::<objc2_app_kit::NSGraphicsContext>(),
        characters: ns_string!("o"),
        charactersIgnoringModifiers: ns_string!("o"),
        isARepeat: false,
        keyCode: 31_u16,
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
