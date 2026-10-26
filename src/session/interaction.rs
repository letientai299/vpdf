use super::{App, KeyEvent, Result, Session};
use vpdf_model::input::Mouse;

pub(super) enum Interaction {
  Key(KeyEvent),
  Paste(String),
  Mouse(Mouse),
  Leave,
}

impl Session {
  pub(super) fn confirming_recent(&self) -> bool {
    self.home.as_ref().is_some_and(|home| home.view.confirming())
      || self
        .tabs
        .get(self.active)
        .and_then(|tab| tab.panes.get(tab.focus))
        .is_some_and(App::confirming_recent)
  }

  pub(super) fn interact(&mut self, input: Interaction) -> Result<bool> {
    if !self.tabs.is_empty() && self.app().confirming_recent() {
      match input {
        Interaction::Key(key) => self.app().confirm_recent(key),
        Interaction::Mouse(report) => {
          if let Some(mouse) = self.screen_mouse(report) {
            self.app().confirm_click(mouse);
          }
        }
        Interaction::Paste(_) | Interaction::Leave => {}
      }
      self.settle()?;
      return Ok(true);
    }
    let changed = match input {
      Interaction::Key(key) => self.type_key(key).map(|()| true),
      Interaction::Paste(text) => {
        let tab = &mut self.tabs[self.active];
        tab.panes[tab.focus].paste(&text, &mut self.history).map(|()| true)
      }
      Interaction::Mouse(mouse) => self.mouse_report(mouse),
      // Native leaving skips pending requests.
      Interaction::Leave => return Ok(self.pointer_left()),
    }?;
    self.settle()?;
    Ok(changed)
  }
}
