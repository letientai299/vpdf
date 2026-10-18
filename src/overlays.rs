//! Overlay items the pointer picks and runs.

use crate::Result;
use crate::app::{App, InputOwner};
use crate::pointer::{Hit, Items, List};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

impl App {
  /// The item list that owns input.
  pub(crate) fn owned_list(&self) -> Option<List> {
    match self.input_owner() {
      InputOwner::Context => Some(List::Menu),
      InputOwner::Region => Some(List::Actions),
      InputOwner::Hints => Some(List::Hints),
      _ => None,
    }
  }

  pub(crate) fn items(&self, list: List) -> Items {
    match list {
      List::Menu => {
        self.context_menu.as_ref().map(crate::context::ContextMenu::items)
      }
      List::Actions => Some(self.bar_items()),
      List::Hints => Some(self.hint_items()),
    }
    .unwrap_or_default()
  }

  /// Selects an item, as keys do.
  pub(crate) fn pick_item(&mut self, list: List, index: usize) {
    match list {
      List::Menu => {
        if let Some(menu) = &mut self.context_menu {
          menu.selected = index;
        }
      }
      List::Actions => self.pick_action(index),
      List::Hints => {
        if let Some(hints) = &mut self.hints {
          hints.selected = Some(index);
        }
      }
    }
  }

  /// Runs an item; its overlay may close.
  pub(crate) fn run_item(&mut self, list: List, index: usize) -> Result<()> {
    match list {
      List::Menu => {
        self.pick_item(list, index);
        self.context_run()
      }
      List::Actions => self.run_action(index),
      List::Hints => self.run_hint(index),
    }
  }

  /// Motion picks the owning list's item.
  pub(crate) fn moved_onto(&mut self, hit: Hit) {
    if let Hit::Item(list, Some(index)) = hit
      && self.owned_list() == Some(list)
    {
      self.pick_item(list, index);
    }
  }

  /// Clicks run the owning list's item.
  pub(crate) fn item_press(&mut self, mouse: MouseEvent) -> Result<bool> {
    let alt = mouse.modifiers.contains(KeyModifiers::ALT);
    let left = mouse.kind == MouseEventKind::Down(MouseButton::Left);
    // Alt-clicks close the bar, as before.
    let list =
      self.owned_list().filter(|list| !(alt && *list == List::Actions));
    let Some(list) = list.filter(|_| left) else { return Ok(false) };
    let (column, row) = self.pane_cell(mouse);
    let Some(index) = self.items(list).at(column, row) else {
      return Ok(false);
    };
    self.run_item(list, index).map(|()| true)
  }
}
