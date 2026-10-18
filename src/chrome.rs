//! Clickable status items, clues, and completions.

use crate::Result;
use crate::app::{App, Request, TabMove};
use crate::keys::{Action, Input, PanelKind};
use crate::pointer::Region;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton};

/// A clickable status item, clue, or completion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Chrome {
  /// A listed tab.
  Tab(usize),
  /// The section name: opens the outline there.
  Section,
  /// The page counter: opens the overview.
  Pages,
  /// The zoom label: fits width.
  Zoom,
  /// A key clue that names keys.
  Clue(usize),
  Completion(usize),
}

impl App {
  /// The chrome item at a pane cell.
  pub fn chrome_at(
    &self,
    region: Region,
    column: u16,
    row: u16,
  ) -> Option<Chrome> {
    match region {
      Region::Status if row + 1 == self.screen.rows => self.status_item(column),
      Region::Overlay => self.overlay_item(column, row),
      _ => None,
    }
  }

  /// A completion or a clue naming keys.
  fn overlay_item(&self, column: u16, row: u16) -> Option<Chrome> {
    let status = self.screen.rows.checked_sub(1)?;
    let line = usize::from(row.checked_sub(status - self.menu_rows())?);
    let column = usize::from(column);
    if let Some(menu) = self.prompt.as_ref().and_then(|p| p.menu.as_ref()) {
      let limit = crate::grid::limit(self.screen);
      let grid = menu.layout(usize::from(self.screen.columns), limit);
      let index = grid.item_at(column, line, menu.items.len())?;
      return Some(Chrome::Completion(index));
    }
    let clues = self.clues.as_ref()?;
    let (labels, grid, _) = self.clue_grid(clues);
    let index = grid.item_at(column, line, labels.len())?;
    clue_keys(&clues.items[index].0).map(|_| Chrome::Clue(index))
  }

  /// Acts on a pressed status item.
  pub(crate) fn chrome_press(
    &mut self,
    item: Chrome,
    button: MouseButton,
  ) -> Result<()> {
    let action = match (button, item) {
      (MouseButton::Left, Chrome::Tab(index)) => {
        self.request = Some(Request::Tab(TabMove::Go(index + 1)));
        return Ok(());
      }
      (MouseButton::Middle, Chrome::Tab(index)) => {
        self.request = Some(Request::TabClose(Some(index)));
        return Ok(());
      }
      (MouseButton::Left, Chrome::Section) => return self.outline_here(),
      (MouseButton::Left, Chrome::Pages) => Action::Overview,
      (MouseButton::Left, Chrome::Zoom) => Action::Fit,
      _ => return Ok(()),
    };
    self.action(Input { action, count: 1, explicit: false, register: '"' })
  }

  /// The outline, focused on the current section.
  fn outline_here(&mut self) -> Result<()> {
    let action = Action::Panel(PanelKind::Outline);
    self.action(Input { action, count: 1, explicit: false, register: '"' })?;
    let section = self.current_section;
    if let Some(panel) = &mut self.panel
      && let Some(index) = section
    {
      panel.selected = index;
      panel.reveal();
    }
    Ok(())
  }
}

/// Keys a clue names; ranges name none.
pub(crate) fn clue_keys(label: &str) -> Option<Vec<KeyEvent>> {
  let range = label.len() == 3
    && label.as_bytes()[1] == b'-'
    && label.as_bytes()[0] != b'C';
  if range || label.contains(' ') {
    return None;
  }
  let mut keys = Vec::new();
  let mut rest = label;
  while let Some(first) = rest.chars().next() {
    let (code, modifiers, tail) = match rest.strip_prefix("C-") {
      Some(tail) if !tail.is_empty() => {
        let ch = tail.chars().next()?;
        (ch, KeyModifiers::CONTROL, &tail[ch.len_utf8()..])
      }
      _ => (first, KeyModifiers::NONE, &rest[first.len_utf8()..]),
    };
    keys.push(KeyEvent::new(KeyCode::Char(code), modifiers));
    rest = tail;
  }
  (!keys.is_empty()).then_some(keys)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn clue_labels_name_keys() {
    let key = |ch, modifiers| KeyEvent::new(KeyCode::Char(ch), modifiers);
    let (none, ctrl) = (KeyModifiers::NONE, KeyModifiers::CONTROL);
    // (label) -> keys
    let cases = [
      ("v", Some(vec![key('v', none)])),
      ("gg", Some(vec![key('g', none), key('g', none)])),
      ("C-s", Some(vec![key('s', ctrl)])),
      ("-", Some(vec![key('-', none)])),
      ("a-z", None),
      ("A-Z", None),
      ("+ *", None),
    ];
    for (label, want) in cases {
      assert_eq!(clue_keys(label), want, "{label}");
    }
  }
}
