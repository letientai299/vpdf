//! Window toolbar controls for installed menu entries.
use super::{Handler, Item, menu_item};
use objc2::rc::{Retained, Weak};
use objc2::runtime::ProtocolObject;
use objc2::runtime::Sel;
use objc2::{MainThreadOnly, Message, sel};
use objc2_app_kit::{
  NSImage, NSMenu, NSMenuItem, NSMenuToolbarItem, NSPasteboard,
  NSPasteboardTypeString, NSStackView, NSTextAlignment, NSTextField, NSToolbar,
  NSToolbarDisplayMode, NSToolbarFlexibleSpaceItemIdentifier, NSToolbarItem,
  NSToolbarItemVisibilityPriorityHigh, NSToolbarItemVisibilityPriorityUser,
  NSView, NSWindow,
};
use objc2_foundation::{
  NSArray, NSCopying, NSObjectProtocol, NSString, ns_string,
};
use std::cell::RefCell;

/// A toolbar control.
pub enum Tool {
  /// A button for an installed entry; `keep` resists overflow.
  Button {
    entry: usize,
    label: &'static str,
    symbol: &'static str,
    keep: bool,
  },
  /// A pull-down of installed entries.
  Menu {
    label: &'static str,
    symbol: &'static str,
    entries: Vec<usize>,
  },
  /// The page field and count.
  Page,
  Space,
}

pub(super) struct Control {
  pub item: Retained<NSToolbarItem>,
  /// Entries that enable it.
  pub entries: Vec<usize>,
}

pub(super) struct Bar {
  pub toolbar: Retained<NSToolbar>,
  /// None marks flexible space.
  pub items: Vec<Option<Control>>,
  pub pulldowns: Vec<Retained<NSMenuItem>>,
  pub page: Option<Retained<NSToolbarItem>>,
  pub field: Retained<NSTextField>,
  pub count: Retained<NSTextField>,
  /// The page text outside editing.
  pub shown: RefCell<String>,
  pub window: Weak<NSWindow>,
  pub view: Weak<NSView>,
}

impl Bar {
  pub fn new(
    handler: &Handler,
    tools: &[Tool],
    entries: &[Item],
    window: &NSWindow,
    view: &NSView,
  ) -> Self {
    let mtm = handler.mtm();
    let field = NSTextField::textFieldWithString(ns_string!(""), mtm);
    field.setAlignment(NSTextAlignment::Right);
    field.widthAnchor().constraintEqualToConstant(44.0).setActive(true);
    // SAFETY: Integration retains Handler until removal.
    unsafe { field.setDelegate(Some(ProtocolObject::from_ref(handler))) };
    field.setEnabled(false);
    let count = NSTextField::labelWithString(ns_string!(""), mtm);
    let mut pulldowns = Vec::new();
    let mut items = Vec::new();
    let mut page_item = None;
    for (index, tool) in tools.iter().enumerate() {
      let id = NSString::from_str(&format!("vpdf.{index}"));
      let control = match tool {
        Tool::Space => None,
        &Tool::Button { entry, label, symbol, keep } => Some(Control {
          item: button(handler, &id, entries, entry, [label, symbol], keep),
          entries: vec![entry],
        }),
        Tool::Menu { label, symbol, entries: chosen } => Some(Control {
          item: pulldown(
            handler,
            &id,
            entries,
            chosen,
            [label, symbol],
            &mut pulldowns,
          ),
          entries: chosen.clone(),
        }),
        Tool::Page => {
          let item = page(&id, &field, &count);
          page_item = Some(item.clone());
          Some(Control { item, entries: Vec::new() })
        }
      };
      if let Some(control) = &control {
        control.item.setAutovalidates(false);
        control.item.setEnabled(false);
      }
      items.push(control);
    }
    let toolbar =
      NSToolbar::initWithIdentifier(NSToolbar::alloc(mtm), ns_string!("Vpdf"));
    toolbar.setDisplayMode(NSToolbarDisplayMode::IconOnly);
    toolbar.setAllowsUserCustomization(false);
    // Right-click offers display modes from macOS 15.
    if toolbar.respondsToSelector(sel!(setAllowsDisplayModeCustomization:)) {
      toolbar.setAllowsDisplayModeCustomization(false);
    }
    Self {
      toolbar,
      items,
      pulldowns,
      page: page_item,
      field,
      count,
      shown: RefCell::default(),
      window: Weak::from_retained(&window.retain()),
      view: Weak::from_retained(&view.retain()),
    }
  }

  pub fn identifiers(&self) -> Retained<NSArray<NSString>> {
    let ids: Vec<_> = self
      .items
      .iter()
      .map(|item| match item {
        Some(control) => control.item.itemIdentifier(),
        // SAFETY: AppKit owns this constant.
        None => unsafe { NSToolbarFlexibleSpaceItemIdentifier }.copy(),
      })
      .collect();
    NSArray::from_retained_slice(&ids)
  }

  pub fn item(&self, id: &NSString) -> Option<Retained<NSToolbarItem>> {
    self
      .items
      .iter()
      .flatten()
      .find(|control| &*control.item.itemIdentifier() == id)
      .map(|control| control.item.clone())
  }

  /// Enables controls that show any enabled entry.
  pub fn enable(&self, enabled: &[bool]) {
    let on = |entry: usize| enabled.get(entry) == Some(&true);
    for Control { item, entries } in self.items.iter().flatten() {
      if !entries.is_empty() {
        item.setEnabled(entries.iter().any(|&entry| on(entry)));
      }
    }
    for choice in &self.pulldowns {
      choice.setEnabled(on(choice.tag() as usize));
    }
  }

  /// Gives buttons for `index` its menu title as a tooltip.
  pub fn retitle(&self, index: usize, title: &NSString) {
    for Control { item, entries } in self.items.iter().flatten() {
      if entries == &[index] {
        item.setToolTip(Some(title));
      }
    }
  }

  /// Whether the field editor can perform a standard edit `action`.
  pub fn can(&self, action: Sel) -> bool {
    let Some(editor) = self.field.currentEditor() else { return false };
    if action == sel!(copy:) || action == sel!(cut:) {
      editor.selectedRange().length > 0
    } else if action == sel!(undo:) {
      editor.undoManager().is_some_and(|undo| undo.canUndo())
    } else if action == sel!(paste:) {
      // SAFETY: AppKit owns the string type constant.
      NSPasteboard::generalPasteboard()
        .availableTypeFromArray(&NSArray::from_slice(&[unsafe {
          NSPasteboardTypeString
        }]))
        .is_some()
    } else {
      true
    }
  }

  pub fn editing(&self) -> bool {
    self.field.currentEditor().is_some()
  }

  /// Returns keyboard focus to the document view.
  pub fn focus_content(&self) {
    if !self.editing() {
      return;
    }
    if let (Some(window), Some(view)) = (self.window.load(), self.view.load()) {
      window.makeFirstResponder(Some(&view));
    }
  }

  /// Shows the page; disabling waits for an edit to end.
  pub fn page(&self, location: Option<(usize, usize)>, enabled: bool) {
    let (shown, count) = location
      .map_or_else(Default::default, |(page, pages)| {
        (page.to_string(), format!("of {pages}"))
      });
    let enabled = enabled && location.is_some() || self.editing();
    self.field.setEnabled(enabled);
    if let Some(item) = &self.page {
      item.setEnabled(enabled);
    }
    self.count.setStringValue(&NSString::from_str(&count));
    if !self.editing() {
      self.field.setStringValue(&NSString::from_str(&shown));
    }
    *self.shown.borrow_mut() = shown;
  }

  /// Restores the shown page after editing.
  pub fn revert(&self) {
    self.field.setStringValue(&NSString::from_str(&self.shown.borrow()));
  }

  pub fn attach(&self, handler: &Handler, window: &NSWindow, visible: bool) {
    self.toolbar.setDelegate(Some(ProtocolObject::from_ref(handler)));
    window.setToolbarStyle(objc2_app_kit::NSWindowToolbarStyle::UnifiedCompact);
    window.setToolbar(Some(&self.toolbar));
    // Attaching shows the toolbar.
    self.toolbar.setVisible(visible);
  }

  pub fn detach(&self) {
    if let Some(window) = self.window.load() {
      window.setToolbar(None);
    }
    self.toolbar.setDelegate(None);
    // SAFETY: Disconnect before releasing the retained delegate.
    unsafe { self.field.setDelegate(None) };
    for control in self.items.iter().flatten() {
      // SAFETY: Disconnect before releasing the retained target.
      unsafe { control.item.setTarget(None) };
    }
    for choice in &self.pulldowns {
      // SAFETY: Disconnect before releasing the retained target.
      unsafe { choice.setTarget(None) };
    }
  }
}

/// A positive page number typed into the field.
/// Accepts full-width digits from CJK input.
pub(super) fn requested_page(text: &str) -> Option<usize> {
  let ascii: String = text
    .trim()
    .chars()
    .map(|c| match c {
      '\u{FF10}'..='\u{FF19}' => char::from(b'0' + (c as u32 - 0xFF10) as u8),
      c => c,
    })
    .collect();
  ascii.parse().ok().filter(|&page| page > 0)
}

fn button(
  handler: &Handler,
  id: &NSString,
  entries: &[Item],
  entry: usize,
  [label, symbol]: [&str; 2],
  keep: bool,
) -> Retained<NSToolbarItem> {
  let item = NSToolbarItem::initWithItemIdentifier(
    NSToolbarItem::alloc(handler.mtm()),
    id,
  );
  describe(&item, label, symbol);
  let tip = entries.get(entry).map(|e| NSString::from_str(e.title));
  item.setToolTip(tip.as_deref());
  item.setBordered(true);
  item.setTag(entry as isize);
  if keep {
    item.setVisibilityPriority(NSToolbarItemVisibilityPriorityHigh);
  }
  // SAFETY: Handler implements this one-argument action.
  unsafe {
    item.setTarget(Some(handler));
    item.setAction(Some(sel!(toolbarAction:)));
  }
  item
}

/// A pull-down whose choices land in `choices`.
fn pulldown(
  handler: &Handler,
  id: &NSString,
  entries: &[Item],
  chosen: &[usize],
  [label, symbol]: [&str; 2],
  choices: &mut Vec<Retained<NSMenuItem>>,
) -> Retained<NSToolbarItem> {
  let mtm = handler.mtm();
  let menu = NSMenu::new(mtm);
  menu.setAutoenablesItems(false);
  for &entry in chosen {
    let Some(source) = entries.get(entry) else { continue };
    let title = NSString::from_str(source.title);
    let choice = menu_item(mtm, &title, sel!(openReader:), handler);
    choice.setTag(entry as isize);
    choice.setEnabled(false);
    menu.addItem(&choice);
    choices.push(choice);
  }
  let item = NSMenuToolbarItem::initWithItemIdentifier(
    NSMenuToolbarItem::alloc(mtm),
    id,
  );
  describe(&item, label, symbol);
  item.setToolTip(Some(&NSString::from_str(label)));
  item.setMenu(&menu);
  Retained::into_super(item)
}

fn page(
  id: &NSString,
  field: &NSTextField,
  count: &NSTextField,
) -> Retained<NSToolbarItem> {
  let mtm = field.mtm();
  let views: [&NSView; 2] = [field, count];
  let stack =
    NSStackView::stackViewWithViews(&NSArray::from_slice(&views), mtm);
  let item =
    NSToolbarItem::initWithItemIdentifier(NSToolbarItem::alloc(mtm), id);
  item.setLabel(ns_string!("Page"));
  item.setPaletteLabel(ns_string!("Page"));
  item.setView(Some(&stack));
  item.setVisibilityPriority(NSToolbarItemVisibilityPriorityUser);
  item
}

fn describe(item: &NSToolbarItem, label: &str, symbol: &str) {
  let label = NSString::from_str(label);
  item.setLabel(&label);
  item.setPaletteLabel(&label);
  let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
    &NSString::from_str(symbol),
    Some(&label),
  );
  item.setImage(image.as_deref());
}

#[cfg(test)]
mod tests {
  #[test]
  fn requested_pages_are_positive_numbers() {
    for (text, page) in [
      ("3", Some(3)),
      ("\u{FF11}\u{FF12}", Some(12)),
      (" 12 ", Some(12)),
      ("0", None),
      ("-1", None),
      ("x", None),
      ("", None),
    ] {
      assert_eq!(super::requested_page(text), page, "{text:?}");
    }
  }
}
