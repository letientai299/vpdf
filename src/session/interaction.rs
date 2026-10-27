use super::{KeyEvent, Result, Session};
use vpdf_model::input::Mouse;

pub(super) enum Interaction {
  Key(KeyEvent),
  Paste(String),
  Mouse(Mouse),
  /// A trackpad turn or pinch at a wheel report.
  Gesture(Mouse, super::pointer::Gesture),
  Leave,
}

impl Session {
  pub(super) fn confirming_removal(&self) -> bool {
    self.tabs.get(self.active).is_some_and(|tab| {
      match &tab.panes[tab.focus].body {
        super::Body::Document(app) => app.confirming_removal(),
        super::Body::Recent(home) => home.view.confirming(),
      }
    })
  }

  pub(super) fn interact(&mut self, input: Interaction) -> Result<bool> {
    if self.confirming_removal() {
      if self.focused().is_recent() {
        match input {
          Interaction::Key(key) => {
            self.recent_event(vpdf_model::input::Event::Key(key))?;
          }
          Interaction::Mouse(mouse) => {
            self.recent_event(vpdf_model::input::Event::Mouse(mouse))?;
          }
          Interaction::Paste(_)
          | Interaction::Gesture(..)
          | Interaction::Leave => {}
        }
        return Ok(true);
      }
      match input {
        Interaction::Key(key) => self.app()?.confirm_removal(key),
        Interaction::Mouse(report) => {
          if let Some(mouse) = self.screen_mouse(report) {
            self.app()?.confirm_click(mouse);
          }
        }
        Interaction::Paste(_)
        | Interaction::Gesture(..)
        | Interaction::Leave => {}
      }
      self.settle()?;
      return Ok(true);
    }
    let changed = match input {
      Interaction::Key(key) => self.type_key(key).map(|()| true),
      Interaction::Paste(text) => {
        if self.focused().is_recent() {
          self
            .recent_event(vpdf_model::input::Event::Paste(text))
            .map(|()| true)
        } else {
          let tab = &mut self.tabs[self.active];
          tab.panes[tab.focus]
            .document_mut()
            .ok_or_else(|| anyhow::anyhow!("No document"))?
            .paste(&text, &mut self.history)
            .map(|()| true)
        }
      }
      Interaction::Mouse(mouse) => self.mouse_report(mouse),
      Interaction::Gesture(mouse, trackpad) => {
        self.gesture_report(mouse, trackpad)
      }
      // Native leaving skips pending requests.
      Interaction::Leave => return Ok(self.pointer_left()),
    }?;
    self.record_recent_layout();
    self.settle()?;
    Ok(changed)
  }
}
