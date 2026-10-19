//! Overlay items the pointer picks and runs.

use crate::Result;
use crate::app::{App, Request};
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use vpdf_ui::input::InputOwner;
use vpdf_ui::pointer::{Hit, Items, List};

impl App {
  /// The item list that owns input.
  pub(crate) fn owned_list(&self) -> Option<List> {
    match self.input_owner() {
      InputOwner::Context => Some(List::Menu),
      InputOwner::Region => Some(List::Actions),
      InputOwner::Hints => Some(List::Hints),
      InputOwner::Prompt => self
        .prompt
        .as_ref()
        .and_then(|prompt| prompt.menu())
        .map(|_| List::Completions),
      _ => self.clues.as_ref().map(|_| List::Clues),
    }
  }

  pub(crate) fn items(&self, list: List) -> Items {
    match list {
      List::Menu => {
        self.context_menu.as_ref().map(crate::context::ContextMenu::items)
      }
      List::Actions => Some(self.bar_items()),
      List::Hints => Some(self.hint_items()),
      List::Clues => Some(self.clue_cells()),
      List::Completions => Some(self.completion_cells()),
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
      // Picking would rewrite the line; clicks do.
      List::Clues | List::Completions => {}
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
      List::Clues => {
        let label = self.clues.as_ref().map(|clues| &clues.items[index].0);
        let keys = label.and_then(|label| crate::clues::clue_keys(label));
        // The session types them, as keys.
        self.request = keys.map(Request::Keys);
        Ok(())
      }
      List::Completions => {
        if let Some(prompt) = &mut self.prompt {
          prompt.pick(index);
        }
        Ok(())
      }
    }
  }

  /// The `list` item under the pointer.
  pub(crate) fn pointed_item(&self, list: List) -> Option<usize> {
    match self.hover {
      Hit::Item(hovered, index) if hovered == list => index,
      _ => None,
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
