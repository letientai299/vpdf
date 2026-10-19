//! Clickable status items.

use crate::Result;
use crate::app::{App, Request};
use crossterm::event::MouseButton;
use vpdf_ui::keys::{Action, Input, PanelKind};
use vpdf_ui::window::TabMove;

use vpdf_ui::chrome::{Item, Press};

impl App {
  /// The status item at a pane cell.
  pub fn chrome_at(&self, column: u16, row: u16) -> Option<Item> {
    self.chrome_view(&self.name()).hit(column, row, self.overlaid())
  }

  /// Acts on a pressed status item.
  pub(crate) fn chrome_press(
    &mut self,
    item: Item,
    button: MouseButton,
  ) -> Result<()> {
    let Some(press) = item.press(button) else { return Ok(()) };
    let action = match press {
      Press::Tab(index) => {
        self.request = Some(Request::Tab(TabMove::Go(index + 1)));
        return Ok(());
      }
      Press::CloseTab(index) => {
        self.request = Some(Request::TabClose(Some(index)));
        return Ok(());
      }
      Press::Section => return self.outline_here(),
      Press::Action(action) => action,
    };
    self.action(Input::plain(action))
  }

  /// The outline, focused on the current section.
  fn outline_here(&mut self) -> Result<()> {
    let action = Action::Panel(PanelKind::Outline);
    self.action(Input::plain(action))?;
    let section = self.outline.current();
    if let Some(panel) = &mut self.panel
      && let Some(index) = section
    {
      panel.view.select(index);
    }
    Ok(())
  }
}
