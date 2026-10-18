//! Clickable status items.

use crate::Result;
use crate::app::{App, Request, TabMove};
use crate::keys::{Action, Input, PanelKind};
use crossterm::event::MouseButton;

/// A clickable status item.
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
}

impl App {
  /// The status item at a pane cell.
  pub fn chrome_at(&self, column: u16, row: u16) -> Option<Chrome> {
    let status = row + 1 == self.screen.rows;
    status.then(|| self.status_item(column)).flatten()
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
