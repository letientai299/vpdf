use super::{KeyEvent, Result, Session};
use vpdf_terminal::decode::Mouse;

pub(super) enum Interaction {
  Key(KeyEvent),
  Paste(String),
  Mouse(Mouse),
  Leave,
}

impl Session {
  pub(super) fn interact(&mut self, input: Interaction) -> Result<bool> {
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
